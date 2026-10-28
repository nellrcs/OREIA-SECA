//! API HTTP: entrada REST (cron, scripts, CI) e webhook do Forgejo.
//!
//! - `GET  /health`
//! - `GET  /api/status`              (Bearer)
//! - `POST /api/message`             (Bearer) `{"text": "...", "userId": "...", "source": "rest"|"cron"}`
//! - `POST /api/check`               (Bearer) `{"candidate": "<toml>"}` — ver `crate::check`
//! - `POST /webhooks/forgejo`        (assinatura HMAC do Forgejo)

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use oreia_core::{Incoming, Source};
use oreia_forgejo::webhook;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::check;
use crate::config::Config;
use crate::maker::Maker;

pub struct HttpState {
    pub maker: Arc<Maker>,
    pub tx: mpsc::Sender<Incoming>,
    pub api_key: Option<String>,
    pub webhook_secret: Option<String>,
}

pub fn router(state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/status", get(api_status))
        .route("/api/message", post(api_message))
        .route("/api/check", post(api_check))
        .route("/webhooks/forgejo", post(forgejo_webhook))
        .with_state(state)
}

fn authorized(state: &HttpState, headers: &HeaderMap) -> bool {
    let Some(key) = &state.api_key else { return true };
    let Some(given) =
        headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    // Comparação em tempo constante
    given.len() == key.len() && given.bytes().zip(key.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": "token inválido"}))).into_response()
}

async fn api_status(State(s): State<Arc<HttpState>>, headers: HeaderMap) -> Response {
    if !authorized(&s, &headers) {
        return unauthorized();
    }
    Json(s.maker.status_json()).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiMessage {
    text: String,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

async fn api_message(State(s): State<Arc<HttpState>>, headers: HeaderMap, Json(body): Json<ApiMessage>) -> Response {
    if !authorized(&s, &headers) {
        return unauthorized();
    }
    if body.text.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "text vazio"}))).into_response();
    }
    let source = match body.source.as_deref() {
        None | Some("rest") => Source::Rest,
        Some("cron") => Source::Cron,
        Some(other) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("source inválido: {other} (use rest ou cron)")})),
            )
                .into_response()
        }
    };
    let user = body.user_id.unwrap_or_else(|| "api".into());
    let needs_approval = s.maker.cfg.needs_approval(source);

    if s.tx.send(Incoming::new(source, user, body.text)).await.is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "orquestrador encerrando"}))).into_response();
    }
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "status": "accepted",
            "needsApproval": needs_approval,
            "message": if needs_approval {
                "Recebido. Tarefas desta fonte aguardam /approve nos canais humanos."
            } else {
                "Recebido."
            }
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct ApiCheck {
    candidate: String,
}

/// Valida um `oreia-seca.toml` candidato e roda as checagens de rede/processo só do
/// que mudou em relação à config atualmente em memória — nunca a substitui (ver
/// `crate::check`). Usado pelo painel de Configuração do px0 antes de gravar o arquivo.
async fn api_check(State(s): State<Arc<HttpState>>, headers: HeaderMap, Json(body): Json<ApiCheck>) -> Response {
    if !authorized(&s, &headers) {
        return unauthorized();
    }
    let candidate: Config = match toml::from_str(&body.candidate) {
        Ok(c) => c,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": format!("toml inválido: {e}")}))).into_response()
        }
    };
    if let Err(e) = candidate.validate() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": format!("config inválida: {e:#}")}))).into_response();
    }

    let targets = check::diff(&s.maker.cfg, &candidate);
    let results = check::run(&candidate, &targets).await;

    let log_path = s.maker.cfg.paths.data.join("checks.jsonl");
    if let Err(e) = check::log::append(&log_path, &results).await {
        tracing::warn!(error = %e, path = %log_path.display(), "falha ao gravar o histórico de checagens");
    }

    Json(json!({ "results": results })).into_response()
}

fn header<'a>(headers: &'a HeaderMap, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|n| headers.get(*n).and_then(|v| v.to_str().ok()))
}

