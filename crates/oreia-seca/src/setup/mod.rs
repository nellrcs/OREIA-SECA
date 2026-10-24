//! Setup inicial do OREIA-SECA, de três formas (documentadas em
//! `docs/instalacao.md`):
//!
//! - `oreia-seca setup` — wizard interativo no terminal.
//! - `oreia-seca setup --web [--host H] [--port P]` — formulário numa página local.
//! - `oreia-seca setup --answers ARQUIVO.toml` (ou flags soltas) — não interativo,
//!   para rodar a partir de um script ou de um agente de IA.
//!
//! As três formas convergem no mesmo [`Answers`] e na mesma [`apply_answers`]:
//! a UI só decide como *coletar* as respostas, nunca como aplicá-las.

pub mod env_file;
pub mod llm;
mod web;
mod wizard;

use std::path::Path;

use serde::Deserialize;

/// O que o setup grava/faz. Espelha `setup.answers.example.toml` na raiz do
/// repositório — mantenha os dois em sincronia.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Answers {
    pub telegram: TelegramAnswers,
    pub llm: LlmAnswers,
    pub forgejo: ForgejoAnswers,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TelegramAnswers {
    /// Vazio = Telegram fica desativado (sem mudar o que já está no `.env`).
    pub token: String,
    /// IDs separados por vírgula.
    pub allowed: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LlmAnswers {
    /// Vazio = não toca na configuração de LLM já existente.
    pub url: String,
    pub model: String,
    /// Vazio = usa o mesmo `model` para a conversa direta.
    pub chat_model: String,
    /// Vazio = sem autenticação.
    pub api_key: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ForgejoAnswers {
    /// Roda `scripts/bootstrap.sh` (sobe Postgres+Forgejo, cria contas, tokens,
    /// repositórios, labels e webhook). Precisa de Docker; pode demorar.
    pub bootstrap: bool,
}

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub llm_configured: bool,
    pub telegram_configured: bool,
    pub bootstrap_ran: bool,
    pub bootstrap_output: String,
    pub notes: Vec<String>,
}

/// Único caminho de aplicação das respostas, usado pelo wizard, pela página
/// web e pelo arquivo/flags — veja o comentário do módulo.
pub async fn apply_answers(
    answers: &Answers,
    env_path: &Path,
    env_example_path: &Path,
    bootstrap_script: &Path,
) -> anyhow::Result<ApplyReport> {
    let mut updates: Vec<(String, String)> = Vec::new();
    let mut report = ApplyReport::default();

    if !answers.telegram.token.is_empty() {
        updates.push(("TELEGRAM_TOKEN".into(), answers.telegram.token.clone()));
        report.telegram_configured = true;
    }
    if !answers.telegram.allowed.is_empty() {
        updates.push(("TELEGRAM_ALLOWED".into(), answers.telegram.allowed.clone()));
    }

    if !answers.llm.url.is_empty() || !answers.llm.model.is_empty() {
        anyhow::ensure!(
            !answers.llm.url.is_empty() && !answers.llm.model.is_empty(),
            "llm.url e llm.model precisam vir junto, um não faz sentido sem o outro"
        );
        let chat_model =
            if answers.llm.chat_model.is_empty() { answers.llm.model.clone() } else { answers.llm.chat_model.clone() };
        let api_key = (!answers.llm.api_key.is_empty()).then(|| answers.llm.api_key.clone());
        let collected = llm::Collected {
            url: answers.llm.url.clone(),
            coder_model: answers.llm.model.clone(),
            chat_model,
            api_key,
        };
        updates.extend(collected.env_updates());
        report.llm_configured = true;
    } else {
        report.notes.push("Nenhum LLM configurado agora — rode `make llm-setup` quando quiser.".into());
    }

    env_file::write_env_updates(env_path, env_example_path, &updates)?;

    if answers.forgejo.bootstrap {
        match run_bootstrap(bootstrap_script).await {
            Ok(out) => {
                report.bootstrap_ran = true;
                report.bootstrap_output = out;
            }
            Err(e) => report.notes.push(format!("bootstrap do Forgejo falhou: {e:#}")),
        }
    } else {
        report.notes.push("Forgejo não configurado agora — rode `make bootstrap` quando quiser.".into());
    }

    Ok(report)
}

async fn run_bootstrap(script: &Path) -> anyhow::Result<String> {
    let output = tokio::process::Command::new(script)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("não consegui iniciar {}: {e}", script.display()))?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    anyhow::ensure!(output.status.success(), "{} terminou com erro:\n{text}", script.display());
    Ok(text)
}

