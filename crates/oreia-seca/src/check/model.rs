//! Checagem de `[models.*]` e `[roles.*]`: resolve as variáveis, confere que
//! `GET {base_url}/models` responde e que o modelo configurado está na lista,
//! e só então arrisca uma chamada de geração mínima — pulada para modelos
//! pagos remotos (ver [`crate::setup::llm::looks_local`]), para não gastar
//! dinheiro só por validar a configuração.

use std::time::Duration;

use serde::Deserialize;

use super::CheckStatus;
use crate::config::{env, Config};
use crate::setup::llm::looks_local;

fn network_msg(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "não respondeu a tempo".to_string()
    } else if e.is_connect() {
        "conexão recusada (a máquina pode estar desligada)".to_string()
    } else {
        // `reqwest::Error::to_string()` nunca inclui o corpo da requisição nem
        // cabeçalhos de autenticação — só a URL e a causa de transporte.
        e.to_string()
    }
}

fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?)
}

#[derive(Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

pub async fn check_model(cfg: &Config, key: &str) -> (CheckStatus, String) {
    let Some(spec) = cfg.models.get(key) else {
        return (CheckStatus::Fail, format!("modelo \"{key}\" não existe mais em [models]"));
    };

    let resolved = match spec.resolve() {
        Ok(r) => r,
        Err(var) => {
            return (CheckStatus::Warn, format!("{key}: não configurado (falta a variável {var})"));
        }
    };

    let client = match http_client() {
        Ok(c) => c,
        Err(e) => return (CheckStatus::Fail, format!("{key}: não consegui montar o cliente HTTP: {e}")),
    };
    let api_key = spec.api_key_env.as_deref().and_then(env);

    let models_url = format!("{}/models", resolved.base_url.trim_end_matches('/'));
    let mut req = client.get(&models_url);
    if let Some(k) = &api_key {
        req = req.bearer_auth(k);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return (CheckStatus::Warn, format!("{key}: {models_url} — {}", network_msg(&e))),
    };

    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return (CheckStatus::Fail, format!("{key}: token rejeitado ({status}) em {models_url}"));
    }
    if !status.is_success() {
        return (CheckStatus::Warn, format!("{key}: {models_url} respondeu {status}"));
    }

    let available: Vec<String> = match resp.json::<ModelsResponse>().await {
        Ok(body) => body.data.into_iter().map(|m| m.id).collect(),
        Err(_) => Vec::new(),
    };
    if !available.is_empty() && !available.contains(&resolved.model) {
        let sample: Vec<&str> = available.iter().take(10).map(String::as_str).collect();
        return (
            CheckStatus::Fail,
            format!(
                "{key}: modelo \"{}\" não existe em {} (disponíveis: {})",
                resolved.model,
                resolved.base_url,
                sample.join(", ")
            ),
        );
    }

    if !looks_local(&resolved.base_url) && api_key.is_some() {
        return (
            CheckStatus::Ok,
            format!(
                "{key}: {} em {} (lista de modelos ok; chamada de geração não testada — custo)",
                resolved.model, resolved.base_url
            ),
        );
    }

    match generate_probe(&client, &resolved.base_url, &resolved.model, api_key.as_deref()).await {
        Ok(()) => (CheckStatus::Ok, format!("{key}: {} em {} respondeu", resolved.model, resolved.base_url)),
        Err(ProbeError::Http(s)) => (CheckStatus::Warn, format!("{key}: chamada de geração respondeu {s}")),
        Err(ProbeError::Network(msg)) => (CheckStatus::Warn, format!("{key}: chamada de geração — {msg}")),
    }
}

enum ProbeError {
    Http(reqwest::StatusCode),
    Network(String),
}

async fn generate_probe(
    client: &reqwest::Client,
    base_url: &str,
    model: &str,
    api_key: Option<&str>,
) -> Result<(), ProbeError> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Reply with just the word OK."}],
        "max_tokens": 5,
        "stream": false,
    });
    let mut req = client.post(&url).json(&body);
    if let Some(k) = api_key {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await.map_err(|e| ProbeError::Network(network_msg(&e)))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(ProbeError::Http(resp.status()))
    }
}

/// Primeiro modelo da cadeia do papel que existe em `[models]` — o mesmo que
/// `ModelRouter::candidates` tentaria primeiro, sem a parte de cooldown
/// (checagem estática, não depende de um roteador já montado).
fn first_model_of_role<'a>(cfg: &'a Config, role: &str) -> Option<&'a str> {
    cfg.roles
        .get(role)
        .into_iter()
        .flatten()
        .chain(cfg.roles.get("default").into_iter().flatten())
        .chain(cfg.llm.fallback.iter())
        .find(|k| cfg.models.contains_key(k.as_str()))
        .map(String::as_str)
}

