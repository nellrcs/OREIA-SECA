//! API HTTP: entrada REST (cron, scripts, CI) e webhook do Forgejo.
//!
//! - `GET  /health`
//! - `GET  /api/status`              (Bearer)
//! - `POST /api/message`             (Bearer) `{"text": "...", "userId": "...", "source": "rest"|"cron"}`
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
}
