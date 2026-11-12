//! Setup inicial (e configuração contínua) do OREIA-SECA, de duas formas
//! (documentadas em `docs/instalacao.md`):
//!
//! - `oreia-seca setup` — TUI interativa no terminal (`tui`): assistente de
//!   primeira instalação (quando ainda não há nada configurado, ou com
//!   `--wizard`), painel (Status/Saúde/Uso) e as áreas de configuração.
//! - `oreia-seca setup --answers ARQUIVO.toml` (ou flags soltas) — não interativo,
//!   para rodar a partir de um script ou de um agente de IA.
//!
//! As duas formas convergem no mesmo [`Answers`] e na mesma [`apply_answers`]
//! para o `.env` (mais [`toml_edits::apply`] para `[telegram].enabled`/bloco
//! `[mcp]`/Forgejo/Servidor/modelos no `oreia-seca.toml`): a UI só decide
//! como *coletar* as respostas, nunca como aplicá-las.

pub mod current_state;
pub mod env_file;
pub mod llm;
pub mod toml_edits;
pub mod tui;

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// O que o setup grava/faz. Espelha `setup.answers.example.toml` na raiz do
/// repositório — mantenha os dois em sincronia.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Answers {
    pub telegram: TelegramAnswers,
    pub llm: LlmAnswers,
    pub forgejo: ForgejoAnswers,
    pub mcp: McpAnswers,
    pub px0: Px0Answers,
    pub ports: PortsAnswers,
}

/// Portas publicadas no host (`.env`). `None` = não muda. Só afetam o que você
/// acessa DE FORA — a rede interna do Docker continua nas portas padrão.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PortsAnswers {
    pub forgejo_http: Option<u16>,
    pub forgejo_ssh: Option<u16>,
    pub api: Option<u16>,
    pub px0: Option<u16>,
}

impl PortsAnswers {
    fn env_updates(&self) -> Vec<(String, String)> {
        [
            ("FORGEJO_HTTP_PORT", self.forgejo_http),
            ("FORGEJO_SSH_PORT", self.forgejo_ssh),
            ("OREIA_SECA_PORT", self.api),
            ("PX0_PORT", self.px0),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|p| (k.to_string(), p.to_string())))
        .collect()
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct McpAnswers {
    /// `None` = não muda o bloco `[mcp]` do `oreia-seca.toml`.
    pub enabled: Option<bool>,
    /// Transporte HTTP opcional (além do stdio). `None` = não muda. Ligar
    /// gera `MCP_TOKEN` no `.env` automaticamente se ainda não existir.
    pub http: Option<bool>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Px0Answers {
    /// Contas do Forgejo autorizadas a entrar no px0, separadas por vírgula.
    /// Vazio = não muda o que já está no `.env`.
    pub allowed_users: String,
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
    /// Seu usuário (dono/revisor) no Forgejo — `FORGEJO_OWNER_USER`. Vazio =
    /// não muda. Só pode ser trocado ANTES do primeiro bootstrap (depois, o
    /// token já pertence à conta antiga).
    pub owner_user: String,
    /// Repositórios `dono/repo`, separados por vírgula — `[forgejo].repos`.
    /// Vazio = não muda.
    pub repos: String,
}

/// Nomes que o Forgejo reserva para rotas próprias — criar um usuário ou uma
/// organização com um deles falha no bootstrap com um erro pouco óbvio.
const FORGEJO_RESERVED_NAMES: &[&str] =
    &["admin", "api", "assets", "avatars", "explore", "issues", "login", "org", "pulls", "repo", "user", "new"];

fn valid_forgejo_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().last().is_some_and(|c| c.is_ascii_alphanumeric())
}

/// Valida um nome de usuário do Forgejo; `Err` já vem pronto para mostrar na tela.
pub(crate) fn validate_owner_user(name: &str) -> Result<(), String> {
    if !valid_forgejo_name(name) {
        return Err(format!("\"{name}\" não serve como usuário: use só letras, números, - _ . (sem espaços)"));
    }
    if FORGEJO_RESERVED_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(format!("\"{name}\" é um nome reservado pelo Forgejo — escolha outro (ex.: dono)"));
    }
    Ok(())
}

