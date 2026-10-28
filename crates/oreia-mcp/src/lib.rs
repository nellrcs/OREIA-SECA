//! Servidor MCP do OREIA (P.O. por IA + desenvolvedor sob demanda).
//!
//! Cada ferramenta aqui é um cliente HTTP fino: monta o pedido, chama
//! `{base_url}/api/mcp/<ferramenta>` no `oreia-seca` JÁ RODANDO (mesmo Bearer de
//! `/api/*`) e devolve a resposta. Nenhuma lógica de negócio mora neste crate —
//! ela vive em `oreia-seca::maker::mcp`, testável sem subir o protocolo MCP.
//! Ver `docs/mcp-plano/03-crate-mcp-stdio-leitura.md`.

mod prompts;

use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{prompt, prompt_handler, prompt_router, tool, tool_handler, tool_router};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListStoriesArgs {
    /// `dono/repo`. Ausente = todos os repositórios de `[forgejo].repos`.
    #[serde(default)]
    pub repo: Option<String>,
    /// `all` (padrão) | `available` | `in_progress` | `done`.
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RepoIssueArgs {
    /// `dono/repo`.
    pub repo: String,
    /// Número da issue (ou do PR, em `get_pr`).
    pub n: u64,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct GetStatsArgs {
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Clone)]
pub struct OreiaMcp {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
    // Lidos pelo código gerado por #[tool_handler]/#[prompt_handler] (despacha
    // tools/prompts), não por código nosso — o checador de dead-code não enxerga
    // esse uso. Mesmo padrão dos exemplos oficiais do rmcp.
    #[allow(dead_code)]
    tool_router: ToolRouter<OreiaMcp>,
    #[allow(dead_code)]
    prompt_router: PromptRouter<OreiaMcp>,
}

impl OreiaMcp {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            base_url,
            api_key,
            http: reqwest::Client::new(),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }

    /// Chama `/api/mcp/<tool>` e devolve o corpo (JSON ou texto de erro) como o
    /// texto da ferramenta — nenhum token entra na resposta (só o corpo que o
    /// `oreia-seca` já filtra).
    async fn call(&self, tool: &str, body: serde_json::Value) -> Result<CallToolResult, McpError> {
        let url = format!("{}/api/mcp/{tool}", self.base_url.trim_end_matches('/'));
        let mut req = self.http.post(url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| McpError::internal_error(format!("falha ao chamar o oreia-seca: {e}"), None))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(McpError::internal_error(format!("oreia-seca respondeu {status}: {text}"), None));
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    async fn resource_text(&self, uri: &str) -> anyhow::Result<String> {
        let url = format!("{}/api/mcp/resource", self.base_url.trim_end_matches('/'));
        let mut req = self.http.post(url).json(&serde_json::json!({ "uri": uri }));
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("{}", body.get("error").and_then(|v| v.as_str()).unwrap_or("falha ao ler o recurso"));
        }
        Ok(body.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string())
    }
}

#[tool_router]
impl OreiaMcp {
    #[tool(description = "Lista histórias (issues) e seu estado: livre, em andamento (agente ou MCP) ou concluída")]
    async fn list_stories(&self, Parameters(args): Parameters<ListStoriesArgs>) -> Result<CallToolResult, McpError> {
        self.call("list_stories", serde_json::to_value(args).unwrap_or_default()).await
    }

    #[tool(description = "Issue completa: texto, dono atual, tarefa, fases, degraus de escalada tentados e PR")]
    async fn get_story(&self, Parameters(args): Parameters<RepoIssueArgs>) -> Result<CallToolResult, McpError> {
        self.call("get_story", serde_json::to_value(args).unwrap_or_default()).await
    }

    #[tool(description = "PR: título, autor, arquivos alterados, diff (truncado), notas de revisão e escalada")]
    async fn get_pr(&self, Parameters(args): Parameters<RepoIssueArgs>) -> Result<CallToolResult, McpError> {
        self.call("get_pr", serde_json::to_value(args).unwrap_or_default()).await
    }

    #[tool(description = "Estatísticas de escalada por papel: degraus, custo total e médio (mesmo do /stats)")]
    async fn get_stats(&self, Parameters(args): Parameters<GetStatsArgs>) -> Result<CallToolResult, McpError> {
        self.call("stats", serde_json::to_value(args).unwrap_or_default()).await
    }
}

#[prompt_router]
impl OreiaMcp {
    #[prompt(name = "po_guidelines", description = "Como escrever uma boa história (issue) para os agentes")]
    async fn po_guidelines(&self, _ctx: RequestContext<RoleServer>) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User, prompts::PO_GUIDELINES)])
    }
}

#[tool_handler]
#[prompt_handler]
impl ServerHandler for OreiaMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_prompts().enable_resources().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "OREIA: P.O. por IA (crie/revise histórias com create_story/list_stories/get_story/get_pr/review_pr) \
                 e desenvolvedor sob demanda (checkout_issue/run_checks/submit_issue/release_issue — assuma uma \
                 issue livre ou que um agente não resolveu e codifique com o SEU modelo; o OREIA cuida de git, \
                 checks e PR). Veja o prompt po_guidelines antes de criar uma história.",
            )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![Resource::new("oreia://docs/roadmap", "Roadmap do OREIA".to_string())],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri.clone();
        let text = self.resource_text(&uri).await.map_err(|e| {
            McpError::resource_not_found(
                "resource_not_found",
                Some(serde_json::json!({"uri": uri, "error": e.to_string()})),
            )
        })?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(text, uri)]).into())
    }
}

/// Sobe o servidor no transporte stdio e espera até o cliente encerrar a conexão.
/// Chamado por `oreia-seca mcp` (ver `crates/oreia-seca/src/main.rs`) — a stdout
/// É o protocolo MCP, então quem chama isto NUNCA pode ter escrito nada nela antes
/// (logs vão pra stderr).
pub async fn serve_stdio(base_url: String, api_key: Option<String>) -> anyhow::Result<()> {
    let service = OreiaMcp::new(base_url, api_key).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Item 9 dos testes obrigatórios (ver `docs/mcp-plano/03-crate-mcp-stdio-leitura.md`):
    /// `tools/list` devolve as ferramentas esperadas com schemas válidos. Testado via
    /// introspecção direta do `ToolRouter` (`list_all`, sem precisar de um
    /// `RequestContext` nem de transporte nenhum) — o protocolo/serialização em si é
    /// responsabilidade do próprio `rmcp`, não deste crate; o que É nosso é garantir que
    /// as quatro ferramentas estão registradas com um schema de entrada de verdade.
    #[test]
    fn tool_router_lists_the_four_read_tools_with_valid_schemas() {
        let tools = OreiaMcp::tool_router().list_all();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        for expected in ["list_stories", "get_story", "get_pr", "get_stats"] {
            assert!(names.contains(&expected), "{names:?}");
        }
        for t in &tools {
            assert!(t.input_schema.contains_key("properties") || t.input_schema.contains_key("type"), "{t:?}");
        }
    }

    #[test]
    fn prompt_router_lists_po_guidelines() {
        let prompts = OreiaMcp::prompt_router().list_all();
        assert!(prompts.iter().any(|p| p.name == "po_guidelines"), "{prompts:?}");
    }
}
