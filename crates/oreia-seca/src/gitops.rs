//! Operações git feitas pelo orquestrador em nome de um agente.
//!
//! O token vai num cabeçalho HTTP passado por `git -c` a cada comando, então
//! nunca é gravado em `.git/config`. O shell do modelo roda com ambiente limpo
//! e não tem como ler o token nem fazer push sozinho.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use tokio::process::Command;

pub struct GitAuth {
    header: String,
}

impl GitAuth {
    pub fn new(user: &str, token: &str) -> Self {
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{token}"));
        Self { header: format!("http.extraHeader=Authorization: Basic {basic}") }
    }
}

async fn git(dir: &Path, args: &[&str], auth: Option<&GitAuth>) -> anyhow::Result<String> {
    let mut cmd = Command::new("git");
    if let Some(a) = auth {
        cmd.arg("-c").arg(&a.header);
    }
    cmd.args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()))
        .env("HOME", dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let out = tokio::time::timeout(Duration::from_secs(300), cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("git {} excedeu 300s", args.first().unwrap_or(&"")))??;

    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        // Nunca devolve o cabeçalho de autenticação em mensagens de erro.
        anyhow::bail!("git {} falhou: {}", args.first().unwrap_or(&""), text.trim())
    }
}

/// Clona o repositório em `dir` (que deve estar vazio) e cria/reseta o branch do agente.
pub async fn prepare(
    dir: &Path,
    clone_url: &str,
    base_branch: &str,
    branch: &str,
    auth: &GitAuth,
    author: &str,
) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    git(dir, &["clone", "--branch", base_branch, clone_url, "."], Some(auth)).await?;

    // Se o branch já existe no remoto (rodada anterior), continua dele.
    let remote_has_branch =
        git(dir, &["ls-remote", "--exit-code", "--heads", "origin", branch], Some(auth)).await.is_ok();
    if remote_has_branch {
        git(dir, &["fetch", "origin", &format!("{branch}:{branch}")], Some(auth)).await?;
        git(dir, &["checkout", branch], None).await?;
    } else {
        git(dir, &["checkout", "-b", branch], None).await?;
    }

    git(dir, &["config", "user.name", author], None).await?;
    git(dir, &["config", "user.email", &format!("{author}@agents.oreia-seca.local")], None).await?;
    Ok(())
}

/// Faz commit de tudo e push. Retorna `false` se não havia mudanças.
pub async fn commit_and_push(dir: &Path, branch: &str, message: &str, auth: &GitAuth) -> anyhow::Result<bool> {
    git(dir, &["add", "-A"], None).await?;
    let nothing = git(dir, &["diff", "--cached", "--quiet"], None).await.is_ok();
    if nothing {
        return Ok(false);
    }
    git(dir, &["commit", "-m", message], None).await?;
    git(dir, &["push", "-u", "origin", branch], Some(auth)).await?;
    Ok(true)
}

/// Lista resumida dos arquivos alterados em relação ao branch base.
pub async fn changed_files(dir: &Path, base_branch: &str) -> String {
    git(dir, &["diff", "--stat", &format!("origin/{base_branch}...HEAD")], None).await.unwrap_or_default()
}

/// SHA do HEAD atual (usado para marcar o início de uma fase, antes da escalada).
pub async fn current_sha(dir: &Path) -> anyhow::Result<String> {
    Ok(git(dir, &["rev-parse", "HEAD"], None).await?.trim().to_string())
}

/// Comita o estado atual como ponto de restauração local (sem push). Usado pela
/// escalada ao final de uma fase aprovada: se a próxima fase escalar e precisar
/// desfazer uma tentativa, `git checkout -- . && git clean -fd` volta até este ponto.
pub async fn checkpoint(dir: &Path, message: &str) -> anyhow::Result<()> {
    git(dir, &["add", "-A"], None).await?;
    if git(dir, &["diff", "--cached", "--quiet"], None).await.is_ok() {
        return Ok(()); // nada para commitar
    }
    git(dir, &["commit", "-m", message], None).await?;
    Ok(())
}

/// Descarta uma tentativa de fase que falhou a verificação, voltando ao último
/// checkpoint (commitado ou clonado), sem afetar fases já aprovadas.
pub async fn discard_attempt(dir: &Path) -> anyhow::Result<()> {
    git(dir, &["checkout", "--", "."], None).await?;
    git(dir, &["clean", "-fd"], None).await?;
    Ok(())
}

/// Junta todos os commits de checkpoint feitos desde `start_sha` num só (soft
/// reset), para que `commit_and_push` abra o PR com um único commit por fase
/// de trabalho, não um por degrau de escalada.
pub async fn squash_since(dir: &Path, start_sha: &str) -> anyhow::Result<()> {
    let head = current_sha(dir).await?;
    if head == start_sha.trim() {
        return Ok(());
    }
    git(dir, &["reset", "--soft", start_sha], None).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_header_is_basic() {
        let a = GitAuth::new("agent-back", "tok");
        assert!(a.header.starts_with("http.extraHeader=Authorization: Basic "));
        assert!(!a.header.contains("tok"));
    }

    /// Fluxo completo contra um repositório "remoto" local (bare).
    #[tokio::test]
    async fn prepare_commit_push_local_remote() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let remote = root.join("remote.git");
        let seed = root.join("seed");
        std::fs::create_dir_all(&seed).unwrap();

        let run = |dir: &Path, args: &[&str]| {
            let ok = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
            assert!(ok.status.success(), "{:?}: {}", args, String::from_utf8_lossy(&ok.stderr));
        };
        run(&root, &["init", "--bare", "-b", "main", remote.to_str().unwrap()]);
        run(&seed, &["init", "-b", "main"]);
        std::fs::write(seed.join("README.md"), "oi").unwrap();
        run(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
        run(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "init"]);
        run(&seed, &["push", remote.to_str().unwrap(), "main"]);

        let auth = GitAuth::new("u", "t");
        let work = root.join("work");
        let url = format!("file://{}", remote.display());
        prepare(&work, &url, "main", "agent/dev-back/issue-1", &auth, "agent-back").await.unwrap();

        assert!(!commit_and_push(&work, "agent/dev-back/issue-1", "nada", &auth).await.unwrap());
        std::fs::write(work.join("novo.rs"), "fn main() {}").unwrap();
        assert!(commit_and_push(&work, "agent/dev-back/issue-1", "feat: novo", &auth).await.unwrap());
        assert!(changed_files(&work, "main").await.contains("novo.rs"));

        // Segunda rodada continua do branch remoto existente
        let work2 = root.join("work2");
        prepare(&work2, &url, "main", "agent/dev-back/issue-1", &auth, "agent-back").await.unwrap();
        assert!(work2.join("novo.rs").exists());

        let _ = std::fs::remove_dir_all(&root);
    }
}
