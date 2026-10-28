//! Checagem de `[forgejo]` (saúde da API, token do dono, repositórios) e de
//! cada `[[agents]]` (o token do agente aponta pro usuário certo).

use std::time::Duration;

use oreia_forgejo::{split_repo, ForgejoClient, ForgejoError};

use super::CheckStatus;
use crate::config::{env, Config};

fn network_msg(e: &ForgejoError) -> (CheckStatus, String) {
    match e {
        ForgejoError::Api { status, .. } if *status == 401 || *status == 403 => {
            (CheckStatus::Fail, format!("token rejeitado (HTTP {status})"))
        }
        ForgejoError::Api { status, .. } if *status == 404 => (CheckStatus::Fail, "não encontrado (404)".to_string()),
        ForgejoError::Api { status, body } => {
            (CheckStatus::Warn, format!("HTTP {status}: {}", body.chars().take(200).collect::<String>()))
        }
        ForgejoError::Request(e) if e.is_timeout() => (CheckStatus::Warn, "não respondeu a tempo".to_string()),
        ForgejoError::Request(e) if e.is_connect() => {
            (CheckStatus::Warn, "conexão recusada (o Forgejo pode estar desligado)".to_string())
        }
        ForgejoError::Request(e) => (CheckStatus::Warn, e.to_string()),
    }
}

pub async fn check_forgejo(cfg: &Config) -> (CheckStatus, String) {
    let Some(fcfg) = &cfg.forgejo else {
        return (CheckStatus::Fail, "forgejo: seção [forgejo] não existe mais".to_string());
    };

    let client = match reqwest::Client::builder().timeout(Duration::from_secs(10)).build() {
        Ok(c) => c,
        Err(e) => return (CheckStatus::Fail, format!("forgejo: não consegui montar o cliente HTTP: {e}")),
    };
    let healthz = format!("{}/api/healthz", fcfg.url.trim_end_matches('/'));
    match client.get(&healthz).send().await {
        Ok(r) if !r.status().is_success() => {
            return (CheckStatus::Warn, format!("forgejo: {healthz} respondeu {}", r.status()));
        }
        Err(e) if e.is_timeout() => return (CheckStatus::Warn, "forgejo: healthz não respondeu a tempo".to_string()),
        Err(e) if e.is_connect() => {
            return (CheckStatus::Warn, "forgejo: conexão recusada (pode estar desligado)".to_string());
        }
        Err(e) => return (CheckStatus::Warn, format!("forgejo: healthz — {e}")),
        Ok(_) => {}
    }

    let Some(owner_token) = env(&fcfg.owner_token_env) else {
        return (CheckStatus::Warn, format!("forgejo: falta a variável {} (token do dono)", fcfg.owner_token_env));
    };
    let owner = ForgejoClient::new(&fcfg.url, &owner_token);
    let who = match owner.whoami().await {
        Ok(u) => u,
        Err(e) => {
            let (status, msg) = network_msg(&e);
            return (status, format!("forgejo: token do dono — {msg}"));
        }
    };

    for full in &fcfg.repos {
        let Some((o, r)) = split_repo(full) else {
            return (CheckStatus::Fail, format!("forgejo: \"{full}\" em [forgejo].repos não é \"dono/repo\""));
        };
        match owner.repo_exists(o, r).await {
            Ok(true) => {}
            Ok(false) => {
                return (CheckStatus::Fail, format!("forgejo: repositório \"{full}\" não existe"));
            }
            Err(e) => {
                let (status, msg) = network_msg(&e);
                return (status, format!("forgejo: repositório \"{full}\" — {msg}"));
            }
        }
    }

    (
        CheckStatus::Ok,
        format!(
            "forgejo: {} ok, token do dono é \"{}\", {} repo(s) acessível(is)",
            fcfg.url,
            who.login,
            fcfg.repos.len()
        ),
    )
}

