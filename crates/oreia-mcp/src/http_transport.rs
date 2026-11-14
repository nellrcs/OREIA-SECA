//! Transporte HTTP streamable do servidor MCP — opcional, além do stdio (ver
//! `docs/mcp-plano/08-http-opcional.md`). Mesmo `OreiaMcp`, mesma lógica em
//! `oreia-seca::maker::mcp`; só muda como o cliente MCP se conecta.
//!
//! Duas checagens de segurança, nenhuma delegada ao `rmcp`: Bearer `MCP_TOKEN`
//! (esta borda é externa — cliente ↔ processo MCP — diferente do Bearer interno
//! de `/api/mcp/*`, que é o processo MCP falando com o `oreia-seca`) e `Origin`
//! fora de uma lista simples de hosts locais.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use crate::OreiaMcp;

struct AuthState {
    token: String,
}

fn origin_allowed(origin: &str) -> bool {
    origin.contains("://localhost") || origin.contains("://127.0.0.1") || origin.contains("://[::1]")
}

async fn check_auth_and_origin(State(state): State<Arc<AuthState>>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let authorized = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|given| {
            given.len() == state.token.len()
                && given.bytes().zip(state.token.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
        })
        .unwrap_or(false);
    if !authorized {
        return (StatusCode::UNAUTHORIZED, "token inválido").into_response();
    }
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        if !origin_allowed(origin) {
            return (StatusCode::FORBIDDEN, "origin não permitida").into_response();
        }
    }
    next.run(req).await
}

fn app(base_url: String, api_key: Option<String>, mcp_token: String, allowed_hosts: Vec<String>) -> Router {
    // `rmcp` tem sua própria checagem de `Host` contra DNS rebinding, separada
    // da nossa de `Origin` abaixo — sem isto aqui, um `bind` não-loopback
    // (ver docs/mcp.md#transporte-http-opcional) rejeita com 403 qualquer
    // cliente que não chegue como "localhost"/"127.0.0.1"/"::1". `"*"` desliga
    // a checagem (equivalente a `disable_allowed_hosts`) — já tem o Bearer
    // `MCP_TOKEN` protegendo, então liberar qualquer Host é uma opção
    // explícita do usuário, não insegura por si só.
    let http_config = if allowed_hosts.iter().any(|h| h == "*") {
        StreamableHttpServerConfig::default().disable_allowed_hosts()
    } else {
        StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts)
    };
    let service = StreamableHttpService::new(
        move || Ok(OreiaMcp::new(base_url.clone(), api_key.clone())),
        LocalSessionManager::default().into(),
        http_config,
    );
    let auth_state = Arc::new(AuthState { token: mcp_token });
    Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(auth_state, check_auth_and_origin))
}

/// Sobe o transporte HTTP streamable em `bind` (ver `[mcp].bind`, padrão
/// `127.0.0.1:3121` — nunca `0.0.0.0` por padrão). Chamado só quando `[mcp].http =
/// true` e `MCP_TOKEN` está definido; sem token, quem chama (`oreia-seca mcp`) nem
/// tenta subir isto.
pub async fn serve_http(
    base_url: String,
    api_key: Option<String>,
    bind: String,
    mcp_token: String,
    allowed_hosts: Vec<String>,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "transporte HTTP do MCP escutando em /mcp");
    axum::serve(listener, app(base_url, api_key, mcp_token, allowed_hosts)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn spawn(mcp_token: &str) -> String {
        spawn_with_hosts(mcp_token, vec!["localhost".into(), "127.0.0.1".into(), "::1".into()]).await
    }

    async fn spawn_with_hosts(mcp_token: &str, allowed_hosts: Vec<String>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = app("http://localhost:1".to_string(), None, mcp_token.to_string(), allowed_hosts);
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn requires_the_bearer_token() {
        let base = spawn("secret").await;
        let client = reqwest::Client::new();

        let resp = client.post(format!("{base}/mcp")).send().await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

        let resp = client.post(format!("{base}/mcp")).bearer_auth("errado").send().await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_a_disallowed_origin_even_with_a_valid_token() {
        let base = spawn("secret").await;
        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{base}/mcp"))
            .bearer_auth("secret")
            .header("origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn valid_token_and_local_origin_reach_the_mcp_service() {
        let base = spawn("secret").await;
        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{base}/mcp"))
            .bearer_auth("secret")
            .header("origin", "http://localhost:5173")
            .send()
            .await
            .unwrap();
        // Não é um request MCP válido (sem corpo JSON-RPC), mas já passou da nossa
        // autenticação/Origin — o que este teste prova. O próprio rmcp decide o
        // status final daqui pra frente.
        assert_ne!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert_ne!(resp.status(), reqwest::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_a_host_outside_allowed_hosts_even_with_a_valid_token() {
        let base = spawn("secret").await; // só localhost/127.0.0.1/::1
        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{base}/mcp"))
            .bearer_auth("secret")
            .header(reqwest::header::HOST, "192.168.2.101:3121")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn wildcard_allowed_hosts_accepts_any_host() {
        let base = spawn_with_hosts("secret", vec!["*".into()]).await;
        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{base}/mcp"))
            .bearer_auth("secret")
            .header(reqwest::header::HOST, "192.168.2.101:3121")
            .send()
            .await
            .unwrap();
        assert_ne!(resp.status(), reqwest::StatusCode::FORBIDDEN);
    }
}
