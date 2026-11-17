//! Snapshot do que já está configurado em `.env`/`oreia-seca.toml`, para a
//! TUI de `oreia-seca setup` pré-popular os campos em vez de perguntar do
//! zero toda vez. Campos que parecem segredo (token, chave) nunca expõem o
//! valor — só se já estão definidos ou não.
//!
//! Números/listas do `.toml` são lidos já como `String` (prontos pra um
//! `TextField` da TUI) — quem valida e converte de volta é
//! `tui::app::App::build_save_payload`.

use std::path::Path;

use toml_edit::DocumentMut;

use super::env_file::read_env_value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CurrentState {
    /// `false` = primeira instalação, só há `oreia-seca.example.toml`.
    pub toml_path_exists: bool,

    pub telegram_enabled: bool,
    pub telegram_token_set: bool,
    pub telegram_allowed: String,

    /// Bloco `[mcp]` presente e não comentado no `.toml`.
    pub mcp_block_present: bool,
    pub mcp_http: bool,
    pub mcp_token_set: bool,
    pub mcp_agent_user: String,
    pub mcp_workdir: String,
    pub mcp_max_review_rounds: String,
    pub mcp_bind: String,

    pub forgejo_url: String,
    pub forgejo_public_url: String,
    /// Repositórios separados por `, ` (um `Vec<String>` no `.toml`).
    pub forgejo_repos: String,
    pub forgejo_base_branch: String,
    pub forgejo_merge_style: String,
    pub forgejo_owner_token_set: bool,
    pub forgejo_webhook_secret_set: bool,

    pub llm_url: String,
    pub llm_coder_model: String,
    pub llm_chat_model: String,
    pub llm_api_key_set: bool,
    /// Vazio = não customizado no `.toml` (usa o default do `ModelSpec`).
    pub models_coder_context: String,
    pub models_coder_temperature: String,
    pub models_chat_context: String,
    pub models_chat_temperature: String,

    /// Sempre têm um valor efetivo (default do `Config` se o bloco faltar),
    /// por isso pré-populados com o literal certo, nunca vazio.
    pub server_host: String,
    pub server_port: String,
    pub paths_data: String,
    pub paths_workspace: String,
    pub queue_concurrency: String,
    pub queue_timeout_secs: String,
    pub rest_api_key_set: bool,

    pub executor_default: String,
    pub executor_max_steps: String,
    pub executor_phase_retries: String,
    pub executor_shell_timeout_secs: String,
    pub executor_max_phases: String,
    pub executor_step_timeout_secs: String,
    pub executor_go_horse: bool,

    pub px0_allowed_users: String,
    pub px0_forgejo_token_set: bool,
    pub px0_oauth_client_id_set: bool,
    pub px0_oauth_client_secret_set: bool,
}

