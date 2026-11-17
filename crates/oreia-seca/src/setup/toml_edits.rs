//! Edita `oreia-seca.toml` preservando comentários e formatação do resto do
//! arquivo via `toml_edit` — ao contrário de reserializar com `toml::Value`,
//! que apagaria todos os comentários de `oreia-seca.example.toml`. Cobre os
//! toggles de `[telegram]`/`[mcp]` e os campos avançados que a TUI de
//! `oreia-seca setup` edita (MCP, Forgejo, Servidor/Paths/Queue, modelos
//! `coder`/`chat`) — nenhum desses passa por `.env`, só pelo `.toml`.

use std::path::Path;

use toml_edit::{table, value, Array, DocumentMut};

use super::env_file::backup_path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TomlEdits {
    /// `None` = não muda o que já está no arquivo.
    pub telegram_enabled: Option<bool>,
    /// `Some(true)` cria o bloco `[mcp]` (vazio, usando os defaults de
    /// `McpConfig`) se ainda não existir; `Some(false)` remove a tabela.
    pub mcp_enabled: Option<bool>,
    pub mcp_http: Option<bool>,
    pub mcp_agent_user: Option<String>,
    pub mcp_workdir: Option<String>,
    pub mcp_max_review_rounds: Option<usize>,
    pub mcp_bind: Option<String>,

    /// Cria `[forgejo]` se faltar e algum destes for `Some`.
    pub forgejo_url: Option<String>,
    pub forgejo_public_url: Option<String>,
    pub forgejo_repos: Option<Vec<String>>,
    pub forgejo_base_branch: Option<String>,
    pub forgejo_merge_style: Option<String>,

    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub paths_data: Option<String>,
    pub paths_workspace: Option<String>,
    pub queue_concurrency: Option<usize>,
    pub queue_timeout_secs: Option<u64>,

    pub executor_default: Option<String>,
    pub executor_max_steps: Option<usize>,
    pub executor_phase_retries: Option<usize>,
    pub executor_shell_timeout_secs: Option<u64>,
    pub executor_max_phases: Option<usize>,
    pub executor_step_timeout_secs: Option<u64>,
    pub executor_go_horse: Option<bool>,

    /// Exigem que `[models.coder]`/`[models.chat]` já existam — ao contrário
    /// de `[mcp]`/`[forgejo]`, não criamos um modelo novo por aqui.
    pub models_coder_context: Option<u32>,
    pub models_coder_temperature: Option<f64>,
    pub models_chat_context: Option<u32>,
    pub models_chat_temperature: Option<f64>,
}

