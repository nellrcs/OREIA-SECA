use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oreia_core::ChatMessage;
use serde::Serialize;

use crate::{AnthropicNative, Completion, GenerateOpts, LlmError, LlmProvider, ModelSpec, ModelState, OpenAiCompat};

struct Slot {
    key: String,
    spec: ModelSpec,
    /// `None` quando a variável de ambiente de `base_url_env`/`model_env` está
    /// ausente: o modelo existe em `[models]`, mas não há como chamá-lo.
    provider: Option<Arc<dyn LlmProvider>>,
    offline_until: Mutex<Option<Instant>>,
}

impl Slot {
    fn is_ready(&self) -> bool {
        self.provider.is_some()
            && match *self.offline_until.lock().expect("lock do slot") {
                Some(t) => Instant::now() >= t,
                None => true,
            }
    }

    fn state(&self) -> ModelState {
        if self.provider.is_none() {
            ModelState::NotConfigured
        } else if self.is_ready() {
            ModelState::Ok
        } else {
            ModelState::Unreachable
        }
    }

    fn mark_offline(&self, cooldown: Duration) {
        *self.offline_until.lock().expect("lock do slot") = Some(Instant::now() + cooldown);
    }

    fn mark_online(&self) {
        *self.offline_until.lock().expect("lock do slot") = None;
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoleInfo {
    pub role: String,
    pub model: String,
    pub ready: bool,
}

/// Estado de um modelo (não de um papel) — para `/models` e `GET /api/status`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelInfo {
    pub key: String,
    pub state: ModelState,
    /// Vazio quando `state` é `not_configured`. Nunca inclui a chave da API.
    pub base_url: String,
    pub model: String,
}

fn no_model_for_role(role: &str) -> LlmError {
    LlmError::NoModel(format!("⚠️ Nenhum modelo acessível para o papel \"{role}\". Configure com make llm-setup."))
}

/// Roteia chamadas por papel com cadeia de reserva.
///
/// Ordem de tentativa para um papel: modelos do próprio papel, depois os do
/// papel `default`, depois a cadeia `fallback`. Diferente do original, um
/// modelo que falha fica offline só por `cooldown` e volta sozinho.
pub struct ModelRouter {
    slots: HashMap<String, Arc<Slot>>,
    roles: BTreeMap<String, Vec<String>>,
    fallback: Vec<String>,
    default_timeout: Option<Duration>,
    cooldown: Duration,
}

impl ModelRouter {
    pub fn new(cooldown: Duration, default_timeout: Option<Duration>) -> Self {
        Self { slots: HashMap::new(), roles: BTreeMap::new(), fallback: Vec::new(), default_timeout, cooldown }
    }

    /// Monta o router a partir do registro de modelos e dos papéis do config.
    /// Um modelo cuja variável de ambiente (`base_url_env`/`model_env`) está
    /// ausente fica registrado como "não configurado", mas não derruba o boot
    /// — só falha aqui um provedor com formato realmente inválido.
    pub fn from_config(
        models: &BTreeMap<String, ModelSpec>,
        roles: &BTreeMap<String, Vec<String>>,
        fallback: &[String],
        cooldown: Duration,
        default_timeout: Option<Duration>,
    ) -> Result<Self, LlmError> {
        let mut r = Self::new(cooldown, default_timeout);
        for (key, spec) in models {
            match spec.resolve() {
                Ok(resolved) => {
                    if let Some(missing_var) = spec.missing_api_key_var() {
                        tracing::warn!(model = %key, var = %missing_var, "modelo não configurado: variável de ambiente da chave ausente");
                        r.add_unconfigured(key, spec.clone());
                        continue;
                    }
                    let provider: Arc<dyn LlmProvider> = match spec.provider.as_str() {
                        "anthropic" => Arc::new(AnthropicNative::new(key, spec, &resolved)?),
                        _ => Arc::new(OpenAiCompat::new(key, spec, &resolved)?),
                    };
                    r.add_model(key, spec.clone(), provider);
                }
                Err(missing_var) => {
                    tracing::warn!(model = %key, var = %missing_var, "modelo não configurado: variável de ambiente ausente");
                    r.add_unconfigured(key, spec.clone());
                }
            }
        }
        for (role, keys) in roles {
            r.set_role(role, keys.clone());
        }
        r.set_fallback(fallback.to_vec());

        let known: Vec<&String> = roles.values().flatten().chain(fallback).collect();
        for k in known {
            if !r.slots.contains_key(k) {
                tracing::warn!(model = %k, "papel cita um modelo que não está em [models]");
            }
        }
        Ok(r)
    }

    pub fn add_model(&mut self, key: &str, spec: ModelSpec, provider: Arc<dyn LlmProvider>) {
        self.slots.insert(
            key.to_string(),
            Arc::new(Slot { key: key.into(), spec, provider: Some(provider), offline_until: Mutex::new(None) }),
        );
    }