pub fn print_report(report: &ApplyReport) {
    println!();
    if report.llm_configured {
        println!("✅ LLM configurado no .env.");
    }
    if report.telegram_configured {
        println!("✅ Telegram configurado no .env.");
    }
    if report.bootstrap_ran {
        println!("✅ Bootstrap do Forgejo rodou:\n{}", report.bootstrap_output.trim());
    }
    for n in &report.notes {
        println!("ℹ {n}");
    }
}

fn load_answers(path: &Path) -> anyhow::Result<Answers> {
    let text =
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("não consegui ler {}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

#[derive(Debug, Default)]
struct Flags {
    web: bool,
    host: Option<String>,
    port: Option<u16>,
    answers_file: Option<String>,
    telegram_token: Option<String>,
    telegram_allowed: Option<String>,
    llm_url: Option<String>,
    llm_model: Option<String>,
    llm_chat_model: Option<String>,
    llm_api_key: Option<String>,
    forgejo_bootstrap: Option<bool>,
}

impl Flags {
    fn has_any_field(&self) -> bool {
        self.telegram_token.is_some()
            || self.telegram_allowed.is_some()
            || self.llm_url.is_some()
            || self.llm_model.is_some()
            || self.llm_chat_model.is_some()
            || self.llm_api_key.is_some()
            || self.forgejo_bootstrap.is_some()
    }

    fn merge_into(&self, a: &mut Answers) {
        if let Some(v) = &self.telegram_token {
            a.telegram.token = v.clone();
        }
        if let Some(v) = &self.telegram_allowed {
            a.telegram.allowed = v.clone();
        }
        if let Some(v) = &self.llm_url {
            a.llm.url = v.clone();
        }
        if let Some(v) = &self.llm_model {
            a.llm.model = v.clone();
        }
        if let Some(v) = &self.llm_chat_model {
            a.llm.chat_model = v.clone();
        }
        if let Some(v) = &self.llm_api_key {
            a.llm.api_key = v.clone();
        }
        if let Some(v) = self.forgejo_bootstrap {
            a.forgejo.bootstrap = v;
        }
    }
}

fn parse_flags(args: &[String]) -> anyhow::Result<Flags> {
    let mut f = Flags::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--web" => f.web = true,
            "--host" => f.host = Some(next_value(&mut it, "--host")?),
            "--port" => f.port = Some(next_value(&mut it, "--port")?.parse()?),
            "--answers" => f.answers_file = Some(next_value(&mut it, "--answers")?),
            "--telegram-token" => f.telegram_token = Some(next_value(&mut it, "--telegram-token")?),
            "--telegram-allowed" => f.telegram_allowed = Some(next_value(&mut it, "--telegram-allowed")?),
            "--llm-url" => f.llm_url = Some(next_value(&mut it, "--llm-url")?),
            "--llm-model" => f.llm_model = Some(next_value(&mut it, "--llm-model")?),
            "--llm-chat-model" => f.llm_chat_model = Some(next_value(&mut it, "--llm-chat-model")?),
            "--llm-api-key" => f.llm_api_key = Some(next_value(&mut it, "--llm-api-key")?),
            "--forgejo-bootstrap" => f.forgejo_bootstrap = Some(true),
            other => anyhow::bail!("argumento desconhecido: {other}"),
        }
    }
    Ok(f)
}

fn next_value(it: &mut std::slice::Iter<'_, String>, flag: &str) -> anyhow::Result<String> {
    it.next().cloned().ok_or_else(|| anyhow::anyhow!("{flag} precisa de um valor"))
}