/// Valida um repositório no formato `dono/repo`.
pub(crate) fn validate_repo(full: &str) -> Result<(), String> {
    let Some((org, repo)) = full.split_once('/') else {
        return Err(format!("\"{full}\" precisa estar no formato dono/repo (ex.: minha-org/meu-sistema)"));
    };
    if !valid_forgejo_name(org) || !valid_forgejo_name(repo) {
        return Err(format!("\"{full}\": use só letras, números, - _ . em cada parte de dono/repo"));
    }
    if FORGEJO_RESERVED_NAMES.contains(&org.to_ascii_lowercase().as_str()) {
        return Err(format!("\"{org}\" é um nome reservado pelo Forgejo — escolha outro dono/organização"));
    }
    Ok(())
}

/// `"a/b, c/d"` → `["a/b", "c/d"]`, validando cada um.
pub(crate) fn parse_repos(text: &str) -> Result<Vec<String>, String> {
    let repos: Vec<String> = text.split(',').map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).collect();
    for r in &repos {
        validate_repo(r)?;
    }
    Ok(repos)
}

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub llm_configured: bool,
    pub telegram_configured: bool,
    pub mcp_configured: bool,
    pub px0_configured: bool,
    pub forgejo_configured: bool,
    pub ports_configured: bool,
    pub bootstrap_ran: bool,
    pub bootstrap_output: String,
    pub notes: Vec<String>,
}

/// Único caminho de aplicação das respostas, usado pela TUI e pelo
/// arquivo/flags — veja o comentário do módulo.
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

    if !answers.px0.allowed_users.is_empty() {
        updates.push(("PX0_ALLOWED_USERS".into(), answers.px0.allowed_users.clone()));
        report.px0_configured = true;
    }

    let owner = answers.forgejo.owner_user.trim();
    if !owner.is_empty() {
        validate_owner_user(owner).map_err(|e| anyhow::anyhow!(e))?;
        let current_raw = env_file::read_env_value(env_path, "FORGEJO_OWNER_USER");
        let current = current_raw.clone().unwrap_or_else(|| "dono".into());
        let token_set = env_file::read_env_value(env_path, "FORGEJO_OWNER_TOKEN").is_some();
        anyhow::ensure!(
            !token_set || current == owner,
            "o bootstrap já criou a conta \"{current}\" (FORGEJO_OWNER_TOKEN existe) — trocar o dono para \"{owner}\" \
             exigiria recomeçar do zero (veja docs/instalacao.md#recomeçar-do-zero)"
        );
        if current_raw.as_deref() != Some(owner) {
            updates.push(("FORGEJO_OWNER_USER".into(), owner.to_string()));
            // O e-mail padrão acompanha o usuário; um e-mail customizado fica como está.
            let email = env_file::read_env_value(env_path, "FORGEJO_OWNER_EMAIL");
            let default_email = format!("{current}@oreia-seca.local");
            if email.is_none() || email.as_deref() == Some(default_email.as_str()) {
                updates.push(("FORGEJO_OWNER_EMAIL".into(), format!("{owner}@oreia-seca.local")));
            }
            report.forgejo_configured = true;
        }
    }
    if !answers.forgejo.repos.trim().is_empty() {
        let repos = parse_repos(&answers.forgejo.repos).map_err(|e| anyhow::anyhow!(e))?;
        // Mesmo valor que o bootstrap sincroniza a partir de [forgejo].repos — gravar
        // já aqui deixa o `docker compose` carregar mesmo antes do bootstrap rodar.
        updates.push(("FORGEJO_REPOS".into(), repos.join(", ")));
        report.forgejo_configured = true;
    }

    let port_updates = answers.ports.env_updates();
    if !port_updates.is_empty() {
        updates.extend(port_updates);
        report.ports_configured = true;
    }

    if answers.mcp.http == Some(true) {
        let token = env_file::read_env_value(env_path, "MCP_TOKEN")
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        updates.push(("MCP_TOKEN".into(), token));
    }
    if answers.mcp.enabled.is_some() || answers.mcp.http.is_some() {
        report.mcp_configured = true;
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
    if report.mcp_configured {
        println!("✅ MCP configurado (oreia-seca.toml e/ou .env).");
    }
    if report.px0_configured {
        println!("✅ px0 configurado no .env (PX0_ALLOWED_USERS).");
    }
    if report.forgejo_configured {
        println!("✅ Forgejo: dono/repositórios gravados (.env e oreia-seca.toml).");
    }
    if report.ports_configured {
        println!("✅ Portas ajustadas no .env.");
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
    answers_file: Option<String>,
    telegram_token: Option<String>,
    telegram_allowed: Option<String>,
    llm_url: Option<String>,
    llm_model: Option<String>,
    llm_chat_model: Option<String>,
    llm_api_key: Option<String>,
    forgejo_bootstrap: Option<bool>,
    forgejo_owner: Option<String>,
    forgejo_repos: Option<String>,
    px0_allowed: Option<String>,
    /// Força o assistente de primeira instalação na TUI (não conta como "campo").
    wizard: bool,
    mcp_enabled: Option<bool>,
    mcp_http: Option<bool>,
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
            || self.forgejo_owner.is_some()
            || self.forgejo_repos.is_some()
            || self.px0_allowed.is_some()
            || self.mcp_enabled.is_some()
            || self.mcp_http.is_some()
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
        if let Some(v) = &self.forgejo_owner {
            a.forgejo.owner_user = v.clone();
        }
        if let Some(v) = &self.forgejo_repos {
            a.forgejo.repos = v.clone();
        }
        if let Some(v) = &self.px0_allowed {
            a.px0.allowed_users = v.clone();
        }
        if let Some(v) = self.mcp_enabled {
            a.mcp.enabled = Some(v);
        }
        if let Some(v) = self.mcp_http {
            a.mcp.http = Some(v);
        }
    }
}