pub fn load(env_path: &Path, toml_path: &Path) -> CurrentState {
    let doc = std::fs::read_to_string(toml_path).ok().and_then(|t| t.parse::<DocumentMut>().ok());
    let doc = doc.as_ref();

    CurrentState {
        toml_path_exists: toml_path.exists(),

        telegram_enabled: table_bool(doc, "telegram", "enabled", true),
        telegram_token_set: read_env_value(env_path, "TELEGRAM_TOKEN").is_some(),
        telegram_allowed: read_env_value(env_path, "TELEGRAM_ALLOWED").unwrap_or_default(),

        mcp_block_present: table_present(doc, "mcp"),
        mcp_http: table_bool(doc, "mcp", "http", false),
        mcp_token_set: read_env_value(env_path, "MCP_TOKEN").is_some(),
        mcp_agent_user: table_str(doc, "mcp", "agent_user"),
        mcp_workdir: table_str(doc, "mcp", "workdir"),
        mcp_max_review_rounds: table_int_opt(doc, "mcp", "max_review_rounds"),
        mcp_bind: table_str(doc, "mcp", "bind"),

        forgejo_url: table_str(doc, "forgejo", "url"),
        forgejo_public_url: table_str(doc, "forgejo", "public_url"),
        forgejo_repos: forgejo_repos_str(doc),
        forgejo_base_branch: table_str(doc, "forgejo", "base_branch"),
        forgejo_merge_style: table_str(doc, "forgejo", "merge_style"),
        forgejo_owner_token_set: read_env_value(env_path, "FORGEJO_OWNER_TOKEN").is_some(),
        forgejo_webhook_secret_set: read_env_value(env_path, "FORGEJO_WEBHOOK_SECRET").is_some(),

        llm_url: read_env_value(env_path, "LLM_CODER_URL").unwrap_or_default(),
        llm_coder_model: read_env_value(env_path, "LLM_CODER_MODEL").unwrap_or_default(),
        llm_chat_model: read_env_value(env_path, "LLM_CHAT_MODEL").unwrap_or_default(),
        llm_api_key_set: read_env_value(env_path, "LLM_API_KEY").is_some(),
        models_coder_context: nested_int_opt(doc, "models", "coder", "context"),
        models_coder_temperature: nested_float_opt(doc, "models", "coder", "temperature"),
        models_chat_context: nested_int_opt(doc, "models", "chat", "context"),
        models_chat_temperature: nested_float_opt(doc, "models", "chat", "temperature"),

        server_host: table_str_or(doc, "server", "host", "0.0.0.0"),
        server_port: table_int_or(doc, "server", "port", 3120),
        paths_data: table_str_or(doc, "paths", "data", "data"),
        paths_workspace: table_str_or(doc, "paths", "workspace", "workspace"),
        queue_concurrency: table_int_or(doc, "queue", "concurrency", 1),
        queue_timeout_secs: table_int_or(doc, "queue", "timeout_secs", 3600),
        rest_api_key_set: read_env_value(env_path, "REST_API_KEY").is_some(),

        executor_default: table_str_or(doc, "executor", "default", "builtin"),
        executor_max_steps: table_int_or(doc, "executor", "max_steps", 4),
        executor_phase_retries: table_int_or(doc, "executor", "phase_retries", 2),
        executor_shell_timeout_secs: table_int_or(doc, "executor", "shell_timeout_secs", 300),
        executor_max_phases: table_int_or(doc, "executor", "max_phases", 8),
        executor_step_timeout_secs: table_int_or(doc, "executor", "step_timeout_secs", 900),
        executor_go_horse: table_bool(doc, "executor", "go_horse", false),

        px0_allowed_users: read_env_value(env_path, "PX0_ALLOWED_USERS").unwrap_or_default(),
        px0_forgejo_token_set: read_env_value(env_path, "FORGEJO_OWNER_TOKEN").is_some(),
        px0_oauth_client_id_set: read_env_value(env_path, "PX0_OAUTH_CLIENT_ID").is_some(),
        px0_oauth_client_secret_set: read_env_value(env_path, "PX0_OAUTH_CLIENT_SECRET").is_some(),
    }
}

fn table_present(doc: Option<&DocumentMut>, table: &str) -> bool {
    doc.and_then(|d| d.get(table)).is_some_and(|t| t.is_table())
}