pub async fn run(args: &[String]) -> anyhow::Result<()> {
    if args.first().map(String::as_str) == Some("llm") {
        return llm::run(&args[1..]).await;
    }

    let flags = parse_flags(args)?;
    if flags.web {
        let host = flags.host.clone().unwrap_or_else(|| "127.0.0.1".into());
        return web::run(&host, flags.port.unwrap_or(3121)).await;
    }

    let env_path = Path::new(".env");
    let env_example_path = Path::new(".env.example");
    let bootstrap_script = Path::new("scripts/bootstrap.sh");

    if flags.answers_file.is_none() && !flags.has_any_field() {
        return wizard::run(env_path, env_example_path, bootstrap_script).await;
    }

    let mut answers = match &flags.answers_file {
        Some(path) => load_answers(Path::new(path))?,
        None => Answers::default(),
    };
    flags.merge_into(&mut answers);

    let report = apply_answers(&answers, env_path, env_example_path, bootstrap_script).await?;
    print_report(&report);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("oreia-seca-setup-mod-{label}-{}", std::process::id()))
    }

    #[test]
    fn load_answers_parses_partial_toml() {
        let d = dir("load");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("answers.toml");
        std::fs::write(&path, "[llm]\nurl = \"http://x/v1\"\nmodel = \"m\"\n").unwrap();
        let a = load_answers(&path).unwrap();
        assert_eq!(a.llm.url, "http://x/v1");
        assert_eq!(a.llm.model, "m");
        assert_eq!(a.llm.chat_model, "");
        assert!(a.telegram.token.is_empty());
        assert!(!a.forgejo.bootstrap);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn load_answers_rejects_unknown_fields() {
        let d = dir("unknown");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("answers.toml");
        std::fs::write(&path, "[llm]\nurl = \"x\"\nmodel = \"m\"\ntypo = true\n").unwrap();
        assert!(load_answers(&path).is_err());
        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn apply_answers_without_forgejo_adds_a_note_and_does_not_run_anything() {
        let d = dir("noforgejo");
        std::fs::create_dir_all(&d).unwrap();
        let answers = Answers {
            llm: LlmAnswers { url: "http://x/v1".into(), model: "m".into(), ..Default::default() },
            ..Default::default()
        };

        let report = apply_answers(&answers, &d.join(".env"), &d.join(".env.example"), &d.join("missing-bootstrap.sh"))
            .await
            .unwrap();

        assert!(report.llm_configured);
        assert!(!report.telegram_configured);
        assert!(!report.bootstrap_ran);
        assert!(report.notes.iter().any(|n| n.contains("Forgejo")));

        let text = std::fs::read_to_string(d.join(".env")).unwrap();
        assert!(text.contains("LLM_CODER_MODEL=m"), "{text}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn apply_answers_rejects_llm_url_without_model() {
        let d = dir("halfllm");
        std::fs::create_dir_all(&d).unwrap();
        let answers =
            Answers { llm: LlmAnswers { url: "http://x/v1".into(), ..Default::default() }, ..Default::default() };
        let err =
            apply_answers(&answers, &d.join(".env"), &d.join(".env.example"), &d.join("missing.sh")).await.unwrap_err();
        assert!(err.to_string().contains("llm.url"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn apply_answers_reports_telegram_and_runs_bootstrap_script() {
        let d = dir("full");
        std::fs::create_dir_all(&d).unwrap();
        let script = d.join("bootstrap.sh");
        std::fs::write(&script, "#!/bin/sh\necho ok-from-bootstrap\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let answers = Answers {
            telegram: TelegramAnswers { token: "tok".into(), allowed: "1,2".into() },
            llm: LlmAnswers { url: "http://x/v1".into(), model: "m".into(), ..Default::default() },
            forgejo: ForgejoAnswers { bootstrap: true },
        };

        let report = apply_answers(&answers, &d.join(".env"), &d.join(".env.example"), &script).await.unwrap();
        assert!(report.telegram_configured);
        #[cfg(unix)]
        {
            assert!(report.bootstrap_ran, "{:?}", report.notes);
            assert!(report.bootstrap_output.contains("ok-from-bootstrap"));
        }

        std::fs::remove_dir_all(&d).ok();
    }
}
