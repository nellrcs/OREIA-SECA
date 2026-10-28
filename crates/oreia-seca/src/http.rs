//! API HTTP: entrada REST (cron, scripts, CI) e webhook do Forgejo.
//!
//! - `GET  /health`
//! - `GET  /api/status`              (Bearer)
//! - `POST /api/message`             (Bearer) `{"text": "...", "userId": "...", "source": "rest"|"cron"}`
//! - `POST /api/check`               (Bearer) `{"candidate": "<toml>"}` — ver `crate::check`
//! - `POST /api/mcp/*`               (Bearer, mesmo token de `/api/*`) — ver `crate::maker::mcp`
//!   e `docs/mcp-plano/02-endpoints-http-internos.md`. Um endpoint por ferramenta do
//!   servidor MCP; ele é o único chamador destas rotas, pela rede (loopback/mesma
//!   máquina), nunca lendo o estado direto.
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
use crate::maker::mcp::{CreateStoryReq, ListStoriesReq, ReleaseIssueReq, RepoIssueReq, ReviewPrReq, SubmitIssueReq};
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
        .route("/api/mcp/list_stories", post(mcp_list_stories))
        .route("/api/mcp/get_story", post(mcp_get_story))
        .route("/api/mcp/get_pr", post(mcp_get_pr))
        .route("/api/mcp/stats", post(mcp_stats))
        .route("/api/mcp/resource", post(mcp_resource))
        .route("/api/mcp/create_story", post(mcp_create_story))
        .route("/api/mcp/checkout_issue", post(mcp_checkout_issue))
        .route("/api/mcp/run_checks", post(mcp_run_checks))
        .route("/api/mcp/submit_issue", post(mcp_submit_issue))
        .route("/api/mcp/release_issue", post(mcp_release_issue))
        .route("/api/mcp/review_pr", post(mcp_review_pr))
        // Ferramenta de escrita restante: lógica chega na etapa 7 (ver docs/mcp-plano/).
        .route("/api/mcp/dispatch_issue", post(mcp_not_implemented))
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

/// Extrai e valida o Bearer ANTES de decodificar o corpo — um JSON inválido vindo de
/// quem não tem o token nunca deve chegar ao parser (nem gastar trabalho, nem vazar um
/// 422 em vez de 401). Único motivo de `/api/mcp/*` receber `Bytes` cru em vez de
/// `Json<T>` direto no handler.
fn authorized_json<T: serde::de::DeserializeOwned>(
    s: &HttpState,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<T, Box<Response>> {
    if !authorized(s, headers) {
        return Err(Box::new(unauthorized()));
    }
    serde_json::from_slice(body).map_err(|e| {
        Box::new((StatusCode::BAD_REQUEST, Json(json!({"error": format!("json inválido: {e}")}))).into_response())
    })
}

async fn mcp_list_stories(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: ListStoriesReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.list_stories(req).await {
        Ok(v) => Json(json!({ "stories": v })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn mcp_get_story(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: RepoIssueReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.get_story(req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn mcp_get_pr(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: RepoIssueReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.get_pr(req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct McpStatsReq {
    #[serde(default)]
    role: Option<String>,
}

async fn mcp_stats(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: McpStatsReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    Json(json!({ "stats": s.maker.mcp_stats(req.role).await })).into_response()
}

async fn mcp_create_story(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: CreateStoryReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.create_story(req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn mcp_checkout_issue(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: RepoIssueReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.checkout_issue(req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn mcp_run_checks(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: RepoIssueReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.run_checks(req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn mcp_submit_issue(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: SubmitIssueReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.submit_issue(req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn mcp_release_issue(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: ReleaseIssueReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.release_issue(req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn mcp_review_pr(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: ReviewPrReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.clone().review_pr(req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct ResourceReq {
    uri: String,
}

async fn mcp_resource(State(s): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
    let req: ResourceReq = match authorized_json(&s, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    match s.maker.read_resource(&req.uri).await {
        Ok(text) => Json(json!({ "text": text })).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// Ferramentas de escrita ainda não implementadas (etapas 4-7) — devolve 501, exigindo
/// o mesmo Bearer das demais (nenhuma rota de `/api/mcp/*` fica sem autenticação).
async fn mcp_not_implemented(State(s): State<Arc<HttpState>>, headers: HeaderMap, _body: Bytes) -> Response {
    if !authorized(&s, &headers) {
        return unauthorized();
    }
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "ainda não implementado (ver docs/mcp-plano/)"})))
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

    /// Nenhuma rota `/api/mcp/*` fica sem o mesmo Bearer das demais rotas de `/api/*` —
    /// nem as de leitura (já implementadas) nem os stubs 501 das de escrita.
    #[tokio::test]
    async fn mcp_routes_require_the_same_bearer_token() {
        let (app, _rx) = app(Some("k"), None).await;
        for path in [
            "/api/mcp/list_stories",
            "/api/mcp/get_story",
            "/api/mcp/get_pr",
            "/api/mcp/stats",
            "/api/mcp/create_story",
            "/api/mcp/checkout_issue",
        ] {
            let req = |auth: Option<&str>| {
                let mut b = Request::post(path).header("content-type", "application/json");
                if let Some(a) = auth {
                    b = b.header("authorization", a);
                }
                b.body(Body::from("{}")).unwrap()
            };
            assert_eq!(
                app.clone().oneshot(req(None)).await.unwrap().status(),
                StatusCode::UNAUTHORIZED,
                "{path} sem token"
            );
            assert_eq!(
                app.clone().oneshot(req(Some("Bearer errado"))).await.unwrap().status(),
                StatusCode::UNAUTHORIZED,
                "{path} com token errado"
            );
        }
    }

    #[tokio::test]
    async fn mcp_write_tools_are_not_implemented_yet() {
        let (app, _rx) = app(Some("k"), None).await;
        let req = Request::post("/api/mcp/dispatch_issue")
            .header("content-type", "application/json")
            .header("authorization", "Bearer k")
            .body(Body::from("{}"))
            .unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn mcp_list_stories_without_forgejo_configured_is_a_clear_error_not_a_crash() {
        let (app, _rx) = app(Some("k"), None).await;
        let req = Request::post("/api/mcp/list_stories")
            .header("content-type", "application/json")
            .header("authorization", "Bearer k")
            .body(Body::from(r#"{"repo":null,"status":null}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed["error"].as_str().unwrap().contains("Forgejo"), "{parsed}");
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
