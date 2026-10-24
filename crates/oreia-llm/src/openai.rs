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
        let url = format!("{}/chat/completions", self.base_url);
        let resp = self.auth(self.http.post(&url)).json(&body).send().await.map_err(map_reqwest)?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Http { status: status.as_u16(), body: body.chars().take(500).collect() });
        }

        let parsed: ChatResponse = resp.json().await.map_err(|e| LlmError::Decode(e.to_string()))?;
        let text = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| LlmError::Decode("resposta sem choices[0].message.content".into()))?;

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
}