fn parse_flags(args: &[String]) -> anyhow::Result<Flags> {
    let mut f = Flags::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--answers" => f.answers_file = Some(next_value(&mut it, "--answers")?),
            "--telegram-token" => f.telegram_token = Some(next_value(&mut it, "--telegram-token")?),
            "--telegram-allowed" => f.telegram_allowed = Some(next_value(&mut it, "--telegram-allowed")?),
            "--llm-url" => f.llm_url = Some(next_value(&mut it, "--llm-url")?),
            "--llm-model" => f.llm_model = Some(next_value(&mut it, "--llm-model")?),
            "--llm-chat-model" => f.llm_chat_model = Some(next_value(&mut it, "--llm-chat-model")?),
            "--llm-api-key" => f.llm_api_key = Some(next_value(&mut it, "--llm-api-key")?),
            "--forgejo-bootstrap" => f.forgejo_bootstrap = Some(true),
            "--forgejo-owner" => f.forgejo_owner = Some(next_value(&mut it, "--forgejo-owner")?),
            "--forgejo-repos" => f.forgejo_repos = Some(next_value(&mut it, "--forgejo-repos")?),
            "--wizard" => f.wizard = true,
            "--px0-allowed" => f.px0_allowed = Some(next_value(&mut it, "--px0-allowed")?),
            "--mcp-enabled" => f.mcp_enabled = Some(true),
            "--mcp-http" => f.mcp_http = Some(true),
            other => anyhow::bail!("argumento desconhecido: {other}"),
        }
    }
    Ok(f)
}

fn next_value(it: &mut std::slice::Iter<'_, String>, flag: &str) -> anyhow::Result<String> {
    it.next().cloned().ok_or_else(|| anyhow::anyhow!("{flag} precisa de um valor"))
}

/// Caminho do `oreia-seca.toml`, respeitando `OREIA_SECA_CONFIG` — mesma
/// resolução que `setup::llm::apply_to_disk` já usa para a checagem final.
fn toml_path() -> PathBuf {
    crate::config::env("OREIA_SECA_CONFIG").map(PathBuf::from).unwrap_or_else(|| "oreia-seca.toml".into())
}

/// O que os modos não interativos (`--answers`/flags) mexem no `.toml`: o
/// bloco `[mcp]`, `[forgejo].repos` e — se a porta HTTP do Forgejo mudou —
/// `[forgejo].public_url` (o link que você abre no navegador). O toggle de
/// `[telegram].enabled` e os campos avançados (Servidor, modelos) são só da TUI.
fn toml_edits_from_answers(answers: &Answers) -> anyhow::Result<toml_edits::TomlEdits> {
    let repos = parse_repos(&answers.forgejo.repos).map_err(|e| anyhow::anyhow!(e))?;
    Ok(toml_edits::TomlEdits {
        mcp_enabled: answers.mcp.enabled,
        mcp_http: answers.mcp.http,
        forgejo_repos: (!repos.is_empty()).then_some(repos),
        forgejo_public_url: answers.ports.forgejo_http.map(|p| format!("http://localhost:{p}")),
        ..Default::default()
    })
}