pub async fn check_role(cfg: &Config, role: &str) -> (CheckStatus, String) {
    let Some(key) = first_model_of_role(cfg, role) else {
        return (CheckStatus::Fail, format!("papel \"{role}\": nenhum modelo da cadeia existe em [models]"));
    };
    let spec = cfg.models.get(key).expect("first_model_of_role só devolve chaves existentes");
    let resolved = match spec.resolve() {
        Ok(r) => r,
        Err(var) => {
            return (
                CheckStatus::Warn,
                format!("papel \"{role}\": modelo titular \"{key}\" não configurado (falta {var})"),
            );
        }
    };

    let api_key = spec.api_key_env.as_deref().and_then(env);
    if !looks_local(&resolved.base_url) && api_key.is_some() {
        return (
            CheckStatus::Ok,
            format!("papel \"{role}\": modelo \"{key}\" é remoto/pago — teste de formato não testado (custo)"),
        );
    }

    let client = match http_client() {
        Ok(c) => c,
        Err(e) => return (CheckStatus::Fail, format!("papel \"{role}\": não consegui montar o cliente HTTP: {e}")),
    };
    match format_probe(&client, &resolved.base_url, &resolved.model, api_key.as_deref()).await {
        Ok(true) => (CheckStatus::Ok, format!("papel \"{role}\": modelo \"{key}\" respondeu no formato esperado")),
        Ok(false) => (
            CheckStatus::Fail,
            format!("papel \"{role}\": modelo \"{key}\" respondeu, mas não no formato de ação esperado"),
        ),
        Err(ProbeError::Http(s)) => (CheckStatus::Warn, format!("papel \"{role}\": teste de formato respondeu {s}")),
        Err(ProbeError::Network(msg)) => (CheckStatus::Warn, format!("papel \"{role}\": teste de formato — {msg}")),
    }
}