fn table_bool(doc: Option<&DocumentMut>, table: &str, key: &str, default: bool) -> bool {
    doc.and_then(|d| d.get(table))
        .and_then(|t| t.as_table())
        .and_then(|t| t.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

/// String vazia se o campo não existir — usado nos campos "avançados"
/// (opcionais de verdade, nunca re-gravados se deixados em branco).
fn table_str(doc: Option<&DocumentMut>, table: &str, key: &str) -> String {
    doc.and_then(|d| d.get(table))
        .and_then(|t| t.as_table())
        .and_then(|t| t.get(key))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default()
}

fn table_str_or(doc: Option<&DocumentMut>, table: &str, key: &str, default: &str) -> String {
    let s = table_str(doc, table, key);
    if s.is_empty() {
        default.to_string()
    } else {
        s
    }
}

fn table_int_opt(doc: Option<&DocumentMut>, table: &str, key: &str) -> String {
    doc.and_then(|d| d.get(table))
        .and_then(|t| t.as_table())
        .and_then(|t| t.get(key))
        .and_then(|v| v.as_integer())
        .map(|v| v.to_string())
        .unwrap_or_default()
}

fn table_int_or(doc: Option<&DocumentMut>, table: &str, key: &str, default: i64) -> String {
    doc.and_then(|d| d.get(table))
        .and_then(|t| t.as_table())
        .and_then(|t| t.get(key))
        .and_then(|v| v.as_integer())
        .unwrap_or(default)
        .to_string()
}

fn nested_int_opt(doc: Option<&DocumentMut>, parent: &str, child: &str, key: &str) -> String {
    doc.and_then(|d| d.get(parent))
        .and_then(|p| p.as_table())
        .and_then(|p| p.get(child))
        .and_then(|c| c.as_table())
        .and_then(|c| c.get(key))
        .and_then(|v| v.as_integer())
        .map(|v| v.to_string())
        .unwrap_or_default()
}

fn nested_float_opt(doc: Option<&DocumentMut>, parent: &str, child: &str, key: &str) -> String {
    doc.and_then(|d| d.get(parent))
        .and_then(|p| p.as_table())
        .and_then(|p| p.get(child))
        .and_then(|c| c.as_table())
        .and_then(|c| c.get(key))
        .and_then(|v| v.as_float())
        .map(|v| v.to_string())
        .unwrap_or_default()
}

fn forgejo_repos_str(doc: Option<&DocumentMut>) -> String {
    doc.and_then(|d| d.get("forgejo"))
        .and_then(|t| t.as_table())
        .and_then(|t| t.get("repos"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("oreia-seca-current-state-{label}-{}", std::process::id()))
    }

    #[test]
    fn missing_files_fall_back_to_defaults() {
        let d = dir("missing");
        let state = load(&d.join(".env"), &d.join("oreia-seca.toml"));
        assert!(!state.toml_path_exists);
        assert!(state.telegram_enabled);
        assert!(!state.mcp_block_present);
        assert!(!state.telegram_token_set);
        assert_eq!(state.px0_allowed_users, "");
        assert_eq!(state.server_host, "0.0.0.0");
        assert_eq!(state.server_port, "3120");
        assert_eq!(state.paths_data, "data");
        assert_eq!(state.queue_concurrency, "1");
        assert_eq!(state.forgejo_url, "", "forgejo é opcional, não tem default fabricado");
        assert_eq!(state.models_coder_context, "");
    }

    #[test]
    fn reads_telegram_disabled_and_env_values() {
        let d = dir("telegram-disabled");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&env_path, "TELEGRAM_TOKEN=abc\nTELEGRAM_ALLOWED=1,2\n").unwrap();
        std::fs::write(&toml_path, "[telegram]\nenabled = false\n").unwrap();

        let state = load(&env_path, &toml_path);
        assert!(state.toml_path_exists);
        assert!(!state.telegram_enabled);
        assert!(state.telegram_token_set);
        assert_eq!(state.telegram_allowed, "1,2");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn mcp_block_commented_out_is_not_present() {
        let d = dir("mcp-commented");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "# [mcp]\n# enabled = true\n# http = true\n").unwrap();

        let state = load(&d.join(".env"), &toml_path);
        assert!(!state.mcp_block_present);
        assert!(!state.mcp_http);

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn mcp_block_present_reads_http_toggle_and_advanced_fields() {
        let d = dir("mcp-present");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&env_path, "MCP_TOKEN=tok\n").unwrap();
        std::fs::write(
            &toml_path,
            "[mcp]\nenabled = true\nhttp = true\nagent_user = \"agent-mcp\"\nworkdir = \"workspace/mcp\"\nmax_review_rounds = 3\nbind = \"127.0.0.1:3121\"\n",
        )
        .unwrap();

        let state = load(&env_path, &toml_path);
        assert!(state.mcp_block_present);
        assert!(state.mcp_http);
        assert!(state.mcp_token_set);
        assert_eq!(state.mcp_agent_user, "agent-mcp");
        assert_eq!(state.mcp_workdir, "workspace/mcp");
        assert_eq!(state.mcp_max_review_rounds, "3");
        assert_eq!(state.mcp_bind, "127.0.0.1:3121");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reads_forgejo_block_with_repos_joined_by_comma() {
        let d = dir("forgejo");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(
            &toml_path,
            "[forgejo]\nurl = \"http://forgejo:3000\"\nrepos = [\"acme/app\", \"acme/site\"]\nbase_branch = \"main\"\n",
        )
        .unwrap();

        let state = load(&d.join(".env"), &toml_path);
        assert_eq!(state.forgejo_url, "http://forgejo:3000");
        assert_eq!(state.forgejo_repos, "acme/app, acme/site");
        assert_eq!(state.forgejo_base_branch, "main");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reads_models_coder_and_chat_context_and_temperature() {
        let d = dir("models");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(
            &toml_path,
            "[models.coder]\nbase_url = \"http://x/v1\"\nmodel = \"m\"\ncontext = 32768\ntemperature = 0.2\n\n[models.chat]\nbase_url = \"http://x/v1\"\nmodel = \"m\"\n",
        )
        .unwrap();

        let state = load(&d.join(".env"), &toml_path);
        assert_eq!(state.models_coder_context, "32768");
        assert_eq!(state.models_coder_temperature, "0.2");
        assert_eq!(state.models_chat_context, "", "chat não tem context customizado");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn invalid_toml_is_treated_as_defaults_not_a_crash() {
        let d = dir("invalid-toml");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(&toml_path, "not valid toml [[[").unwrap();

        let state = load(&d.join(".env"), &toml_path);
        assert!(state.toml_path_exists);
        assert!(state.telegram_enabled);
        assert!(!state.mcp_block_present);
        assert_eq!(state.server_port, "3120");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn px0_and_llm_secret_fields_never_expose_the_value() {
        let d = dir("secrets");
        std::fs::create_dir_all(&d).unwrap();
        let env_path = d.join(".env");
        std::fs::write(
            &env_path,
            "LLM_API_KEY=super-secret\nFORGEJO_OWNER_TOKEN=tok\nFORGEJO_WEBHOOK_SECRET=sec2\nPX0_OAUTH_CLIENT_ID=id\nPX0_OAUTH_CLIENT_SECRET=sec\nPX0_ALLOWED_USERS=dono\nREST_API_KEY=apikey\n",
        )
        .unwrap();

        let state = load(&env_path, &d.join("oreia-seca.toml"));
        assert!(state.llm_api_key_set);
        assert!(state.px0_forgejo_token_set);
        assert!(state.forgejo_owner_token_set);
        assert!(state.forgejo_webhook_secret_set);
        assert!(state.px0_oauth_client_id_set);
        assert!(state.px0_oauth_client_secret_set);
        assert!(state.rest_api_key_set);
        assert_eq!(state.px0_allowed_users, "dono");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn server_paths_queue_read_custom_values_when_present() {
        let d = dir("server-custom");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(
            &toml_path,
            "[server]\nhost = \"127.0.0.1\"\nport = 4000\n\n[paths]\ndata = \"/data\"\nworkspace = \"/ws\"\n\n[queue]\nconcurrency = 3\ntimeout_secs = 60\n",
        )
        .unwrap();

        let state = load(&d.join(".env"), &toml_path);
        assert_eq!(state.server_host, "127.0.0.1");
        assert_eq!(state.server_port, "4000");
        assert_eq!(state.paths_data, "/data");
        assert_eq!(state.paths_workspace, "/ws");
        assert_eq!(state.queue_concurrency, "3");
        assert_eq!(state.queue_timeout_secs, "60");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn executor_reads_custom_values_when_present() {
        let d = dir("executor-custom");
        std::fs::create_dir_all(&d).unwrap();
        let toml_path = d.join("oreia-seca.toml");
        std::fs::write(
            &toml_path,
            "[executor]\ndefault = \"opencode\"\nmax_steps = 6\nphase_retries = 3\nshell_timeout_secs = 120\nmax_phases = 10\nstep_timeout_secs = 600\ngo_horse = true\n",
        )
        .unwrap();

        let state = load(&d.join(".env"), &toml_path);
        assert_eq!(state.executor_default, "opencode");
        assert_eq!(state.executor_max_steps, "6");
        assert_eq!(state.executor_phase_retries, "3");
        assert_eq!(state.executor_shell_timeout_secs, "120");
        assert_eq!(state.executor_max_phases, "10");
        assert_eq!(state.executor_step_timeout_secs, "600");
        assert!(state.executor_go_horse);

        std::fs::remove_dir_all(&d).ok();
    }
}