    /// Registra um modelo sem provedor: existe em `[models]`, mas a variável de
    /// ambiente da URL ou do nome do modelo está ausente. Nunca entra em
    /// `candidates`, só aparece em `describe_models` como "não configurado".
    pub fn add_unconfigured(&mut self, key: &str, spec: ModelSpec) {
        self.slots.insert(
            key.to_string(),
            Arc::new(Slot { key: key.into(), spec, provider: None, offline_until: Mutex::new(None) }),
        );
    }

    pub fn set_role(&mut self, role: &str, keys: Vec<String>) {
        self.roles.insert(role.to_string(), keys);
    }

    pub fn set_fallback(&mut self, keys: Vec<String>) {
        self.fallback = keys;
    }

    /// Lista ordenada e sem repetição dos modelos a tentar para o papel.
    fn candidates(&self, role: &str) -> Vec<Arc<Slot>> {
        let mut seen = std::collections::HashSet::new();
        let chain = self
            .roles
            .get(role)
            .into_iter()
            .flatten()
            .chain(self.roles.get("default").into_iter().flatten())
            .chain(self.fallback.iter());

        let all: Vec<Arc<Slot>> = chain
            .filter(|k| seen.insert((*k).clone()))
            .filter_map(|k| self.slots.get(k).cloned())
            .filter(|s| s.provider.is_some())
            .collect();

        // Prontos primeiro, mantendo a ordem; se nenhum está pronto, tenta todos.
        let (ready, offline): (Vec<_>, Vec<_>) = all.into_iter().partition(|s| s.is_ready());
        if ready.is_empty() {
            offline
        } else {
            ready
        }
    }

    /// Especificação do modelo que atenderia o papel agora.
    pub fn spec_for(&self, role: &str) -> Option<ModelSpec> {
        self.candidates(role).first().map(|s| s.spec.clone())
    }

    /// Especificação de um modelo específico (pelo nome em `[models]`), sem roteamento por papel.
    pub fn spec_for_model(&self, key: &str) -> Option<ModelSpec> {
        self.slots.get(key).map(|s| s.spec.clone())
    }

    pub fn active_model(&self, role: &str) -> Option<String> {
        self.candidates(role).first().map(|s| s.key.clone())
    }

    pub fn describe(&self) -> Vec<RoleInfo> {
        self.roles
            .keys()
            .map(|role| {
                let first = self.candidates(role).into_iter().next();
                RoleInfo {
                    role: role.clone(),
                    model: first.as_ref().map(|s| s.key.clone()).unwrap_or_else(|| "nenhum".into()),
                    ready: first.map(|s| s.is_ready()).unwrap_or(false),
                }
            })
            .collect()
    }

    /// Estado de cada modelo em `[models]` (não por papel) — para `/models` e
    /// `GET /api/status`. Nunca inclui a chave da API.
    pub fn describe_models(&self) -> Vec<ModelInfo> {
        let mut v: Vec<ModelInfo> = self
            .slots
            .values()
            .map(|s| {
                let resolved = s.spec.resolve().ok();
                ModelInfo {
                    key: s.key.clone(),
                    state: s.state(),
                    base_url: resolved.as_ref().map(|r| r.base_url.clone()).unwrap_or_default(),
                    model: resolved.map(|r| r.model).unwrap_or_default(),
                }
            })
            .collect();
        v.sort_by(|a, b| a.key.cmp(&b.key));
        v
    }

    /// Verifica a saúde de todos os modelos configurados e marca offline os que
    /// não respondem. Modelos sem provedor (variável de ambiente ausente) nem
    /// entram nessa checagem — já aparecem como "não configurado".
    pub async fn check_health(&self) {
        for slot in self.slots.values() {
            let Some(provider) = &slot.provider else { continue };
            if provider.health().await {
                slot.mark_online();
            } else {
                tracing::warn!(model = %slot.key, "modelo indisponível no boot");
                slot.mark_offline(self.cooldown);
            }
        }
    }