impl TomlEdits {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn has_nested_table(doc: &DocumentMut, parent: &str, child: &str) -> bool {
    doc.get(parent).and_then(|p| p.as_table()).is_some_and(|t| t.contains_table(child))
}

/// Aplica `edits` sobre `toml_path` (ou `toml_example_path` se `toml_path`
/// ainda não existir). Faz backup em `<toml_path>.bak` se o arquivo já
/// existia. Sem nenhum campo setado, não toca no arquivo.
pub fn apply(toml_path: &Path, toml_example_path: &Path, edits: &TomlEdits) -> anyhow::Result<()> {
    if edits.is_empty() {
        return Ok(());
    }

    let existing_text = if toml_path.exists() {
        let text = std::fs::read_to_string(toml_path)
            .map_err(|e| anyhow::anyhow!("não consegui ler {}: {e}", toml_path.display()))?;
        std::fs::copy(toml_path, backup_path(toml_path))?;
        text
    } else if toml_example_path.exists() {
        std::fs::read_to_string(toml_example_path)
            .map_err(|e| anyhow::anyhow!("não consegui ler {}: {e}", toml_example_path.display()))?
    } else {
        String::new()
    };

    let mut doc: DocumentMut = existing_text.parse().map_err(|e| anyhow::anyhow!("{}: {e}", toml_path.display()))?;

    if let Some(enabled) = edits.telegram_enabled {
        if !doc.contains_table("telegram") {
            doc["telegram"] = table();
        }
        doc["telegram"]["enabled"] = value(enabled);
    }

    match edits.mcp_enabled {
        Some(true) => {
            if !doc.contains_table("mcp") {
                doc["mcp"] = table();
            }
        }
        Some(false) => {
            doc.remove("mcp");
        }
        None => {}
    }

    let mcp_advanced_requested = edits.mcp_http.is_some()
        || edits.mcp_agent_user.is_some()
        || edits.mcp_workdir.is_some()
        || edits.mcp_max_review_rounds.is_some()
        || edits.mcp_bind.is_some();
    if mcp_advanced_requested {
        anyhow::ensure!(
            doc.contains_table("mcp"),
            "não dá para editar campos de [mcp] sem o bloco existir — habilite o MCP primeiro"
        );
    }
    if let Some(v) = &edits.mcp_agent_user {
        doc["mcp"]["agent_user"] = value(v.clone());
    }
    if let Some(v) = &edits.mcp_workdir {
        doc["mcp"]["workdir"] = value(v.clone());
    }
    if let Some(v) = edits.mcp_max_review_rounds {
        doc["mcp"]["max_review_rounds"] = value(v as i64);
    }
    if let Some(v) = &edits.mcp_bind {
        doc["mcp"]["bind"] = value(v.clone());
    }
    if let Some(http) = edits.mcp_http {
        doc["mcp"]["http"] = value(http);
    }

    let forgejo_requested = edits.forgejo_url.is_some()
        || edits.forgejo_public_url.is_some()
        || edits.forgejo_repos.is_some()
        || edits.forgejo_base_branch.is_some()
        || edits.forgejo_merge_style.is_some();
    if forgejo_requested && !doc.contains_table("forgejo") {
        doc["forgejo"] = table();
    }
    if let Some(v) = &edits.forgejo_url {
        doc["forgejo"]["url"] = value(v.clone());
    }
    if let Some(v) = &edits.forgejo_public_url {
        doc["forgejo"]["public_url"] = value(v.clone());
    }
    if let Some(repos) = &edits.forgejo_repos {
        let mut arr = Array::new();
        for r in repos {
            arr.push(r.as_str());
        }
        doc["forgejo"]["repos"] = value(arr);
    }
    if let Some(v) = &edits.forgejo_base_branch {
        doc["forgejo"]["base_branch"] = value(v.clone());
    }
    if let Some(v) = &edits.forgejo_merge_style {
        doc["forgejo"]["merge_style"] = value(v.clone());
    }

    if (edits.server_host.is_some() || edits.server_port.is_some()) && !doc.contains_table("server") {
        doc["server"] = table();
    }
    if let Some(v) = &edits.server_host {
        doc["server"]["host"] = value(v.clone());
    }
    if let Some(v) = edits.server_port {
        doc["server"]["port"] = value(v as i64);
    }

    if (edits.paths_data.is_some() || edits.paths_workspace.is_some()) && !doc.contains_table("paths") {
        doc["paths"] = table();
    }
    if let Some(v) = &edits.paths_data {
        doc["paths"]["data"] = value(v.clone());
    }
    if let Some(v) = &edits.paths_workspace {
        doc["paths"]["workspace"] = value(v.clone());
    }

    if (edits.queue_concurrency.is_some() || edits.queue_timeout_secs.is_some()) && !doc.contains_table("queue") {
        doc["queue"] = table();
    }
    if let Some(v) = edits.queue_concurrency {
        doc["queue"]["concurrency"] = value(v as i64);
    }
    if let Some(v) = edits.queue_timeout_secs {
        doc["queue"]["timeout_secs"] = value(v as i64);
    }

    let executor_touched = edits.executor_default.is_some()
        || edits.executor_max_steps.is_some()
        || edits.executor_phase_retries.is_some()
        || edits.executor_shell_timeout_secs.is_some()
        || edits.executor_max_phases.is_some()
        || edits.executor_step_timeout_secs.is_some()
        || edits.executor_go_horse.is_some();
    if executor_touched && !doc.contains_table("executor") {
        doc["executor"] = table();
    }
    if let Some(v) = &edits.executor_default {
        doc["executor"]["default"] = value(v.clone());
    }
    if let Some(v) = edits.executor_max_steps {
        doc["executor"]["max_steps"] = value(v as i64);
    }
    if let Some(v) = edits.executor_phase_retries {
        doc["executor"]["phase_retries"] = value(v as i64);
    }
    if let Some(v) = edits.executor_shell_timeout_secs {
        doc["executor"]["shell_timeout_secs"] = value(v as i64);
    }
    if let Some(v) = edits.executor_max_phases {
        doc["executor"]["max_phases"] = value(v as i64);
    }
    if let Some(v) = edits.executor_step_timeout_secs {
        doc["executor"]["step_timeout_secs"] = value(v as i64);
    }
    if let Some(v) = edits.executor_go_horse {
        doc["executor"]["go_horse"] = value(v);
    }

    if edits.models_coder_context.is_some() || edits.models_coder_temperature.is_some() {
        anyhow::ensure!(
            has_nested_table(&doc, "models", "coder"),
            "não dá para editar [models.coder] sem esse modelo existir no oreia-seca.toml"
        );
    }
    if let Some(v) = edits.models_coder_context {
        doc["models"]["coder"]["context"] = value(v as i64);
    }
    if let Some(v) = edits.models_coder_temperature {
        doc["models"]["coder"]["temperature"] = value(v);
    }

    if edits.models_chat_context.is_some() || edits.models_chat_temperature.is_some() {
        anyhow::ensure!(
            has_nested_table(&doc, "models", "chat"),
            "não dá para editar [models.chat] sem esse modelo existir no oreia-seca.toml"
        );
    }
    if let Some(v) = edits.models_chat_context {
        doc["models"]["chat"]["context"] = value(v as i64);
    }
    if let Some(v) = edits.models_chat_temperature {
        doc["models"]["chat"]["temperature"] = value(v);
    }

    std::fs::write(toml_path, doc.to_string())
        .map_err(|e| anyhow::anyhow!("não consegui gravar {}: {e}", toml_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("oreia-seca-toml-edits-{label}-{}", std::process::id()))
    }

    #[test]
    fn no_edits_leaves_file_untouched() {
        let d = dir("noop");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        apply(&toml_path, &d.join("oreia-seca.example.toml"), &TomlEdits::default()).unwrap();
        assert!(!toml_path.exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn toggles_telegram_enabled_preserving_comments() {
        let d = dir("telegram");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(
            &toml_path,
            "# comentário importante\n[telegram]\nenabled = true\ntoken_env = \"TELEGRAM_TOKEN\"\n",
        )
        .unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits { telegram_enabled: Some(false), ..Default::default() },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(text.contains("# comentário importante"), "{text}");
        assert!(text.contains("enabled = false"), "{text}");
        assert!(text.contains("token_env = \"TELEGRAM_TOKEN\""), "{text}");
        assert!(backup_path(&toml_path).exists());

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn enables_mcp_from_a_file_without_the_block() {
        let d = dir("mcp-enable");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "[telegram]\nenabled = true\n").unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits { mcp_enabled: Some(true), mcp_http: Some(true), ..Default::default() },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(text.contains("[mcp]"), "{text}");
        assert!(text.contains("http = true"), "{text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn disables_mcp_removing_the_whole_table() {
        let d = dir("mcp-disable");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "[telegram]\nenabled = true\n\n[mcp]\nhttp = true\nbind = \"127.0.0.1:9\"\n")
            .unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits { mcp_enabled: Some(false), ..Default::default() },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(!text.contains("[mcp]"), "{text}");
        assert!(text.contains("[telegram]"), "{text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn mcp_http_without_the_block_is_an_error() {
        let d = dir("mcp-http-error");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "[telegram]\nenabled = true\n").unwrap();

        let err = apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits { mcp_http: Some(true), ..Default::default() },
        )
        .unwrap_err();
        assert!(err.to_string().contains("[mcp]"), "{err}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn mcp_advanced_fields_write_into_an_existing_block() {
        let d = dir("mcp-advanced");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "[mcp]\nenabled = true\n").unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits {
                mcp_agent_user: Some("agent-mcp-2".into()),
                mcp_workdir: Some("workspace/mcp2".into()),
                mcp_max_review_rounds: Some(5),
                mcp_bind: Some("127.0.0.1:9999".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(text.contains(r#"agent_user = "agent-mcp-2""#), "{text}");
        assert!(text.contains(r#"workdir = "workspace/mcp2""#), "{text}");
        assert!(text.contains("max_review_rounds = 5"), "{text}");
        assert!(text.contains(r#"bind = "127.0.0.1:9999""#), "{text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn creates_from_example_when_toml_is_missing() {
        let d = dir("from-example");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        let example_path = d.join("oreia-seca.example.toml");
        std::fs::write(&example_path, "[telegram]\nenabled = true\n").unwrap();

        apply(&toml_path, &example_path, &TomlEdits { telegram_enabled: Some(false), ..Default::default() }).unwrap();

        assert!(!backup_path(&toml_path).exists(), "não devia haver backup sem oreia-seca.toml prévio");
        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(text.contains("enabled = false"), "{text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn creates_forgejo_block_and_writes_repos_as_an_array() {
        let d = dir("forgejo-create");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "[telegram]\nenabled = true\n").unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits {
                forgejo_url: Some("http://forgejo:3000".into()),
                forgejo_repos: Some(vec!["acme/app".into(), "acme/site".into()]),
                forgejo_base_branch: Some("main".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(text.contains("[forgejo]"), "{text}");
        assert!(text.contains(r#"url = "http://forgejo:3000""#), "{text}");
        assert!(text.contains("acme/app") && text.contains("acme/site"), "{text}");
        assert!(text.contains(r#"base_branch = "main""#), "{text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn edits_existing_forgejo_block_in_place() {
        let d = dir("forgejo-edit");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "[forgejo]\nurl = \"http://old:3000\"\nrepos = [\"a/b\"]\n").unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits { forgejo_url: Some("http://new:3000".into()), ..Default::default() },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(text.contains("http://new:3000"), "{text}");
        assert!(text.contains("a/b"), "repos não devia ter sido tocado: {text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn server_paths_queue_create_tables_when_missing() {
        let d = dir("server-create");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "[telegram]\nenabled = true\n").unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits {
                server_host: Some("127.0.0.1".into()),
                server_port: Some(4000),
                paths_data: Some("/data".into()),
                queue_concurrency: Some(2),
                queue_timeout_secs: Some(1200),
                ..Default::default()
            },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(
            text.contains("[server]") && text.contains(r#"host = "127.0.0.1""#) && text.contains("port = 4000"),
            "{text}"
        );
        assert!(text.contains("[paths]") && text.contains(r#"data = "/data""#), "{text}");
        assert!(
            text.contains("[queue]") && text.contains("concurrency = 2") && text.contains("timeout_secs = 1200"),
            "{text}"
        );

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn models_coder_and_chat_require_the_model_to_already_exist() {
        let d = dir("models-missing");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "[telegram]\nenabled = true\n").unwrap();

        let err = apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits { models_coder_context: Some(32768), ..Default::default() },
        )
        .unwrap_err();
        assert!(err.to_string().contains("models.coder"), "{err}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn models_coder_and_chat_edit_in_place_when_present() {
        let d = dir("models-present");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(
            &toml_path,
            "[models.coder]\nbase_url = \"http://x/v1\"\nmodel = \"m\"\ncontext = 8192\n\n[models.chat]\nbase_url = \"http://x/v1\"\nmodel = \"m\"\n",
        )
        .unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits {
                models_coder_context: Some(32768),
                models_coder_temperature: Some(0.3),
                models_chat_temperature: Some(0.7),
                ..Default::default()
            },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(text.contains("context = 32768"), "{text}");
        assert!(text.contains("temperature = 0.3"), "{text}");
        assert!(text.contains("temperature = 0.7"), "{text}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn executor_edits_in_place_and_creates_table_when_missing() {
        let d = dir("executor-edit");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "# cabeçalho\n[executor]\ndefault = \"builtin\"\nmax_steps = 4\n").unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits {
                executor_default: Some("opencode".to_string()),
                executor_go_horse: Some(true),
                executor_max_steps: Some(6),
                executor_phase_retries: Some(3),
                executor_shell_timeout_secs: Some(180),
                executor_max_phases: Some(10),
                executor_step_timeout_secs: Some(600),
                ..Default::default()
            },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(text.contains("# cabeçalho"));
        assert!(text.contains("default = \"opencode\""));
        assert!(text.contains("max_steps = 6"));
        assert!(text.contains("phase_retries = 3"));
        assert!(text.contains("shell_timeout_secs = 180"));
        assert!(text.contains("max_phases = 10"));
        assert!(text.contains("step_timeout_secs = 600"));
        assert!(text.contains("go_horse = true"));

        std::fs::remove_dir_all(&d).ok();
    }

    /// `go_horse` sozinho (sem os outros campos de `[executor]`) ainda cria a
    /// tabela se faltar — regressão do `executor_touched` esquecer o campo novo.
    #[test]
    fn executor_go_horse_alone_touches_the_table() {
        let d = dir("executor-go-horse-alone");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "# cabeçalho\n").unwrap();

        apply(
            &toml_path,
            &d.join("oreia-seca.example.toml"),
            &TomlEdits { executor_go_horse: Some(true), ..Default::default() },
        )
        .unwrap();

        let text = std::fs::read_to_string(&toml_path).unwrap();
        assert!(text.contains("[executor]"), "{text}");
        assert!(text.contains("go_horse = true"), "{text}");

        std::fs::remove_dir_all(&d).ok();
    }
}