pub async fn check_agent_token(cfg: &Config, agent_name: &str) -> (CheckStatus, String) {
    let Some(agent) = cfg.agent(agent_name) else {
        return (CheckStatus::Fail, format!("agente \"{agent_name}\" não existe mais em [[agents]]"));
    };
    let Some(fcfg) = &cfg.forgejo else {
        return (CheckStatus::Fail, format!("agente \"{agent_name}\": há [[agents]] mas falta [forgejo]"));
    };
    let Some(token) = env(&agent.token_env) else {
        return (
            CheckStatus::Warn,
            format!("agente \"{agent_name}\": falta a variável {} (token do agente)", agent.token_env),
        );
    };

    let client = ForgejoClient::new(&fcfg.url, &token);
    match client.whoami().await {
        Ok(who) if who.login.eq_ignore_ascii_case(&agent.user) => {
            (CheckStatus::Ok, format!("agente \"{agent_name}\": token aponta pro usuário \"{}\"", who.login))
        }
        Ok(who) => (
            CheckStatus::Fail,
            format!("agente \"{agent_name}\": token aponta pro usuário \"{}\", esperado \"{}\"", who.login, agent.user),
        ),
        Err(e) => {
            let (status, msg) = network_msg(&e);
            (status, format!("agente \"{agent_name}\": {msg}"))
        }
    }
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

    async fn fake_forgejo(login_for_token: fn(&str) -> &'static str) -> String {
        let app = axum::Router::new()
            .route("/api/healthz", axum::routing::get(|| async { "ok" }))
            .route(
                "/api/v1/user",
                axum::routing::get(move |headers: axum::http::HeaderMap| async move {
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
                    let token = auth.strip_prefix("token ").unwrap_or("");
                    axum::Json(serde_json::json!({ "login": login_for_token(token) }))
                }),
            )
            .route("/api/v1/repos/acme/app", axum::routing::get(|| async { "{}" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn forgejo_ok_when_healthz_whoami_and_repos_all_succeed() {
        std::env::set_var("OREIA_SECA_CHECK_TEST_OWNER_TOKEN", "owner-tok");
        let url = fake_forgejo(|_| "dono").await;
        let toml = format!(
            "{}\n[forgejo]\nurl = \"{url}\"\nrepos = [\"acme/app\"]\nowner_token_env = \"OREIA_SECA_CHECK_TEST_OWNER_TOKEN\"\n",
            SAMPLE.replace("[forgejo]\nurl = \"http://forgejo:3000\"\nrepos = [\"acme/app\"]", "")
        );
        let c = cfg(&toml);
        let (status, msg) = check_forgejo(&c).await;
        assert_eq!(status, CheckStatus::Ok, "{msg}");
    }

    #[tokio::test]
    async fn forgejo_fails_when_a_configured_repo_does_not_exist() {
        std::env::set_var("OREIA_SECA_CHECK_TEST_OWNER_TOKEN_2", "owner-tok");
        let app = axum::Router::new()
            .route("/api/healthz", axum::routing::get(|| async { "ok" }))
            .route("/api/v1/user", axum::routing::get(|| async { axum::Json(serde_json::json!({ "login": "dono" })) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("http://{addr}");

        let toml = format!(
            "{}\n[forgejo]\nurl = \"{url}\"\nrepos = [\"acme/missing\"]\nowner_token_env = \"OREIA_SECA_CHECK_TEST_OWNER_TOKEN_2\"\n",
            SAMPLE.replace("[forgejo]\nurl = \"http://forgejo:3000\"\nrepos = [\"acme/app\"]", "")
        );
        let c = cfg(&toml);
        let (status, msg) = check_forgejo(&c).await;
        assert_eq!(status, CheckStatus::Fail, "{msg}");
        assert!(msg.contains("acme/missing"), "{msg}");
    }

    #[tokio::test]
    async fn agent_token_pointing_at_the_wrong_user_fails() {
        std::env::set_var("OREIA_SECA_CHECK_TEST_AGENT_TOKEN", "agent-tok");
        let url = fake_forgejo(|_| "outro-usuario").await;
        let toml = format!(
            "{}\n[forgejo]\nurl = \"{url}\"\nrepos = [\"acme/app\"]\n",
            SAMPLE.replace("[forgejo]\nurl = \"http://forgejo:3000\"\nrepos = [\"acme/app\"]", "")
        );
        let toml =
            toml.replace("token_env = \"AGENT_DEV_BACK_TOKEN\"", "token_env = \"OREIA_SECA_CHECK_TEST_AGENT_TOKEN\"");
        let c = cfg(&toml);
        let (status, msg) = check_agent_token(&c, "dev-back").await;
        assert_eq!(status, CheckStatus::Fail, "{msg}");
        assert!(msg.contains("outro-usuario") && msg.contains("agent-back"), "{msg}");
    }
}
