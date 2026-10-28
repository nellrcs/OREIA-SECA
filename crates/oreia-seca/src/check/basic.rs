//! Checagens sem rede complexa: Telegram (`getMe`), diretórios de `[paths]`
//! (existem e têm escrita) e `[server]` (host/porta minimamente plausíveis).

use std::time::Duration;

use serde::Deserialize;

use super::CheckStatus;
use crate::config::{env, Config};

#[derive(Deserialize)]
struct TgResponse {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
}

pub async fn check_telegram(cfg: &Config) -> (CheckStatus, String) {
    if !cfg.telegram.enabled {
        return (CheckStatus::Ok, "telegram: desativado em [telegram].enabled".to_string());
    }
    let Some(token) = env(&cfg.telegram.token_env) else {
        return (CheckStatus::Warn, format!("telegram: desativado (falta a variável {})", cfg.telegram.token_env));
    };

    let allowed_raw = env(&cfg.telegram.allowed_env).unwrap_or_default();
    let ids: Vec<&str> = allowed_raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if let Some(bad) = ids.iter().find(|s| s.parse::<i64>().is_err()) {
        return (
            CheckStatus::Fail,
            format!("telegram: \"{bad}\" em {} não é um ID numérico", cfg.telegram.allowed_env),
        );
    }

    let client = match reqwest::Client::builder().timeout(Duration::from_secs(10)).build() {
        Ok(c) => c,
        Err(e) => return (CheckStatus::Fail, format!("telegram: não consegui montar o cliente HTTP: {e}")),
    };
    let resp = match client.get(format!("https://api.telegram.org/bot{token}/getMe")).send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return (CheckStatus::Warn, "telegram: getMe não respondeu a tempo".to_string()),
        Err(e) if e.is_connect() => return (CheckStatus::Warn, "telegram: conexão recusada".to_string()),
        Err(e) => return (CheckStatus::Warn, format!("telegram: getMe — {e}")),
    };
    let status = resp.status();
    let body: Result<TgResponse, _> = resp.json().await;
    match body {
        Ok(b) if b.ok => {
            if ids.is_empty() {
                (CheckStatus::Warn, "telegram: token ok, mas TELEGRAM_ALLOWED está vazio — ninguém acessa".to_string())
            } else {
                (CheckStatus::Ok, format!("telegram: token ok, {} ID(s) na allowlist", ids.len()))
            }
        }
        Ok(b) => (CheckStatus::Fail, format!("telegram: token rejeitado ({})", b.description.unwrap_or_default())),
        Err(_) if status.as_u16() == 401 => (CheckStatus::Fail, "telegram: token rejeitado (401)".to_string()),
        Err(e) => (CheckStatus::Warn, format!("telegram: resposta inesperada de getMe ({e})")),
    }
}

pub async fn check_paths(cfg: &Config) -> (CheckStatus, String) {
    for (label, path) in [("data", &cfg.paths.data), ("workspace", &cfg.paths.workspace), ("skills", &cfg.paths.skills)]
    {
        if label == "skills" {
            // Skills são só lidas, nunca escritas pelo orquestrador — só precisa existir.
            if !path.exists() {
                return (CheckStatus::Warn, format!("paths.{label}: {} não existe ainda", path.display()));
            }
            continue;
        }
        if let Err(e) = tokio::fs::create_dir_all(path).await {
            return (CheckStatus::Fail, format!("paths.{label}: não consegui criar {}: {e}", path.display()));
        }
        let probe = path.join(".oreia-seca-check-write-probe");
        if let Err(e) = tokio::fs::write(&probe, b"ok").await {
            return (CheckStatus::Fail, format!("paths.{label}: {} não tem permissão de escrita: {e}", path.display()));
        }
        tokio::fs::remove_file(&probe).await.ok();
    }
    (CheckStatus::Ok, "paths: data e workspace existem e têm escrita".to_string())
}

pub async fn check_server(cfg: &Config) -> (CheckStatus, String) {
    if cfg.server.host.trim().is_empty() {
        return (CheckStatus::Fail, "server.host está vazio".to_string());
    }
    if cfg.server.port == 0 {
        return (CheckStatus::Fail, "server.port não pode ser 0".to_string());
    }
    if env(&cfg.server.api_key_env).is_none() {
        return (
            CheckStatus::Warn,
            format!("server: API REST sem autenticação (falta a variável {})", cfg.server.api_key_env),
        );
    }
    (CheckStatus::Ok, format!("server: {}:{} com token configurado", cfg.server.host, cfg.server.port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::SAMPLE;

    fn cfg(toml: &str) -> Config {
        let c: Config = toml::from_str(toml).unwrap();
        c.validate().unwrap();
        c
    }

    #[tokio::test]
    async fn telegram_disabled_is_ok() {
        let toml = format!("{SAMPLE}\n[telegram]\nenabled = false\n");
        let c = cfg(&toml);
        let (status, _) = check_telegram(&c).await;
        assert_eq!(status, CheckStatus::Ok);
    }

    #[tokio::test]
    async fn telegram_without_token_env_is_a_warning() {
        std::env::remove_var("OREIA_SECA_CHECK_TEST_TG_TOKEN");
        let toml = format!("{SAMPLE}\n[telegram]\ntoken_env = \"OREIA_SECA_CHECK_TEST_TG_TOKEN\"\n");
        let c = cfg(&toml);
        let (status, _) = check_telegram(&c).await;
        assert_eq!(status, CheckStatus::Warn);
    }

    #[tokio::test]
    async fn paths_creates_data_and_workspace_and_confirms_write_access() {
        let base = std::env::temp_dir().join(format!("oreia-seca-check-paths-{}", std::process::id()));
        tokio::fs::create_dir_all(base.join("skills")).await.unwrap();
        let toml = format!(
            "{SAMPLE}\n[paths]\ndata = \"{}\"\nworkspace = \"{}\"\nskills = \"{}\"\n",
            base.join("data").to_string_lossy().replace('\\', "\\\\"),
            base.join("workspace").to_string_lossy().replace('\\', "\\\\"),
            base.join("skills").to_string_lossy().replace('\\', "\\\\"),
        );
        let c = cfg(&toml);
        let (status, msg) = check_paths(&c).await;
        assert_eq!(status, CheckStatus::Ok, "{msg}");
        assert!(base.join("data").is_dir());
        assert!(base.join("workspace").is_dir());
        tokio::fs::remove_dir_all(&base).await.ok();
    }

    #[tokio::test]
    async fn server_without_api_key_env_is_a_warning() {
        std::env::remove_var("OREIA_SECA_CHECK_TEST_API_KEY");
        let toml = format!("{SAMPLE}\n[server]\napi_key_env = \"OREIA_SECA_CHECK_TEST_API_KEY\"\n");
        let c = cfg(&toml);
        let (status, _) = check_server(&c).await;
        assert_eq!(status, CheckStatus::Warn);
    }
}
