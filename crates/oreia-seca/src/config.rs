//! Configuração (`oreia-seca.toml`). Segredos nunca ficam no arquivo: ele guarda
//! apenas o *nome* da variável de ambiente (`*_env`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use oreia_core::Source;
use oreia_llm::ModelSpec;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub paths: PathsConfig,
    #[serde(default)]
    pub queue: QueueConfig,
    #[serde(default)]
    pub approval: ApprovalConfig,
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub executor: ExecutorConfig,
    #[serde(default)]
    pub models: BTreeMap<String, ModelSpec>,
    #[serde(default)]
    pub roles: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub forgejo: Option<ForgejoConfig>,
    #[serde(default)]
    pub agents: Vec<AgentConfig>,
    /// Presets de executores externos (sobrescrevem os embutidos: aider, opencode, goose, codex).
    #[serde(default)]
    pub executors: BTreeMap<String, ExecutorPreset>,
    /// Escada de escalada por papel. Papel sem entrada aqui usa um degrau só (comportamento atual).
    #[serde(default)]
    pub escalation: BTreeMap<String, EscalationConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "d_host")]
    pub host: String,
    #[serde(default = "d_port")]
    pub port: u16,
    /// Variável com o token Bearer exigido em `/api/*`.
    #[serde(default = "d_api_key_env")]
    pub api_key_env: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { host: d_host(), port: d_port(), api_key_env: d_api_key_env() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathsConfig {
    #[serde(default = "d_data")]
    pub data: PathBuf,
    #[serde(default = "d_workspace")]
    pub workspace: PathBuf,
    #[serde(default = "d_skills")]
    pub skills: PathBuf,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self { data: d_data(), workspace: d_workspace(), skills: d_skills() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueConfig {
    #[serde(default = "d_one")]
    pub concurrency: usize,
    /// 0 = sem limite.
    #[serde(default = "d_task_timeout")]
    pub timeout_secs: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self { concurrency: 1, timeout_secs: d_task_timeout() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalConfig {
    /// Fontes cujas tarefas esperam `/approve` antes de executar.
    #[serde(default = "d_approval_sources")]
    pub sources: Vec<Source>,
    #[serde(default = "d_approval_timeout")]
    pub timeout_secs: u64,
}

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self { sources: d_approval_sources(), timeout_secs: d_approval_timeout() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramConfig {
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default = "d_tg_token")]
    pub token_env: String,
    /// IDs separados por vírgula.
    #[serde(default = "d_tg_allowed")]
    pub allowed_env: String,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self { enabled: true, token_env: d_tg_token(), allowed_env: d_tg_allowed() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalConfig {
    #[serde(default = "d_true")]
    pub enabled: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmConfig {
    /// Quanto tempo um modelo que falhou fica fora da rotação.
    #[serde(default = "d_cooldown")]
    pub cooldown_secs: u64,
    /// Timeout padrão por chamada (cada modelo pode sobrescrever).
    #[serde(default = "d_llm_timeout")]
    pub timeout_secs: u64,
    /// Cadeia global de reserva.
    #[serde(default)]
    pub fallback: Vec<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self { cooldown_secs: d_cooldown(), timeout_secs: d_llm_timeout(), fallback: Vec::new() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorConfig {
    /// Rodadas modelo → ações → resultado dentro de uma fase (executor `builtin`).
    #[serde(default = "d_max_steps")]
    pub max_steps: usize,
    #[serde(default = "d_phase_retries")]
    pub phase_retries: usize,
    #[serde(default = "d_shell_timeout")]
    pub shell_timeout_secs: u64,
    #[serde(default = "d_max_phases")]
    pub max_phases: usize,
    /// Executor usado por papéis sem `[escalation.<papel>]`: `builtin` ou um nome de `[executors.*]`.
    #[serde(default = "d_default_executor")]
    pub default: String,
    /// Timeout de uma tentativa de fase inteira (um degrau), diferente de `shell_timeout_secs`
    /// (que é por comando de shell dentro do executor `builtin`).
    #[serde(default = "d_step_timeout")]
    pub step_timeout_secs: u64,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_steps: d_max_steps(),
            phase_retries: d_phase_retries(),
            shell_timeout_secs: d_shell_timeout(),
            max_phases: d_max_phases(),
            default: d_default_executor(),
            step_timeout_secs: d_step_timeout(),
        }
    }
}

/// Preset de um executor externo (CLI de terceiros). `command` é um template com
/// `{model}`, `{base_url}`, `{api_key}`, `{message_file}` e `{test_cmd}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorPreset {
    pub command: String,
}

/// Um degrau da escada de escalada: qual executor e qual modelo (de `[models]`) tentar.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscalationStep {
    pub executor: String,
    pub model: String,
    #[serde(default)]
    pub cost_per_mtok_in: f64,
    #[serde(default)]
    pub cost_per_mtok_out: f64,
}

/// Escada de escalada de um papel: degraus em ordem, aprovação e teto de custo.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscalationConfig {
    pub steps: Vec<EscalationStep>,
    /// A partir deste degrau (1-based), pede aprovação no Telegram antes de tentar.
    #[serde(default)]
    pub ask_before_step: Option<usize>,
    /// Teto de custo estimado por tarefa; ao atingir, a escalada para.
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Comando que valida uma tentativa (ex.: `cargo test`). Sem ele, a verificação
    /// só confere se o executor não falhou e se algo mudou (modo fraco).
    #[serde(default)]
    pub test_cmd: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgejoConfig {
    /// URL interna usada pela API e pelo git (ex.: `http://forgejo:3000`).
    pub url: String,
    /// URL que você abre no navegador (para os links nas mensagens).
    #[serde(default)]
    pub public_url: Option<String>,
    /// Base para clonar via git, se diferente de `url` (ex.: outro host ou `file://` em testes).
    #[serde(default)]
    pub git_url: Option<String>,
    #[serde(default = "d_webhook_secret_env")]
    pub webhook_secret_env: String,
    /// Token do *seu* usuário: usado por /prs, /merge e /changes.
    #[serde(default = "d_owner_token_env")]
    pub owner_token_env: String,
    /// Repositórios acompanhados por /prs (`dono/repo`).
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default = "d_main")]
    pub base_branch: String,
    #[serde(default = "d_merge")]
    pub merge_style: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    /// Identificador do agente (ex.: `dev-back`).
    pub name: String,
    /// Label da issue que aciona o agente (ex.: `agent:back`).
    pub label: String,
    /// Usuário do agente no Forgejo.
    pub user: String,
    /// Variável com o token de acesso do agente.
    pub token_env: String,
    /// Papel no roteador de modelos. Padrão: o próprio `name`.
    #[serde(default)]
    pub model_role: Option<String>,
    /// Arquivo com as instruções do agente (AGENT.md).
    #[serde(default)]
    pub prompt: Option<PathBuf>,
}

impl AgentConfig {
    pub fn role(&self) -> &str {
        self.model_role.as_deref().unwrap_or(&self.name)
    }
}

fn d_host() -> String {
    "0.0.0.0".into()
}
fn d_port() -> u16 {
    3120
}
fn d_api_key_env() -> String {
    "REST_API_KEY".into()
}
fn d_data() -> PathBuf {
    "data".into()
}
fn d_workspace() -> PathBuf {
    "workspace".into()
}
fn d_skills() -> PathBuf {
    "skills".into()
}
fn d_one() -> usize {
    1
}
fn d_task_timeout() -> u64 {
    3600
}
fn d_approval_sources() -> Vec<Source> {
    vec![Source::Rest, Source::Cron]
}
fn d_approval_timeout() -> u64 {
    600
}
fn d_true() -> bool {
    true
}
fn d_tg_token() -> String {
    "TELEGRAM_TOKEN".into()
}
fn d_tg_allowed() -> String {
    "TELEGRAM_ALLOWED".into()
}
fn d_cooldown() -> u64 {
    120
}
fn d_llm_timeout() -> u64 {
    600
}
fn d_max_steps() -> usize {
    4
}
fn d_phase_retries() -> usize {
    2
}
fn d_shell_timeout() -> u64 {
    300
}
fn d_max_phases() -> usize {
    8
}
fn d_default_executor() -> String {
    "builtin".into()
}
fn d_step_timeout() -> u64 {
    900
}
fn d_webhook_secret_env() -> String {
    "FORGEJO_WEBHOOK_SECRET".into()
}
fn d_owner_token_env() -> String {
    "FORGEJO_OWNER_TOKEN".into()
}
fn d_main() -> String {
    "main".into()
}
fn d_merge() -> String {
    "merge".into()
}

/// Lê uma variável de ambiente, tratando vazio como ausente.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("não consegui ler {}: {e}", path.display()))?;
        let cfg: Config = toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.models.is_empty(), "nenhum modelo em [models]");
        for (key, spec) in &self.models {
            spec.validate_shape(key).map_err(|e| anyhow::anyhow!(e))?;
        }
        anyhow::ensure!(
            self.roles.contains_key("default") || !self.llm.fallback.is_empty(),
            "defina [roles].default ou [llm].fallback para ter sempre um modelo"
        );
        for (role, keys) in &self.roles {
            for k in keys {
                anyhow::ensure!(
                    self.models.contains_key(k),
                    "papel \"{role}\" usa o modelo \"{k}\", que não existe em [models]"
                );
            }
        }
        for k in &self.llm.fallback {
            anyhow::ensure!(self.models.contains_key(k), "[llm].fallback usa \"{k}\", que não existe em [models]");
        }
        let mut seen = std::collections::HashSet::new();
        for a in &self.agents {
            anyhow::ensure!(seen.insert(&a.name), "agente duplicado: {}", a.name);
            anyhow::ensure!(
                a.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "nome de agente inválido: {} (use letras, números, - e _)",
                a.name
            );
        }
        if !self.agents.is_empty() {
            anyhow::ensure!(self.forgejo.is_some(), "há [[agents]] mas falta a seção [forgejo]");
        }

        self.validate_executor_name(&self.executor.default, "[executor].default")?;
        for (role, esc) in &self.escalation {
            anyhow::ensure!(!esc.steps.is_empty(), "[escalation.{role}].steps está vazio");
            for (i, step) in esc.steps.iter().enumerate() {
                self.validate_executor_name(&step.executor, &format!("[escalation.{role}].steps[{i}].executor"))?;
                anyhow::ensure!(
                    self.models.contains_key(&step.model),
                    "[escalation.{role}].steps[{i}] usa o modelo \"{}\", que não existe em [models]",
                    step.model
                );
            }
            if let Some(ask) = esc.ask_before_step {
                anyhow::ensure!(
                    ask >= 1 && ask <= esc.steps.len(),
                    "[escalation.{role}].ask_before_step deve estar entre 1 e {} (número de degraus)",
                    esc.steps.len()
                );
            }
        }
        Ok(())
    }

    /// `builtin`, um nome de `[executors.*]`, ou um preset embutido (aider/opencode/goose/codex).
    fn validate_executor_name(&self, name: &str, field: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            name == "builtin"
                || self.executors.contains_key(name)
                || crate::executor::external::builtin_preset(name).is_some(),
            "{field} usa o executor \"{name}\", que não é \"builtin\" nem está em [executors] ou nos presets embutidos"
        );
        Ok(())
    }

    pub fn queue_timeout(&self) -> Option<Duration> {
        (self.queue.timeout_secs > 0).then(|| Duration::from_secs(self.queue.timeout_secs))
    }

    pub fn needs_approval(&self, source: Source) -> bool {
        self.approval.sources.contains(&source)
    }

    pub fn agent(&self, name: &str) -> Option<&AgentConfig> {
        self.agents.iter().find(|a| a.name == name)
    }

    pub fn agent_by_label<'a>(&self, labels: impl IntoIterator<Item = &'a str>) -> Option<&AgentConfig> {
        let labels: Vec<&str> = labels.into_iter().collect();
        self.agents.iter().find(|a| labels.iter().any(|l| l.eq_ignore_ascii_case(&a.label)))
    }

    pub fn is_agent_user(&self, login: &str) -> bool {
        self.agents.iter().any(|a| a.user.eq_ignore_ascii_case(login))
    }

    pub fn tasks_dir(&self) -> PathBuf {
        self.paths.data.join("tasks")
    }

    pub fn sessions_file(&self) -> PathBuf {
        self.paths.data.join("sessions.json")
    }

    pub fn summaries_dir(&self) -> PathBuf {
        self.paths.data.join("resumos")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const SAMPLE: &str = r#"
[approval]
sources = ["rest", "cron"]

[models.qwen]
base_url = "http://ollama:11434/v1"
model = "qwen2.5-coder:14b"
context = 32768

[models.llama]
base_url = "http://ollama:11434/v1"
model = "llama3.1:8b"

[roles]
default = ["qwen"]
direct = ["llama", "qwen"]

[llm]
fallback = ["llama"]

[forgejo]
url = "http://forgejo:3000"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "AGENT_DEV_BACK_TOKEN"
"#;

    #[test]
    fn parses_sample_with_defaults() {
        let c: Config = toml::from_str(SAMPLE).unwrap();
        c.validate().unwrap();
        assert_eq!(c.server.port, 3120);
        assert_eq!(c.queue.concurrency, 1);
        assert_eq!(c.models["qwen"].context, 32768);
        assert_eq!(c.models["llama"].context, 8192);
        assert!(c.needs_approval(Source::Rest));
        assert!(!c.needs_approval(Source::Telegram));
        assert_eq!(c.agent("dev-back").unwrap().role(), "dev-back");
        assert_eq!(c.agent_by_label(["bug", "Agent:Back"]).unwrap().name, "dev-back");
        assert!(c.is_agent_user("AGENT-BACK"));
        assert_eq!(c.forgejo.as_ref().unwrap().base_branch, "main");
    }

    #[test]
    fn rejects_unknown_model_in_role() {
        let bad = SAMPLE.replace("direct = [\"llama\", \"qwen\"]", "direct = [\"gpt\"]");
        let c: Config = toml::from_str(&bad).unwrap();
        assert!(c.validate().unwrap_err().to_string().contains("gpt"));
    }

    #[test]
    fn rejects_typos() {
        assert!(toml::from_str::<Config>("[queue]\nconcurency = 2").is_err());
    }

    #[test]
    fn example_file_is_valid() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oreia-seca.example.toml");
        Config::load(&path).unwrap();
    }

    #[test]
    fn escalation_rejects_unknown_model() {
        let bad = format!(
            "{SAMPLE}\n[escalation.dev-back]\nsteps = [{{ executor = \"builtin\", model = \"nao-existe\" }}]\n"
        );
        let c: Config = toml::from_str(&bad).unwrap();
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("nao-existe"), "{err}");
    }

    #[test]
    fn escalation_rejects_unknown_executor() {
        let bad =
            format!("{SAMPLE}\n[escalation.dev-back]\nsteps = [{{ executor = \"inexistente\", model = \"qwen\" }}]\n");
        let c: Config = toml::from_str(&bad).unwrap();
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("inexistente"), "{err}");
    }

    #[test]
    fn escalation_accepts_builtin_presets_and_custom_executors() {
        let ok = format!(
            "{SAMPLE}\n[executors.minha-cli]\ncommand = \"minha-cli {{message_file}}\"\n\n\
             [escalation.dev-back]\nsteps = [\n  {{ executor = \"aider\", model = \"qwen\" }},\n  {{ executor = \"minha-cli\", model = \"llama\" }},\n]\n"
        );
        let c: Config = toml::from_str(&ok).unwrap();
        c.validate().unwrap();
    }

    #[test]
    fn model_accepts_url_from_env_variant() {
        let ok = SAMPLE.replace(
            "[models.qwen]\nbase_url = \"http://ollama:11434/v1\"\nmodel = \"qwen2.5-coder:14b\"",
            "[models.qwen]\nbase_url_env = \"TEST_QWEN_URL\"\nmodel_env = \"TEST_QWEN_MODEL\"",
        );
        let c: Config = toml::from_str(&ok).unwrap();
        c.validate().unwrap();
    }

    #[test]
    fn model_rejects_both_url_variants() {
        // A primeira ocorrência (bloco [models.qwen]) ganha base_url_env além de base_url.
        let bad = SAMPLE.replacen(
            "base_url = \"http://ollama:11434/v1\"",
            "base_url = \"http://ollama:11434/v1\"\nbase_url_env = \"X\"",
            1,
        );
        let c: Config = toml::from_str(&bad).unwrap();
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("base_url") && err.contains("qwen"), "{err}");
    }

    #[test]
    fn model_rejects_neither_url_variant() {
        // Remove só a primeira ocorrência: [models.qwen] fica sem base_url nem base_url_env.
        let bad = SAMPLE.replacen("base_url = \"http://ollama:11434/v1\"\n", "", 1);
        let c: Config = toml::from_str(&bad).unwrap();
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("base_url") && err.contains("qwen"), "{err}");
    }

    #[test]
    fn escalation_rejects_ask_before_step_out_of_range() {
        let bad = format!(
            "{SAMPLE}\n[escalation.dev-back]\nsteps = [{{ executor = \"builtin\", model = \"qwen\" }}]\nask_before_step = 2\n"
        );
        let c: Config = toml::from_str(&bad).unwrap();
        assert!(c.validate().unwrap_err().to_string().contains("ask_before_step"));
    }
}
