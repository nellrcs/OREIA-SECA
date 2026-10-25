//! Camada de modelos.
//!
//! Dois provedores: [`OpenAiCompat`] cobre qualquer servidor que fale
//! `/v1/chat/completions` (Ollama, LM Studio, llama.cpp, vLLM, OpenRouter, o
//! endpoint compatível do Gemini); [`AnthropicNative`] fala a API própria da
//! Anthropic (`/v1/messages`) para quem tem uma chave direto do
//! console.anthropic.com — os formatos de request/response são diferentes,
//! então não dá para reusar o `OpenAiCompat` ali.
//!
//! O [`ModelRouter`] escolhe o modelo por papel (planner, executor, direct,
//! ou o papel de um agente como `dev-back`) com cadeia de reserva: se o
//! titular cai ou estoura o tempo, ele é marcado offline por um período e o
//! próximo da lista assume.

mod anthropic;
mod openai;
mod router;

use std::time::Duration;

use async_trait::async_trait;
use oreia_core::ChatMessage;
use serde::{Deserialize, Serialize};

pub use anthropic::AnthropicNative;
pub use openai::OpenAiCompat;
pub use router::{ModelInfo, ModelRouter, RoleInfo};

#[derive(Debug, Clone, Default)]
pub struct GenerateOpts {
    pub system: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

impl GenerateOpts {
    pub fn system(s: impl Into<String>) -> Self {
        Self { system: Some(s.into()), ..Default::default() }
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
}

impl Usage {
    pub fn total(&self) -> u32 {
        self.prompt_tokens + self.completion_tokens
    }
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub usage: Usage,
    /// Chave do modelo (no registro) que gerou a resposta.
    pub model: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("falha de conexão: {0}")]
    Connection(String),
    #[error("modelo não respondeu em {0:?}")]
    Timeout(Duration),
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("resposta inválida: {0}")]
    Decode(String),
    /// A mensagem já vem pronta do chamador (ver `ModelRouter::generate`/`generate_with_model`),
    /// porque o texto certo depende de ser um papel (cadeia de reserva vazia) ou um modelo
    /// específico (chave inexistente ou não configurada).
    #[error("{0}")]
    NoModel(String),
}

impl LlmError {
    /// Erros em que vale tentar o próximo modelo da cadeia.
    pub fn should_fallback(&self) -> bool {
        match self {
            LlmError::Connection(_) | LlmError::Timeout(_) => true,
            LlmError::Http { status, .. } => *status == 429 || *status >= 500,
            LlmError::Decode(_) | LlmError::NoModel(_) => false,
        }
    }
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Nome do modelo no provedor (ex.: `qwen2.5-coder:14b`).
    fn model_name(&self) -> &str;
    async fn generate(&self, messages: &[ChatMessage], opts: &GenerateOpts) -> Result<Completion, LlmError>;
    /// Verificação rápida de disponibilidade.
    async fn health(&self) -> bool {
        true
    }
}

/// Especificação de um modelo no `oreia-seca.toml`. `base_url`/`model` podem vir
/// literais no arquivo OU de uma variável de ambiente (`base_url_env`/`model_env`)
/// — exatamente um de cada par — para apontar a um LLM externo (outra máquina,
/// LM Studio, OpenRouter...) sem gravar endereço nem nome de modelo no arquivo.
/// Veja `validate_shape` (forma, no load) e `resolve` (valor, ao montar o roteador).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelSpec {
    /// `openai_compat` (padrão, qualquer servidor `/v1/chat/completions`) ou
    /// `anthropic` (API nativa da Anthropic, `/v1/messages`, exige `api_key_env`).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Ex.: `http://ollama:11434/v1`, `http://host.docker.internal:1234/v1`,
    /// `https://openrouter.ai/api/v1`, `https://generativelanguage.googleapis.com/v1beta/openai`.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Variável de ambiente com a URL, no lugar de `base_url`.
    #[serde(default)]
    pub base_url_env: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Variável de ambiente com o nome do modelo, no lugar de `model`.
    #[serde(default)]
    pub model_env: Option<String>,
    /// Nome da variável de ambiente com a chave (opcional para modelos locais).
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Janela de contexto do modelo.
    #[serde(default = "default_context")]
    pub context: u32,
    /// Tokens reservados para a resposta.
    #[serde(default = "default_reserve")]
    pub reserve_output: u32,
    /// Timeout por chamada, em segundos.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub temperature: Option<f32>,
}

