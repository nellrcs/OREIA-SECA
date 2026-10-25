use std::time::Duration;

use async_trait::async_trait;
use oreia_core::{ChatMessage, Role};
use serde::Deserialize;
use serde_json::json;

use crate::{Completion, GenerateOpts, LlmError, LlmProvider, ModelSpec, ResolvedModel, Usage};

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Cliente para a API nativa da Anthropic (`/v1/messages`) — formato próprio,
/// diferente do `/v1/chat/completions` que [`crate::OpenAiCompat`] fala:
/// cabeçalho `x-api-key` em vez de `Authorization: Bearer`, `system` como
/// campo do corpo em vez de mensagem na lista, `max_tokens` obrigatório, e
/// resposta em blocos `content: [{type, text}]` em vez de `choices`.
pub struct AnthropicNative {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    key: String,
    temperature: Option<f32>,
    /// `max_tokens` é obrigatório na Anthropic; usa `reserve_output` do
    /// `[models.*]` quando a chamada não pede um valor explícito.
    default_max_tokens: u32,
}

impl AnthropicNative {
    /// `resolved` já traz `base_url`/`model` prontos (ver `ModelSpec::resolve`).
    pub fn new(key: &str, spec: &ModelSpec, resolved: &ResolvedModel) -> Result<Self, LlmError> {
        if spec.provider != "anthropic" {
            return Err(LlmError::Decode(format!("provedor \"{}\" não suportado (use anthropic)", spec.provider)));
        }
        let api_key = spec
            .api_key_env
            .as_deref()
            .and_then(|name| std::env::var(name).ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                LlmError::Decode(format!("[models.{key}] provider \"anthropic\" precisa de api_key_env com a chave"))
            })?;

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| LlmError::Connection(e.to_string()))?;

        Ok(Self {
            http,
            base_url: resolved.base_url.trim_end_matches('/').to_string(),
            api_key,
            model: resolved.model.clone(),
            key: key.to_string(),
            temperature: spec.temperature,
            default_max_tokens: spec.reserve_output,
        })
    }
}

#[derive(Deserialize, Default)]
struct AnthropicResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize, Default)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

fn map_reqwest(e: reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::Timeout(Duration::ZERO)
    } else if e.is_connect() || e.is_request() {
        LlmError::Connection(e.to_string())
    } else {
        LlmError::Decode(e.to_string())
    }
}

/// Monta o corpo da requisição (separado para testar sem rede). Mensagens com
/// `Role::System` nunca devem chegar aqui na prática (ver `GenerateOpts::system`),
/// mas, se chegarem, entram no `system` do corpo em vez de quebrar a chamada —
/// a Anthropic rejeita `role: "system"` dentro de `messages`.
pub(crate) fn build_body(
    model: &str,
    messages: &[ChatMessage],
    opts: &GenerateOpts,
    temperature: Option<f32>,
    default_max_tokens: u32,
) -> serde_json::Value {
    let mut system_parts = Vec::new();
    if let Some(sys) = &opts.system {
        system_parts.push(sys.clone());
    }
    let mut msgs = Vec::with_capacity(messages.len());
    for m in messages {
        match m.role {
            Role::System => system_parts.push(m.content.clone()),
            Role::User => msgs.push(json!({"role": "user", "content": m.content})),
            Role::Assistant => msgs.push(json!({"role": "assistant", "content": m.content})),
        }
    }

    let mut body = json!({
        "model": model,
        "max_tokens": opts.max_tokens.unwrap_or(default_max_tokens),
        "messages": msgs,
    });
    if !system_parts.is_empty() {
        body["system"] = json!(system_parts.join("\n\n"));
    }
    if let Some(t) = opts.temperature.or(temperature) {
        body["temperature"] = json!(t);
    }
    body
}

#[async_trait]
impl LlmProvider for AnthropicNative {
    fn model_name(&self) -> &str {
        &self.model
    }

    async fn generate(&self, messages: &[ChatMessage], opts: &GenerateOpts) -> Result<Completion, LlmError> {
        let body = build_body(&self.model, messages, opts, self.temperature, self.default_max_tokens);
        if crate::llm_debug_enabled() {
            tracing::debug!(model = %self.key, request = %body, "requisição ao LLM (OREIA_SECA_LLM_DEBUG)");
        }
        let url = format!("{}/messages", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest)?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Http { status: status.as_u16(), body: body.chars().take(500).collect() });
        }

        let raw = resp.text().await.map_err(|e| LlmError::Decode(e.to_string()))?;
        if crate::llm_debug_enabled() {
            tracing::debug!(model = %self.key, response = %raw, "resposta crua do LLM (OREIA_SECA_LLM_DEBUG)");
        }
        let parsed: AnthropicResponse = serde_json::from_str(&raw).map_err(|e| LlmError::Decode(e.to_string()))?;
        let text: String = parsed.content.into_iter().filter(|b| b.kind == "text").filter_map(|b| b.text).collect();
        if text.is_empty() {
            return Err(LlmError::Decode("resposta sem bloco de texto em content".into()));
        }

        let usage = parsed
            .usage
            .map(|u| Usage { prompt_tokens: u.input_tokens, completion_tokens: u.output_tokens })
            .unwrap_or_default();
        Ok(Completion { text, usage, model: self.key.clone() })
    }

    async fn health(&self) -> bool {
        let req = self
            .http
            .get(format!("{}/models", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .timeout(Duration::from_secs(3));
        matches!(req.send().await, Ok(r) if r.status().is_success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_for(key_env: &str) -> ModelSpec {
        let mut spec = ModelSpec::local("https://api.anthropic.com/v1", "claude-sonnet-5-5");
        spec.provider = "anthropic".into();
        spec.api_key_env = Some(key_env.into());
        spec
    }

    #[test]
    fn body_puts_system_before_messages_and_sets_max_tokens() {
        let opts = GenerateOpts::system("seja breve");
        let b = build_body("claude-sonnet-5-5", &[ChatMessage::user("oi")], &opts, Some(0.2), 2048);
        assert_eq!(b["model"], "claude-sonnet-5-5");
        assert_eq!(b["system"], "seja breve");
        assert_eq!(b["messages"][0]["role"], "user");
        assert_eq!(b["messages"][0]["content"], "oi");
        assert_eq!(b["max_tokens"], 2048);
        assert!((b["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert!(b.get("stream").is_none());
    }

    #[test]
    fn body_has_no_system_field_when_absent() {
        let b = build_body("claude-sonnet-4-5", &[ChatMessage::user("oi")], &GenerateOpts::default(), None, 4096);
        assert!(b.get("system").is_none());
        assert_eq!(b["max_tokens"], 4096);
    }

    #[test]
    fn explicit_max_tokens_wins_over_default() {
        let opts = GenerateOpts::default().max_tokens(777);
        let b = build_body("claude-sonnet-4-5", &[], &opts, None, 4096);
        assert_eq!(b["max_tokens"], 777);
    }

    #[test]
    fn rejects_unknown_provider() {
        let mut spec = ModelSpec::local("https://api.anthropic.com/v1", "claude-sonnet-5-5");
        spec.provider = "openai_compat".into();
        let resolved = spec.resolve().unwrap();
        assert!(AnthropicNative::new("k", &spec, &resolved).is_err());
    }

    #[test]
    fn requires_api_key_env() {
        let mut spec = spec_for("OREIA_SECA_ANTHROPIC_TEST_MISSING_KEY");
        spec.api_key_env = None;
        let resolved = spec.resolve().unwrap();
        assert!(AnthropicNative::new("k", &spec, &resolved).is_err());
    }
}