/// Pede uma ação `file_write` conhecida e confere que `parse_actions` extrai
/// `path`/`content` dela — é o mesmo parser que o executor `builtin` usa de
/// verdade, então isto prova o formato ponta a ponta, não só "o HTTP respondeu".
async fn format_probe(
    client: &reqwest::Client,
    base_url: &str,
    model: &str,
    api_key: Option<&str>,
) -> Result<bool, ProbeError> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": "Create a file named hello.txt with the text oi, using the file_write action. \
                Respond with ONLY the <action> tag, nothing else."
        }],
        "max_tokens": 200,
        "stream": false,
    });
    let mut req = client.post(&url).json(&body);
    if let Some(k) = api_key {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await.map_err(|e| ProbeError::Network(network_msg(&e)))?;
    if !resp.status().is_success() {
        return Err(ProbeError::Http(resp.status()));
    }
    let raw: serde_json::Value = resp.json().await.map_err(|e| ProbeError::Network(e.to_string()))?;
    let text = raw["choices"][0]["message"]["content"].as_str().unwrap_or("");
    let actions = oreia_core::actions::parse_actions(text);
    Ok(actions.iter().any(|a| a.name == "file_write" && a.params.contains_key("path")))
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

    async fn fake_models_server(ids: &'static [&'static str]) -> String {
        let app = axum::Router::new().route(
            "/v1/models",
            axum::routing::get(move || async move {
                axum::Json(serde_json::json!({
                    "data": ids.iter().map(|id| serde_json::json!({"id": id})).collect::<Vec<_>>()
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1")
    }

    async fn fake_chat_server(reply: &'static str) -> String {
        let app = axum::Router::new()
            .route(
                "/v1/models",
                axum::routing::get(|| async { axum::Json(serde_json::json!({"data": [{"id": "qwen2.5-coder:14b"}]})) }),
            )
            .route(
                "/v1/chat/completions",
                axum::routing::post(move || async move {
                    axum::Json(serde_json::json!({
                        "choices": [{"message": {"role": "assistant", "content": reply}}],
                        "usage": {"prompt_tokens": 1, "completion_tokens": 1}
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1")
    }

    #[tokio::test]
    async fn model_in_the_list_is_ok() {
        let url = fake_chat_server("Reply: OK").await;
        let toml = SAMPLE.replace("http://ollama:11434/v1", &url);
        let c = cfg(&toml);
        let (status, msg) = check_model(&c, "qwen").await;
        assert_eq!(status, CheckStatus::Ok, "{msg}");
    }

    #[tokio::test]
    async fn model_missing_from_the_list_fails_and_lists_available_ones() {
        let url = fake_models_server(&["llama3.1:8b", "mixtral"]).await;
        let toml = SAMPLE.replace("http://ollama:11434/v1", &url);
        let c = cfg(&toml);
        let (status, msg) = check_model(&c, "qwen").await;
        assert_eq!(status, CheckStatus::Fail, "{msg}");
        assert!(msg.contains("llama3.1:8b") && msg.contains("mixtral"), "{msg}");
    }

    #[tokio::test]
    async fn connection_refused_is_a_warning_not_a_failure() {
        let toml = SAMPLE.replace("http://ollama:11434/v1", "http://127.0.0.1:1/v1");
        let c = cfg(&toml);
        let (status, msg) = check_model(&c, "qwen").await;
        assert_eq!(status, CheckStatus::Warn, "{msg}");
    }

    /// Modelo remoto E pago (host não-local com `api_key_env` presente): a checagem
    /// não deve fazer a chamada de geração (custo) — só o `GET /models`.
    #[tokio::test]
    async fn paid_remote_model_skips_the_generation_call() {
        std::env::set_var("OREIA_SECA_CHECK_TEST_KEY", "s3cr3t-sentinel-value");
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls2 = calls.clone();
        let app = axum::Router::new()
            .route(
                "/v1/models",
                axum::routing::get(|| async { axum::Json(serde_json::json!({"data": [{"id": "qwen2.5-coder:14b"}]})) }),
            )
            .route(
                "/v1/chat/completions",
                axum::routing::post(move || {
                    let calls = calls2.clone();
                    async move {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        axum::Json(serde_json::json!({"choices": [{"message": {"content": "ok"}}]}))
                    }
                }),
            );
        // Loopback IPv6: continua respondendo de verdade (preciso do servidor falso
        // alcançável para o GET /models, que a checagem sempre faz), mas
        // `looks_local` (string-based, não resolve DNS) só reconhece "localhost" e
        // prefixos de IPv4 privado — não bate aqui, então é tratado como remoto/pago.
        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let toml = SAMPLE.replace("http://ollama:11434/v1", &format!("http://{addr}/v1")).replace(
            "model = \"qwen2.5-coder:14b\"\ncontext = 32768",
            "model = \"qwen2.5-coder:14b\"\napi_key_env = \"OREIA_SECA_CHECK_TEST_KEY\"\ncontext = 32768",
        );
        let c = cfg(&toml);
        let (status, msg) = check_model(&c, "qwen").await;
        assert_eq!(status, CheckStatus::Ok, "{msg}");
        assert!(msg.contains("não testada"), "{msg}");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "não deveria ter chamado /chat/completions");
        assert!(!msg.contains("s3cr3t-sentinel-value"), "a chave nunca deveria aparecer na mensagem: {msg}");

        // Mesma garantia na saída de ponta a ponta: nem o JSON de `run()` nem o
        // `data/checks.jsonl` escrito por `check::log` podem vazar a chave.
        let results = crate::check::run(&c, &[crate::check::CheckTarget::Model("qwen".into())]).await;
        let json = serde_json::to_string(&results).unwrap();
        assert!(!json.contains("s3cr3t-sentinel-value"), "{json}");

        let log_path = std::env::temp_dir().join(format!("oreia-seca-check-leak-{}.jsonl", std::process::id()));
        tokio::fs::remove_file(&log_path).await.ok();
        crate::check::log::append(&log_path, &results).await.unwrap();
        let jsonl = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert!(!jsonl.contains("s3cr3t-sentinel-value"), "{jsonl}");
        tokio::fs::remove_file(&log_path).await.ok();
    }

    #[tokio::test]
    async fn role_format_probe_passes_with_a_valid_action_and_fails_on_plain_text() {
        let url_ok = fake_chat_server("<action name=\"file_write\"><param name=\"path\">hello.txt</param><param name=\"content\">oi</param></action>").await;
        let toml_ok = SAMPLE.replace("http://ollama:11434/v1", &url_ok);
        let c_ok = cfg(&toml_ok);
        let (status, msg) = check_role(&c_ok, "default").await;
        assert_eq!(status, CheckStatus::Ok, "{msg}");

        let url_bad = fake_chat_server("desculpe, não posso fazer isso agora").await;
        let toml_bad = SAMPLE.replace("http://ollama:11434/v1", &url_bad);
        let c_bad = cfg(&toml_bad);
        let (status, msg) = check_role(&c_bad, "default").await;
        assert_eq!(status, CheckStatus::Fail, "{msg}");
    }
}