/// Devolve a você a posse dos arquivos que o setup acabou de gravar.
///
/// Sem `cargo` no host, o launcher `./oreia` roda este binário num container
/// como root (única forma portátil de escrever na pasta montada e ler o
/// `docker.sock`). No Linux isso deixaria `.env`/`oreia-seca.toml` com dono
/// root — e o `scripts/bootstrap.sh`, que roda como você logo depois, não
/// conseguiria editá-los. O launcher passa o seu uid:gid em `OREIA_CHOWN`.
fn fix_ownership(paths: &[&Path]) {
    #[cfg(unix)]
    {
        let ids = crate::config::env("OREIA_CHOWN").and_then(|v| {
            let (uid, gid) = v.split_once(':')?;
            Some((uid.trim().parse::<u32>().ok()?, gid.trim().parse::<u32>().ok()?))
        });
        let Some((uid, gid)) = ids else { return };
        for path in paths {
            for candidate in [path.to_path_buf(), env_file::backup_path(path)] {
                if candidate.exists() {
                    if let Err(e) = std::os::unix::fs::chown(&candidate, Some(uid), Some(gid)) {
                        tracing::warn!(path = %candidate.display(), error = %e, "não consegui devolver a posse do arquivo");
                    }
                }
            }
        }
    }
    #[cfg(not(unix))]
    let _ = paths;
}

pub async fn run(args: &[String]) -> anyhow::Result<()> {
    let result = run_inner(args).await;
    let toml = toml_path();
    let action_file = crate::config::env("OREIA_ACTION_FILE").map(PathBuf::from);
    let mut written: Vec<&Path> = vec![Path::new(".env"), &toml];
    written.extend(action_file.as_deref());
    fix_ownership(&written);
    result
}