    pub async fn generate(
        &self,
        role: &str,
        messages: &[ChatMessage],
        opts: &GenerateOpts,
    ) -> Result<Completion, LlmError> {
        let candidates = self.candidates(role);
        if candidates.is_empty() {
            return Err(no_model_for_role(role));
        }

        let mut last_err = None;
        for slot in candidates {
            let provider = slot.provider.as_ref().expect("candidates só devolve slots configurados");
            let timeout = slot.spec.timeout_secs.map(Duration::from_secs).or(self.default_timeout);
            let call = provider.generate(messages, opts);
            let result = match timeout {
                Some(d) => tokio::time::timeout(d, call).await.unwrap_or(Err(LlmError::Timeout(d))),
                None => call.await,
            };

            match result {
                Ok(c) => {
                    tracing::debug!(
                        role,
                        model = %slot.key,
                        prompt = c.usage.prompt_tokens,
                        completion = c.usage.completion_tokens,
                        "tokens"
                    );
                    return Ok(c);
                }
                Err(e) if e.should_fallback() => {
                    tracing::warn!(role, model = %slot.key, error = %e, "modelo falhou, tentando o próximo");
                    slot.mark_offline(self.cooldown);
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| no_model_for_role(role)))
    }

    /// Chama um modelo específico (pela chave em `[models]`), sem cadeia de reserva por papel.
    /// Usado pelos degraus de escalada, que fixam o modelo do degrau: uma falha aqui é
    /// tratada pelo chamador (próximo degrau), não pelo roteador.
    pub async fn generate_with_model(
        &self,
        key: &str,
        messages: &[ChatMessage],
        opts: &GenerateOpts,
    ) -> Result<Completion, LlmError> {
        let slot =
            self.slots.get(key).ok_or_else(|| LlmError::NoModel(format!("modelo \"{key}\" não existe em [models]")))?;
        let Some(provider) = &slot.provider else {
            return Err(LlmError::NoModel(format!(
                "modelo \"{key}\" não está configurado (variável de ambiente ausente)"
            )));
        };
        let timeout = slot.spec.timeout_secs.map(Duration::from_secs).or(self.default_timeout);
        let call = provider.generate(messages, opts);
        let result = match timeout {
            Some(d) => tokio::time::timeout(d, call).await.unwrap_or(Err(LlmError::Timeout(d))),
            None => call.await,
        };
        if let Err(e) = &result {
            if e.should_fallback() {
                slot.mark_offline(self.cooldown);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    enum Behavior {
        Ok(&'static str),
        Down,
        BadRequest,
        Slow,
    }

    struct Fake {
        behavior: Behavior,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl LlmProvider for Fake {
        fn model_name(&self) -> &str {
            "fake"
        }
        async fn generate(&self, _: &[ChatMessage], _: &GenerateOpts) -> Result<Completion, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.behavior {
                Behavior::Ok(t) => Ok(Completion { text: t.into(), usage: Usage::default(), model: String::new() }),
                Behavior::Down => Err(LlmError::Connection("recusada".into())),
                Behavior::BadRequest => Err(LlmError::Http { status: 400, body: "ruim".into() }),
                Behavior::Slow => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    unreachable!()
                }
            }
        }
    }

    use crate::Usage;

    fn fake(b: Behavior) -> Arc<Fake> {
        Arc::new(Fake { behavior: b, calls: AtomicUsize::new(0) })
    }

    fn router(models: Vec<(&str, Arc<Fake>)>, cooldown: Duration) -> ModelRouter {
        let mut r = ModelRouter::new(cooldown, Some(Duration::from_millis(50)));
        for (k, f) in models {
            r.add_model(k, ModelSpec::local("http://x", k), f);
        }
        r
    }

    #[tokio::test]
    async fn falls_back_on_connection_error() {
        let a = fake(Behavior::Down);
        let b = fake(Behavior::Ok("da reserva"));
        let mut r = router(vec![("a", a.clone()), ("b", b.clone())], Duration::from_secs(60));
        r.set_role("executor", vec!["a".into(), "b".into()]);

        let c = r.generate("executor", &[], &GenerateOpts::default()).await.unwrap();
        assert_eq!(c.text, "da reserva");
        assert_eq!(r.active_model("executor").as_deref(), Some("b"), "a fica offline");

        r.generate("executor", &[], &GenerateOpts::default()).await.unwrap();
        assert_eq!(a.calls.load(Ordering::SeqCst), 1, "não tenta o offline de novo");
    }

    #[tokio::test]
    async fn does_not_fall_back_on_bad_request() {
        let a = fake(Behavior::BadRequest);
        let b = fake(Behavior::Ok("x"));
        let mut r = router(vec![("a", a), ("b", b.clone())], Duration::from_secs(60));
        r.set_role("planner", vec!["a".into(), "b".into()]);
        assert!(r.generate("planner", &[], &GenerateOpts::default()).await.is_err());
        assert_eq!(b.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn timeout_triggers_fallback() {
        let mut r = router(
            vec![("lento", fake(Behavior::Slow)), ("rapido", fake(Behavior::Ok("ok")))],
            Duration::from_secs(60),
        );
        r.set_role("default", vec!["lento".into()]);
        r.set_fallback(vec!["rapido".into()]);
        let c = r.generate("dev-back", &[], &GenerateOpts::default()).await.unwrap();
        assert_eq!(c.text, "ok");
    }

    #[tokio::test]
    async fn offline_recovers_after_cooldown() {
        let a = fake(Behavior::Down);
        let mut r = router(vec![("a", a.clone()), ("b", fake(Behavior::Ok("b")))], Duration::from_millis(30));
        r.set_role("default", vec!["a".into(), "b".into()]);
        r.generate("default", &[], &GenerateOpts::default()).await.unwrap();
        assert_eq!(r.active_model("default").as_deref(), Some("b"));
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(r.active_model("default").as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn unknown_role_uses_default_then_errors_when_empty() {
        let mut r = router(vec![("a", fake(Behavior::Ok("a")))], Duration::from_secs(1));
        assert!(matches!(r.generate("qualquer", &[], &GenerateOpts::default()).await, Err(LlmError::NoModel(_))));
        r.set_role("default", vec!["a".into()]);
        assert_eq!(r.generate("qualquer", &[], &GenerateOpts::default()).await.unwrap().text, "a");
    }

    #[tokio::test]
    async fn unconfigured_model_is_skipped_and_reported() {
        let mut spec = ModelSpec::local("http://x", "m");
        spec.base_url = None;
        spec.base_url_env = Some("OREIA_SECA_ROUTER_TEST_MISSING".into());
        std::env::remove_var("OREIA_SECA_ROUTER_TEST_MISSING");

        let mut r = ModelRouter::new(Duration::from_secs(60), None);
        r.add_unconfigured("a", spec);
        r.add_model("b", ModelSpec::local("http://x", "m"), fake(Behavior::Ok("b")));
        r.set_role("default", vec!["a".into(), "b".into()]);

        // O modelo não configurado nunca é tentado: "b" responde de primeira.
        assert_eq!(r.generate("default", &[], &GenerateOpts::default()).await.unwrap().text, "b");

        let models = r.describe_models();
        let a = models.iter().find(|m| m.key == "a").unwrap();
        assert_eq!(a.state, ModelState::NotConfigured);
        assert!(a.base_url.is_empty() && a.model.is_empty());
        let b = models.iter().find(|m| m.key == "b").unwrap();
        assert_eq!(b.state, ModelState::Ok);
    }

    #[tokio::test]
    async fn describe_models_never_leaks_the_api_key() {
        std::env::set_var("OREIA_SECA_ROUTER_TEST_SECRET_KEY", "s3cr3t-token");
        let mut spec = ModelSpec::local("http://x/v1", "m");
        spec.api_key_env = Some("OREIA_SECA_ROUTER_TEST_SECRET_KEY".into());

        let mut r = ModelRouter::new(Duration::from_secs(60), None);
        r.add_model("a", spec, fake(Behavior::Ok("ok")));

        let models = r.describe_models();
        let json = serde_json::to_string(&models).unwrap();
        assert!(!json.contains("s3cr3t-token"), "{json}");
        assert!(!json.contains("OREIA_SECA_ROUTER_TEST_SECRET_KEY"), "{json}");
    }

    #[tokio::test]
    async fn anthropic_without_api_key_is_unconfigured_not_fatal() {
        // Regressão: antes, um [models.*] com provider = "anthropic" e
        // api_key_env ausente/vazio derrubava o boot inteiro (Err propagado de
        // from_config), quebrando a garantia de boot tolerante que vale para
        // base_url_env/model_env ausentes.
        std::env::remove_var("OREIA_SECA_ROUTER_TEST_ANTHROPIC_KEY");
        let mut spec = ModelSpec::local("https://api.anthropic.com/v1", "claude-sonnet-5-5");
        spec.provider = "anthropic".into();
        spec.api_key_env = Some("OREIA_SECA_ROUTER_TEST_ANTHROPIC_KEY".into());

        let mut models = BTreeMap::new();
        models.insert("claude".to_string(), spec);
        let mut roles = BTreeMap::new();
        roles.insert("default".to_string(), vec!["claude".to_string()]);

        let r = ModelRouter::from_config(&models, &roles, &[], Duration::from_secs(60), None).unwrap();
        let models = r.describe_models();
        let claude = models.iter().find(|m| m.key == "claude").unwrap();
        assert_eq!(claude.state, ModelState::NotConfigured);
    }

    #[tokio::test]
    async fn boot_without_any_reachable_model_does_not_fail() {
        let mut spec = ModelSpec::local("http://x", "m");
        spec.base_url = None;
        spec.base_url_env = Some("OREIA_SECA_ROUTER_TEST_MISSING_2".into());
        std::env::remove_var("OREIA_SECA_ROUTER_TEST_MISSING_2");

        let mut models = BTreeMap::new();
        models.insert("a".to_string(), spec);
        let mut roles = BTreeMap::new();
        roles.insert("default".to_string(), vec!["a".to_string()]);

        let r = ModelRouter::from_config(&models, &roles, &[], Duration::from_secs(60), None).unwrap();
        let err = r.generate("default", &[], &GenerateOpts::default()).await.unwrap_err();
        assert!(err.to_string().contains("make llm-setup"), "{err}");
    }
}
