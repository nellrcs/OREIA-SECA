//! Ações embutidas. Todas ficam presas ao `workdir` da tarefa.
//!
//! O shell roda com o ambiente limpo (sem tokens do orquestrador), para que
//! um comando gerado pelo modelo não consiga ler segredos de variáveis de ambiente.

use std::process::Stdio;

use async_trait::async_trait;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::{confine, Action, ActionContext, ParamSpec, Params};

const MAX_OUTPUT: usize = 8_000;
const MAX_READ: usize = 64_000;

fn truncate(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}\n…(saída truncada)")
}

/// Comando de shell sem herdar variáveis de ambiente do processo pai.
pub fn sandboxed_shell(command: &str, ctx: &ActionContext) -> Command {
    // `ctx.workdir` costuma ser relativo (ex. "workspace/task-x", relativo ao cwd do
    // orquestrador — ver `exec.rs`). `current_dir` resolve isso certo (o kernel resolve
    // contra o cwd do processo pai), mas `HOME` é só uma STRING: um processo filho que
    // monta caminhos a partir dela (ex. Node.js `path.resolve($HOME, ...)`) resolve de
    // novo contra o PRÓPRIO cwd (= o mesmo workdir, já absoluto) — duplicando o caminho
    // ("workdir/workdir/..."). Bug real visto em produção: o opencode gravou cache/dados
    // em exatamente esse caminho duplicado, dentro do próprio repositório, e tudo foi
    // comitado pro PR. Resolve pra absoluto aqui pra `HOME` nunca ser ambíguo.
    let home = std::fs::canonicalize(&ctx.workdir).unwrap_or_else(|_| ctx.workdir.clone());
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(&home)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()))
        .env("HOME", &home)
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(id) = &ctx.task_id {
        cmd.env("OREIA_SECA_TASK_ID", id);
    }
    cmd
}

/// Roda o comando com timeout e devolve `$ cmd` + saída + código.
pub async fn run_command(mut cmd: Command, display: &str, ctx: &ActionContext) -> anyhow::Result<String> {
    let mut child = cmd.spawn()?;
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");

    let work = async {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (r1, r2) = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err));
        r1?;
        r2?;
        let status = child.wait().await?;
        anyhow::Ok((status, out, err))
    };

    let (status, out, err) = match tokio::time::timeout(ctx.shell_timeout, work).await {
        Ok(r) => r?,
        Err(_) => anyhow::bail!("comando excedeu {}s: {display}", ctx.shell_timeout.as_secs()),
    };

    let mut text = format!("$ {display}\n");
    text.push_str(&String::from_utf8_lossy(&out));
    if !err.is_empty() {
        text.push_str(&String::from_utf8_lossy(&err));
    }
    let text = truncate(text, MAX_OUTPUT);

    if status.success() {
        Ok(text)
    } else {
        anyhow::bail!("{text}\n(código de saída {})", status.code().unwrap_or(-1))
    }
}

pub struct ShellAction;

#[async_trait]
impl Action for ShellAction {
    fn name(&self) -> &str {
        "shell"
    }
    fn description(&self) -> &str {
        "Executa um comando de shell no diretório da tarefa e devolve a saída."
    }
    fn params(&self) -> Vec<ParamSpec> {
        vec![ParamSpec::new("command", "comando a executar", true)]
    }
    async fn run(&self, p: &Params, ctx: &ActionContext) -> anyhow::Result<String> {
        let command = &p["command"];
        run_command(sandboxed_shell(command, ctx), command, ctx).await
    }
}

pub struct FileWriteAction;

#[async_trait]
impl Action for FileWriteAction {
    fn name(&self) -> &str {
        "file_write"
    }
    fn description(&self) -> &str {
        "Cria ou sobrescreve um arquivo (caminho relativo ao diretório da tarefa)."
    }
    fn params(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec::new("path", "caminho relativo do arquivo", true),
            ParamSpec::new("content", "conteúdo completo do arquivo", true),
        ]
    }
    async fn run(&self, p: &Params, ctx: &ActionContext) -> anyhow::Result<String> {
        let path = confine(&ctx.workdir, &p["path"])?;
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&path, p["content"].as_bytes()).await?;
        Ok(format!("arquivo escrito: {}", p["path"].trim()))
    }
}