async fn run_inner(args: &[String]) -> anyhow::Result<()> {
    if args.first().map(String::as_str) == Some("llm") {
        return llm::run(&args[1..]).await;
    }

    let flags = parse_flags(args)?;

    let env_path = Path::new(".env");
    let env_example_path = Path::new(".env.example");
    let bootstrap_script = Path::new("scripts/bootstrap.sh");
    let toml_path = toml_path();
    let toml_example_path = Path::new("oreia-seca.example.toml");

    if flags.answers_file.is_none() && !flags.has_any_field() {
        return tui::run(env_path, env_example_path, &toml_path, toml_example_path, bootstrap_script, flags.wizard)
            .await;
    }

    let mut answers = match &flags.answers_file {
        Some(path) => load_answers(Path::new(path))?,
        None => Answers::default(),
    };
    flags.merge_into(&mut answers);

    toml_edits::apply(&toml_path, toml_example_path, &toml_edits_from_answers(&answers)?)?;
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
            forgejo: ForgejoAnswers { bootstrap: true, ..Default::default() },
            ..Default::default()
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

    #[tokio::test]
    async fn apply_answers_writes_px0_allowed_users() {
        let d = dir("px0");
        std::fs::create_dir_all(&d).unwrap();
        let answers = Answers { px0: Px0Answers { allowed_users: "dono,outro".into() }, ..Default::default() };

        let report =
            apply_answers(&answers, &d.join(".env"), &d.join(".env.example"), &d.join("missing.sh")).await.unwrap();
        assert!(report.px0_configured);
        let text = std::fs::read_to_string(d.join(".env")).unwrap();
        assert!(text.contains("PX0_ALLOWED_USERS=dono,outro"), "{text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn apply_answers_generates_mcp_token_only_when_missing() {
        let d = dir("mcp-generate");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        std::fs::write(&env_path, "MCP_TOKEN=already-there\n").unwrap();

        let answers = Answers { mcp: McpAnswers { enabled: Some(true), http: Some(true) }, ..Default::default() };
        let report = apply_answers(&answers, &env_path, &d.join(".env.example"), &d.join("missing.sh")).await.unwrap();
        assert!(report.mcp_configured);

        let text = std::fs::read_to_string(&env_path).unwrap();
        assert!(text.contains("MCP_TOKEN=already-there"), "não devia gerar um token novo se já existia um: {text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn apply_answers_generates_a_fresh_mcp_token_when_absent() {
        let d = dir("mcp-fresh");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");

        let answers = Answers { mcp: McpAnswers { enabled: Some(true), http: Some(true) }, ..Default::default() };
        apply_answers(&answers, &env_path, &d.join(".env.example"), &d.join("missing.sh")).await.unwrap();

        let token = env_file::read_env_value(&env_path, "MCP_TOKEN");
        assert!(token.is_some_and(|t| !t.is_empty()));

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn toml_edits_from_answers_ignores_telegram() {
        let answers = Answers { mcp: McpAnswers { enabled: Some(true), http: None }, ..Default::default() };
        let edits = toml_edits_from_answers(&answers).unwrap();
        assert_eq!(edits.telegram_enabled, None);
        assert_eq!(edits.mcp_enabled, Some(true));
        assert_eq!(edits.mcp_http, None);
        assert_eq!(edits.forgejo_repos, None);
        assert_eq!(edits.forgejo_public_url, None);
    }

    #[test]
    fn toml_edits_from_answers_carries_repos_and_public_url_of_a_changed_port() {
        let answers = Answers {
            forgejo: ForgejoAnswers { repos: "acme/app, acme/site".into(), ..Default::default() },
            ports: PortsAnswers { forgejo_http: Some(3001), ..Default::default() },
            ..Default::default()
        };
        let edits = toml_edits_from_answers(&answers).unwrap();
        assert_eq!(edits.forgejo_repos, Some(vec!["acme/app".to_string(), "acme/site".to_string()]));
        assert_eq!(edits.forgejo_public_url.as_deref(), Some("http://localhost:3001"));
    }

    #[test]
    fn validates_owner_and_repo_names() {
        assert!(validate_owner_user("dono").is_ok());
        assert!(validate_owner_user("ana.silva-2").is_ok());
        assert!(validate_owner_user("admin").unwrap_err().contains("reservado"));
        assert!(validate_owner_user("com espaço").is_err());
        assert!(validate_owner_user("").is_err());
        assert!(validate_repo("minha-org/meu-sistema").is_ok());
        assert!(validate_repo("sem-barra").unwrap_err().contains("dono/repo"));
        assert!(validate_repo("org/").is_err());
        assert!(validate_repo("user/app").unwrap_err().contains("reservado"));
        assert_eq!(parse_repos(" a/b ,, c/d ").unwrap(), vec!["a/b".to_string(), "c/d".to_string()]);
        assert!(parse_repos("a/b, ruim").is_err());
    }

    #[tokio::test]
    async fn apply_answers_writes_owner_repos_and_ports() {
        let d = dir("owner");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        std::fs::write(&env_path, "FORGEJO_OWNER_USER=dono\nFORGEJO_OWNER_EMAIL=dono@oreia-seca.local\n").unwrap();
        let answers = Answers {
            forgejo: ForgejoAnswers { owner_user: "ana".into(), repos: "acme/app".into(), ..Default::default() },
            ports: PortsAnswers { forgejo_http: Some(3001), px0: Some(7778), ..Default::default() },
            ..Default::default()
        };
        let report = apply_answers(&answers, &env_path, &d.join(".env.example"), &d.join("missing.sh")).await.unwrap();
        assert!(report.forgejo_configured && report.ports_configured);

        let text = std::fs::read_to_string(&env_path).unwrap();
        assert!(text.contains("FORGEJO_OWNER_USER=ana"), "{text}");
        assert!(text.contains("FORGEJO_OWNER_EMAIL=ana@oreia-seca.local"), "{text}");
        assert!(text.contains("FORGEJO_REPOS=acme/app"), "{text}");
        assert!(text.contains("FORGEJO_HTTP_PORT=3001"), "{text}");
        assert!(text.contains("PX0_PORT=7778"), "{text}");
        assert!(!text.contains("OREIA_SECA_PORT"), "porta não pedida não deve aparecer: {text}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn apply_answers_refuses_to_change_the_owner_after_bootstrap() {
        let d = dir("owner-locked");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        std::fs::write(&env_path, "FORGEJO_OWNER_USER=dono\nFORGEJO_OWNER_TOKEN=abc\n").unwrap();
        let answers = Answers {
            forgejo: ForgejoAnswers { owner_user: "ana".into(), ..Default::default() },
            ..Default::default()
        };
        let err = apply_answers(&answers, &env_path, &d.join(".env.example"), &d.join("missing.sh")).await.unwrap_err();
        assert!(err.to_string().contains("bootstrap já criou"), "{err}");

        // O mesmo dono de antes não é uma troca — passa sem tocar em nada.
        let same = Answers {
            forgejo: ForgejoAnswers { owner_user: "dono".into(), ..Default::default() },
            ..Default::default()
        };
        apply_answers(&same, &env_path, &d.join(".env.example"), &d.join("missing.sh")).await.unwrap();
        std::fs::remove_dir_all(&d).ok();
    }
}