fn default_provider() -> String {
    "openai_compat".into()
}
fn default_context() -> u32 {
    8_192
}
fn default_reserve() -> u32 {
    2_048
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// `OREIA_SECA_LLM_DEBUG=1` faz [`OpenAiCompat`] e [`AnthropicNative`] logarem
/// (em `debug`) o corpo completo das requisições e respostas aos modelos —
/// nunca a chave da API, que vai só no cabeçalho. Desligado por padrão: esses
/// provedores ainda são experimentais e o conteúdo logado inclui prompts e
/// código do usuário, então ativar é uma escolha explícita, não o padrão.
pub(crate) fn llm_debug_enabled() -> bool {
    std::env::var("OREIA_SECA_LLM_DEBUG").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "True"))
}

/// `base_url`/`model` já resolvidos (literal ou lido do ambiente).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    pub base_url: String,
    pub model: String,
}

impl ModelSpec {
    pub fn local(base_url: &str, model: &str) -> Self {
        Self {
            provider: default_provider(),
            base_url: Some(base_url.into()),
            base_url_env: None,
            model: Some(model.into()),
            model_env: None,
            api_key_env: None,
            context: default_context(),
            reserve_output: default_reserve(),
            timeout_secs: None,
            temperature: None,
        }
    }

    /// Confere que exatamente um de `base_url`/`base_url_env` e um de `model`/`model_env`
    /// foi dado. Checagem de forma só — não lê o ambiente (isso é `resolve`).
    pub fn validate_shape(&self, key: &str) -> Result<(), String> {
        match (&self.base_url, &self.base_url_env) {
            (Some(_), Some(_)) => return Err(format!("[models.{key}] tem base_url e base_url_env; use só um")),
            (None, None) => return Err(format!("[models.{key}] precisa de base_url ou base_url_env")),
            _ => {}
        }
        match (&self.model, &self.model_env) {
            (Some(_), Some(_)) => return Err(format!("[models.{key}] tem model e model_env; use só um")),
            (None, None) => return Err(format!("[models.{key}] precisa de model ou model_env")),
            _ => {}
        }
        Ok(())
    }

    /// Para provedores que exigem chave (hoje só `anthropic` — `openai_compat`
    /// funciona sem, para servidor local sem autenticação): nome da variável de
    /// ambiente que falta, se for o caso. `None` quer dizer "sem pendência", o
    /// que inclui provedores que não exigem chave. Mesmo contrato de `resolve`:
    /// não é fatal, `ModelRouter::from_config` marca como "não configurado" e
    /// segue o boot sem esse modelo em vez de abortar.
    pub fn missing_api_key_var(&self) -> Option<String> {
        if self.provider != "anthropic" {
            return None;
        }
        match &self.api_key_env {
            Some(name) if env_var(name).is_some() => None,
            Some(name) => Some(name.clone()),
            None => Some("api_key_env".to_string()),
        }
    }

    /// Resolve `base_url`/`model`, lendo as variáveis de ambiente quando configuradas.
    /// `Err` traz o nome da variável que falta — não é fatal: `ModelRouter::from_config`
    /// marca o modelo como "não configurado" e segue o boot sem ele.
    pub fn resolve(&self) -> Result<ResolvedModel, String> {
        let base_url = match (&self.base_url, &self.base_url_env) {
            (Some(b), _) => b.clone(),
            (None, Some(e)) => env_var(e).ok_or_else(|| e.clone())?,
            (None, None) => unreachable!("forma checada por validate_shape"),
        };
        let model = match (&self.model, &self.model_env) {
            (Some(m), _) => m.clone(),
            (None, Some(e)) => env_var(e).ok_or_else(|| e.clone())?,
            (None, None) => unreachable!("forma checada por validate_shape"),
        };
        Ok(ResolvedModel { base_url, model })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    /// Falta a variável de ambiente (`base_url_env`/`model_env`).
    NotConfigured,
    /// Resolvido, mas a última checagem de saúde ou chamada falhou.
    Unreachable,
    Ok,
}

impl ModelState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelState::NotConfigured => "não configurado",
            ModelState::Unreachable => "inacessível",
            ModelState::Ok => "ok",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_debug_enabled_accepts_common_truthy_spellings_and_defaults_off() {
        std::env::remove_var("OREIA_SECA_LLM_DEBUG");
        assert!(!llm_debug_enabled());

        for v in ["1", "true", "TRUE", "True"] {
            std::env::set_var("OREIA_SECA_LLM_DEBUG", v);
            assert!(llm_debug_enabled(), "{v} devia habilitar");
        }
        for v in ["0", "false", "", "yes"] {
            std::env::set_var("OREIA_SECA_LLM_DEBUG", v);
            assert!(!llm_debug_enabled(), "{v} não devia habilitar");
        }
        std::env::remove_var("OREIA_SECA_LLM_DEBUG");
    }
}
