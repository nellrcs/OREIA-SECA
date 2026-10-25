use std::time::Duration;

use async_trait::async_trait;
use oreia_core::ChatMessage;
use serde::Deserialize;
use serde_json::json;

use crate::{Completion, GenerateOpts, LlmError, LlmProvider, ModelSpec, ResolvedModel, Usage};

/// Cliente para qualquer servidor com `/chat/completions` no formato OpenAI.
pub struct OpenAiCompat {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    key: String,
    temperature: Option<f32>,
}

impl OpenAiCompat {
    /// `resolved` já traz `base_url`/`model` prontos (ver `ModelSpec::resolve`) —
    /// esta função só cuida do provedor, da chave e do cliente HTTP.
    pub fn new(key: &str, spec: &ModelSpec, resolved: &ResolvedModel) -> Result<Self, LlmError> {
        if spec.provider != "openai_compat" {
            return Err(LlmError::Decode(format!("provedor \"{}\" não suportado (use openai_compat)", spec.provider)));
        }
        let api_key =
            spec.api_key_env.as_deref().and_then(|name| std::env::var(name).ok()).filter(|k| !k.trim().is_empty());

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
        })
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    /// Alguns modelos/provedores (visto com deepseek via OpenRouter) respondem
    /// no formato nativo de tool calling mesmo sem o chamador declarar `tools`
    /// no corpo — `content` vem `null` e a "ação" vira isto em vez de texto.
    /// Como o OREIA-SECA só fala a convenção `<action>` em texto, traduzimos
    /// de volta aqui (ver `tool_calls_to_actions`) em vez de cada chamador
    /// precisar lidar com os dois formatos.
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize)]
struct ToolCall {
    function: FunctionCall,
}

#[derive(Deserialize)]
struct FunctionCall {
    name: String,
    #[serde(default)]
    arguments: String,
}

/// `<action name="{name}">{arguments}</action>` por chamada — o parser de ações
/// já aceita um objeto JSON como corpo (preenche os parâmetros a partir dele),
/// então basta repassar `arguments` (já é esse JSON) sem reformatar nada.
fn tool_calls_to_actions(calls: &[ToolCall]) -> String {
    calls
        .iter()
        .map(|c| format!("<action name=\"{}\">{}</action>", c.function.name, c.function.arguments))
        .collect::<Vec<_>>()
        .join("\n")
}