async fn forgejo_webhook(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(secret) = &s.webhook_secret else {
        tracing::warn!("webhook recusado: segredo não configurado");
        return (StatusCode::FORBIDDEN, "segredo do webhook não configurado").into_response();
    };
    let Some(sig) = header(&headers, webhook::SIGNATURE_HEADERS) else {
        return (StatusCode::UNAUTHORIZED, "assinatura ausente").into_response();
    };
    if !webhook::verify_signature(secret, &body, sig) {
        tracing::warn!("webhook com assinatura inválida");
        return (StatusCode::UNAUTHORIZED, "assinatura inválida").into_response();
    }
    let Some(event_name) = header(&headers, webhook::EVENT_HEADERS) else {
        return (StatusCode::BAD_REQUEST, "evento ausente").into_response();
    };

    match webhook::parse_event(event_name, &body) {
        Ok(event) => {
            let maker = s.maker.clone();
            tokio::spawn(async move { maker.handle_forgejo(event).await });
            (StatusCode::ACCEPTED, "ok").into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "payload de webhook inválido");
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maker::tests::test_maker;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn app(api_key: Option<&str>, secret: Option<&str>) -> (Router, mpsc::Receiver<Incoming>) {
        let (maker, _rec, _dir) = test_maker(vec![]).await;
        let (tx, rx) = mpsc::channel(8);
        let state = Arc::new(HttpState {
            maker,
            tx,
            api_key: api_key.map(String::from),
            webhook_secret: secret.map(String::from),
        });
        (router(state), rx)
    }

    #[tokio::test]
    async fn message_requires_token() {
        let (app, mut rx) = app(Some("k"), None).await;
        let req = |auth: Option<&str>| {
            let mut b = Request::post("/api/message").header("content-type", "application/json");
            if let Some(a) = auth {
                b = b.header("authorization", a);
            }
            b.body(Body::from(r#"{"text":"crie um script","userId":"cron1","source":"cron"}"#)).unwrap()
        };
        assert_eq!(app.clone().oneshot(req(None)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(app.clone().oneshot(req(Some("Bearer x"))).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(app.clone().oneshot(req(Some("Bearer k"))).await.unwrap().status(), StatusCode::ACCEPTED);
        let m = rx.recv().await.unwrap();
        assert_eq!((m.source, m.user_id.as_str()), (Source::Cron, "cron1"));
    }

    #[tokio::test]
    async fn webhook_checks_signature() {
        let (app, _rx) = app(None, Some("s3cr3t")).await;
        let body = r#"{"ref":"x"}"#;
        let send = |sig: &str| {
            Request::post("/webhooks/forgejo")
                .header("x-forgejo-event", "push")
                .header("x-forgejo-signature", sig)
                .body(Body::from(body))
                .unwrap()
        };
        assert_eq!(app.clone().oneshot(send("00")).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        let good = webhook::sign("s3cr3t", body.as_bytes());
        assert_eq!(app.clone().oneshot(send(&good)).await.unwrap().status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn webhook_refuses_without_secret() {
        let (app, _rx) = app(None, None).await;
        let r = Request::post("/webhooks/forgejo").body(Body::empty()).unwrap();
        assert_eq!(app.oneshot(r).await.unwrap().status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn health_is_open() {
        let (app, _rx) = app(Some("k"), None).await;
        let r = Request::get("/health").body(Body::empty()).unwrap();
        assert_eq!(app.oneshot(r).await.unwrap().status(), StatusCode::OK);
    }

    /// Config minimamente válida, com `[paths]` apontando pra um diretório de
    /// temp isolado — sem isso, a checagem do alvo `Paths` (que `diff` sempre
    /// detecta, já que o candidato não herda o `[paths]` de teste do Maker)
    /// criaria `data/`/`workspace`/`skills` relativos ao cwd de `cargo test`.
    fn sample_toml() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "oreia-seca-http-check-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        format!(
            r#"
[paths]
data = "{d}/data"
workspace = "{d}/workspace"
skills = "{d}/skills"

[models.fake]
base_url = "http://localhost:1/v1"
model = "fake"

[roles]
default = ["fake"]
"#,
            d = dir.display().to_string().replace('\\', "\\\\")
        )
    }

    #[tokio::test]
    async fn check_requires_token() {
        let (app, _rx) = app(Some("k"), None).await;
        let candidate = sample_toml();
        let req = |auth: Option<&str>| {
            let mut b = Request::post("/api/check").header("content-type", "application/json");
            if let Some(a) = auth {
                b = b.header("authorization", a);
            }
            b.body(Body::from(serde_json::json!({ "candidate": candidate }).to_string())).unwrap()
        };
        assert_eq!(app.clone().oneshot(req(None)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(app.clone().oneshot(req(Some("Bearer x"))).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(app.oneshot(req(Some("Bearer k"))).await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn check_rejects_invalid_toml_without_touching_the_running_config() {
        let (maker, _rec, _dir) = crate::maker::tests::test_maker(vec![]).await;
        let (tx, _rx) = mpsc::channel(8);
        let state = Arc::new(HttpState { maker: maker.clone(), tx, api_key: Some("k".into()), webhook_secret: None });
        let app = router(state);

        let before = maker.status_json();

        let bad = Request::post("/api/check")
            .header("content-type", "application/json")
            .header("authorization", "Bearer k")
            .body(Body::from(serde_json::json!({ "candidate": "isso não é toml válido [[[" }).to_string()))
            .unwrap();
        assert_eq!(app.clone().oneshot(bad).await.unwrap().status(), StatusCode::BAD_REQUEST);

        let unknown_model = Request::post("/api/check")
            .header("content-type", "application/json")
            .header("authorization", "Bearer k")
            .body(Body::from(serde_json::json!({ "candidate": "[roles]\ndefault = [\"nao-existe\"]" }).to_string()))
            .unwrap();
        assert_eq!(app.clone().oneshot(unknown_model).await.unwrap().status(), StatusCode::BAD_REQUEST);

        // Nenhuma das duas tentativas, mesmo sendo inválidas, altera a config em uso pelo Maker.
        assert_eq!(serde_json::to_string(&before).unwrap(), serde_json::to_string(&maker.status_json()).unwrap());
    }

    #[tokio::test]
    async fn check_runs_against_the_candidate_and_returns_its_results() {
        let (maker, _rec, _dir) = crate::maker::tests::test_maker(vec![]).await;
        let (tx, _rx) = mpsc::channel(8);
        let state = Arc::new(HttpState { maker, tx, api_key: None, webhook_secret: None });
        let app = router(state);

        let req = Request::post("/api/check")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({ "candidate": sample_toml() }).to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed["results"].is_array(), "{parsed}");
    }
}