pub struct FileReadAction;

#[async_trait]
impl Action for FileReadAction {
    fn name(&self) -> &str {
        "file_read"
    }
    fn description(&self) -> &str {
        "Lê um arquivo (caminho relativo ao diretório da tarefa)."
    }
    fn params(&self) -> Vec<ParamSpec> {
        vec![ParamSpec::new("path", "caminho relativo do arquivo", true)]
    }
    async fn run(&self, p: &Params, ctx: &ActionContext) -> anyhow::Result<String> {
        let path = confine(&ctx.workdir, &p["path"])?;
        let text = tokio::fs::read_to_string(&path).await?;
        Ok(truncate(text, MAX_READ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ctx(dir: &std::path::Path) -> ActionContext {
        ActionContext {
            task_id: Some("task-t".into()),
            user_id: "u".into(),
            workdir: dir.to_path_buf(),
            shell_timeout: Duration::from_secs(5),
        }
    }

    #[tokio::test]
    async fn shell_has_clean_env() {
        std::env::set_var("OREIA_SECRET_TEST", "segredo");
        let dir = std::env::temp_dir();
        let mut p = Params::new();
        p.insert("command".into(), "echo \"[$OREIA_SECRET_TEST][$OREIA_SECA_TASK_ID]\"".into());
        let out = ShellAction.run(&p, &ctx(&dir)).await.unwrap();
        assert!(out.contains("[][task-t]"), "{out}");
    }

    #[tokio::test]
    async fn shell_reports_failure_and_timeout() {
        let dir = std::env::temp_dir();
        let mut p = Params::new();
        p.insert("command".into(), "echo falhou >&2; exit 3".into());
        let err = ShellAction.run(&p, &ctx(&dir)).await.unwrap_err().to_string();
        assert!(err.contains("falhou") && err.contains("código de saída 3"), "{err}");

        let mut c = ctx(&dir);
        c.shell_timeout = Duration::from_millis(100);
        p.insert("command".into(), "sleep 5".into());
        let err = ShellAction.run(&p, &c).await.unwrap_err().to_string();
        assert!(err.contains("excedeu"), "{err}");
    }

    /// Regressão real: com `ctx.workdir` relativo (como em produção — ver `exec.rs`),
    /// `HOME` saía como essa mesma string relativa. Um processo filho que resolve
    /// `$HOME` contra o PRÓPRIO cwd (já absoluto) duplicava o caminho
    /// ("workdir/workdir/...") — foi assim que o opencode acabou gravando cache/dados
    /// dentro do próprio repositório, comitados pro PR. `HOME` tem que sair absoluto.
    #[tokio::test]
    async fn shell_home_is_absolute_even_with_a_relative_workdir() {
        let relative = std::path::PathBuf::from(format!("oreia-core-shell-home-{}", std::process::id()));
        tokio::fs::create_dir_all(&relative).await.unwrap();

        let mut p = Params::new();
        p.insert("command".into(), "echo \"$HOME\"".into());
        let out = ShellAction.run(&p, &ctx(&relative)).await.unwrap();
        let home = out.lines().last().unwrap_or("").trim();
        assert!(std::path::Path::new(home).is_absolute(), "HOME não ficou absoluto: {out}");
        let doubled = relative.join(&relative);
        assert!(!home.ends_with(doubled.to_string_lossy().as_ref()), "HOME duplicou o caminho: {out}");

        tokio::fs::remove_dir_all(&relative).await.ok();
    }

    #[tokio::test]
    async fn file_write_refuses_escape() {
        let dir = std::env::temp_dir();
        let mut p = Params::new();
        p.insert("path".into(), "../fora.txt".into());
        p.insert("content".into(), "x".into());
        assert!(FileWriteAction.run(&p, &ctx(&dir)).await.is_err());
    }
}