fn message_text(message: ChoiceMessage) -> Option<String> {
    if let Some(c) = message.content {
        if !c.trim().is_empty() {
            return Some(c);
        }
    }
    match message.tool_calls {
        Some(calls) if !calls.is_empty() => Some(tool_calls_to_actions(&calls)),
        _ => None,
    }
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

/// Monta o corpo da requisição (separado para testar sem rede).
pub(crate) fn build_body(
    model: &str,
    messages: &[ChatMessage],
    opts: &GenerateOpts,
    temp: Option<f32>,
) -> serde_json::Value {
    let mut msgs = Vec::with_capacity(messages.len() + 1);
    if let Some(sys) = &opts.system {
        msgs.push(json!({"role": "system", "content": sys}));
    }
    for m in messages {
        msgs.push(serde_json::to_value(m).expect("mensagem serializável"));
    }
    let mut body = json!({"model": model, "messages": msgs, "stream": false});
    if let Some(n) = opts.max_tokens {
        body["max_tokens"] = json!(n);
    }
    if let Some(t) = opts.temperature.or(temp) {
        body["temperature"] = json!(t);
    }
    body
}

#[async_trait]
impl LlmProvider for OpenAiCompat {
    fn model_name(&self) -> &str {
        &self.model
    }

    async fn generate(&self, messages: &[ChatMessage], opts: &GenerateOpts) -> Result<Completion, LlmError> {
        let body = build_body(&self.model, messages, opts, self.temperature);
        if crate::llm_debug_enabled() {
            tracing::debug!(model = %self.key, request = %body, "requisição ao LLM (OREIA_SECA_LLM_DEBUG)");
        }
        let url = format!("{}/chat/completions", self.base_url);
        let resp = self.auth(self.http.post(&url)).json(&body).send().await.map_err(map_reqwest)?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Http { status: status.as_u16(), body: body.chars().take(500).collect() });
        }

        let raw = resp.text().await.map_err(|e| LlmError::Decode(e.to_string()))?;
        if crate::llm_debug_enabled() {
            tracing::debug!(model = %self.key, response = %raw, "resposta crua do LLM (OREIA_SECA_LLM_DEBUG)");
        }
        let parsed: ChatResponse = serde_json::from_str(&raw).map_err(|e| LlmError::Decode(e.to_string()))?;
        let text = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| message_text(c.message))
            .ok_or_else(|| LlmError::Decode("resposta sem choices[0].message.content nem tool_calls".into()))?;

        Ok(Completion { text, usage: parsed.usage.unwrap_or_default(), model: self.key.clone() })
    }

    async fn health(&self) -> bool {
        let req = self.http.get(format!("{}/models", self.base_url)).timeout(Duration::from_secs(3));
        matches!(self.auth(req).send().await, Ok(r) if r.status().is_success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_has_system_first_and_options() {
        let opts = GenerateOpts::system("seja breve").max_tokens(100);
        let b = build_body("qwen", &[ChatMessage::user("oi")], &opts, Some(0.2));
        assert_eq!(b["model"], "qwen");
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"], "oi");
        assert_eq!(b["max_tokens"], 100);
        assert!((b["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(b["stream"], false);
    }

    #[test]
    fn rejects_unknown_provider() {
        let mut spec = ModelSpec::local("http://x/v1", "m");
        spec.provider = "outro".into();
        let resolved = spec.resolve().unwrap();
        assert!(OpenAiCompat::new("k", &spec, &resolved).is_err());
    }

    #[test]
    fn plain_text_content_wins_over_absent_tool_calls() {
        let msg: ChoiceMessage = serde_json::from_str(r#"{"role":"assistant","content":"oi"}"#).unwrap();
        assert_eq!(message_text(msg), Some("oi".to_string()));
    }

    /// Regressão: deepseek via OpenRouter (Fireworks) respondeu com `content: null`
    /// e um `tool_calls` de "shell" mesmo sem o corpo da requisição declarar `tools` —
    /// isso derrubava a fase com "resposta sem choices[0].message.content". Payload
    /// real, só com os campos que `ChoiceMessage` lê.
    #[test]
    fn falls_back_to_tool_calls_when_content_is_null() {
        let raw = r#"{
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "type": "function",
                "id": "call_71f865f46057444fb0e71676",
                "function": {
                    "name": "shell",
                    "arguments": "{\"command\":\"ls -la\"}"
                }
            }]
        }"#;
        let msg: ChoiceMessage = serde_json::from_str(raw).unwrap();
        let text = message_text(msg).expect("deveria sintetizar a partir de tool_calls");
        assert_eq!(text, r#"<action name="shell">{"command":"ls -la"}</action>"#);

        // E o resultado precisa ser reconhecido pelo parser de ações de verdade.
        let actions = oreia_core::actions::parse_actions(&text);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "shell");
        assert_eq!(actions[0].params["command"], "ls -la");
    }

    #[test]
    fn empty_content_and_no_tool_calls_is_still_an_error() {
        let msg: ChoiceMessage = serde_json::from_str(r#"{"role":"assistant","content":null}"#).unwrap();
        assert_eq!(message_text(msg), None);
    }

    /// Teste de verdade contra a API da OpenRouter — `OpenAiCompat` é o que o
    /// executor `builtin` usa por baixo, então isto valida o builtin (os
    /// outros 4 executores externos são validados em
    /// `crates/oreia-seca/src/executor/external.rs`, teste homônimo). Não
    /// roda em CI nem no `cargo test` normal: só com
    /// `OREIA_SECA_LIVE_EXTERNAL_TESTS=1` e `OPENROUTER_KEY` no ambiente.
    #[tokio::test]
    async fn live_openai_compat_works_with_deepseek_and_gemini() {
        if std::env::var("OREIA_SECA_LIVE_EXTERNAL_TESTS").as_deref() != Ok("1") {
            return;
        }
        let Ok(key) = std::env::var("OPENROUTER_KEY") else {
            panic!("OREIA_SECA_LIVE_EXTERNAL_TESTS=1 mas falta OPENROUTER_KEY no ambiente");
        };
        std::env::set_var("OREIA_SECA_LIVE_OPENAI_TEST_KEY", &key);

        for model in ["deepseek/deepseek-v4.1-flash-1017", "google/gemini-2.5-flash-lite"] {
            let mut spec = ModelSpec::local("https://openrouter.ai/api/v1", model);
            spec.api_key_env = Some("OREIA_SECA_LIVE_OPENAI_TEST_KEY".into());
            let resolved = spec.resolve().unwrap();
            let provider = OpenAiCompat::new(model, &spec, &resolved).unwrap();

            let opts = GenerateOpts::default().max_tokens(20);
            let c = provider
                .generate(&[ChatMessage::user("reply with just the word: ok")], &opts)
                .await
                .unwrap_or_else(|e| panic!("{model} falhou contra a API real: {e:#}"));
            assert!(!c.text.trim().is_empty(), "{model} não devolveu texto");
        }
    }
}
