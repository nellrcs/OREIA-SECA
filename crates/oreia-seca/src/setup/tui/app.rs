//! Estado da TUI de `oreia-seca setup`. Nenhuma função aqui lê terminal ou
//! rede — é só manipulação de dados, testável sem ratatui (ao contrário de
//! `event.rs`/`ui.rs`/`mod.rs`, que dependem de um terminal real).

use crate::setup::current_state::CurrentState;
use crate::setup::toml_edits::TomlEdits;
use crate::setup::{Answers, ForgejoAnswers, LlmAnswers, McpAnswers, Px0Answers, TelegramAnswers};

use std::collections::{BTreeMap, VecDeque};

use chrono::{DateTime, Local};

use super::monitor::{ApiStatus, HealthState, Sample, ServiceHealth};
use super::status::StatusSnapshot;
use super::usage::UsageSummary;
use super::wizard::Wizard;

/// Quantos pontos os mini-gráficos do painel guardam (um por ciclo de coleta).
pub const HISTORY_LEN: usize = 30;

/// O que a TUI pede para o launcher (`./oreia`) fazer depois que ela fecha —
/// subir/parar containers e rodar o bootstrap precisam do `docker compose`
/// de verdade, no host, que a TUI nem sempre tem (ver `super::status`). A
/// TUI grava o nome da ação no arquivo de `OREIA_ACTION_FILE`, sai, o
/// launcher executa e reabre o painel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherAction {
    Up,
    Down,
    Restart,
    Bootstrap,
    Logs,
    /// Fim do assistente de primeira instalação: bootstrap + up.
    Install,
    /// Configuração salva: o launcher pergunta se aplica agora.
    Saved,
}

impl LauncherAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            LauncherAction::Up => "up",
            LauncherAction::Down => "down",
            LauncherAction::Restart => "restart",
            LauncherAction::Bootstrap => "bootstrap",
            LauncherAction::Logs => "logs",
            LauncherAction::Install => "install",
            LauncherAction::Saved => "saved",
        }
    }

    /// Comando equivalente no terminal — mostrado quando a TUI foi aberta sem o launcher.
    pub fn manual_command(&self) -> &'static str {
        match self {
            LauncherAction::Up => "./oreia up",
            LauncherAction::Down => "./oreia down",
            LauncherAction::Restart => "./oreia restart",
            LauncherAction::Bootstrap => "./oreia bootstrap",
            LauncherAction::Logs => "./oreia logs",
            LauncherAction::Install => "./oreia install",
            LauncherAction::Saved => "./oreia restart",
        }
    }
}

/// Dados ao vivo das áreas Status/Saúde/Uso, acumulados a cada [`Sample`].
#[derive(Debug, Clone, Default)]
pub struct MonitorView {
    pub api: Option<ApiStatus>,
    pub health: Vec<ServiceHealth>,
    /// Por serviço: latência de cada checagem (`None` = fora do ar naquela vez).
    pub health_history: BTreeMap<String, VecDeque<Option<u64>>>,
    /// Por serviço: (checagens ok, checagens feitas) desde que a TUI abriu.
    pub health_counts: BTreeMap<String, (u32, u32)>,
    pub cpu_history: BTreeMap<String, VecDeque<f64>>,
    pub mem_history: BTreeMap<String, VecDeque<f64>>,
    pub usage: UsageSummary,
    pub updated_at: Option<DateTime<Local>>,
    pub took_ms: u64,
}

fn push_capped<T>(queue: &mut VecDeque<T>, value: T) {
    if queue.len() >= HISTORY_LEN {
        queue.pop_front();
    }
    queue.push_back(value);
}

impl MonitorView {
    /// Disponibilidade do serviço desde que a TUI abriu, em %.
    pub fn uptime_pct(&self, id: &str) -> Option<f64> {
        let (ok, total) = *self.health_counts.get(id)?;
        (total > 0).then(|| f64::from(ok) * 100.0 / f64::from(total))
    }

    pub fn health_of(&self, id: &str) -> Option<&ServiceHealth> {
        self.health.iter().find(|h| h.id == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaKind {
    Status,
    Health,
    Usage,
    Px0,
    Forgejo,
    Mcp,
    Telegram,
    Llm,
    Executor,
    Server,
}

impl AreaKind {
    pub const ALL: [AreaKind; 10] = [
        AreaKind::Status,
        AreaKind::Health,
        AreaKind::Usage,
        AreaKind::Px0,
        AreaKind::Forgejo,
        AreaKind::Mcp,
        AreaKind::Telegram,
        AreaKind::Llm,
        AreaKind::Executor,
        AreaKind::Server,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            AreaKind::Status => "Status",
            AreaKind::Health => "Saúde",
            AreaKind::Usage => "Uso",
            AreaKind::Px0 => "px0",
            AreaKind::Forgejo => "Forgejo",
            AreaKind::Mcp => "MCP",
            AreaKind::Telegram => "Telegram",
            AreaKind::Llm => "LLM",
            AreaKind::Executor => "Executor",
            AreaKind::Server => "Servidor",
        }
    }

    /// Áreas só de leitura, alimentadas pelo monitor em segundo plano.
    pub fn is_monitor(&self) -> bool {
        matches!(self, AreaKind::Status | AreaKind::Health | AreaKind::Usage)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    AreaList,
    Field(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Abertura: o nome do projeto e o mascote levantando a parede
    /// (`super::mascot::draw_splash`). Qualquer tecla — ou o fim da animação —
    /// leva para o painel (ou para o assistente, na primeira instalação).
    Splash,
    Editing,
    Summary,
    /// Assistente de primeira instalação (`App::wizard` é `Some`).
    Wizard,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextField {
    pub value: String,
}

impl TextField {
    fn from(value: impl Into<String>) -> Self {
        Self { value: value.into() }
    }

    pub fn push(&mut self, c: char) {
        self.value.push(c);
    }

    pub fn backspace(&mut self) {
        self.value.pop();
    }
}

#[derive(Debug, Clone)]
pub struct TelegramFields {
    pub enabled: bool,
    /// Sempre começa vazio — campo de segredo, só troca `TELEGRAM_TOKEN` se
    /// algo for digitado.
    pub token: TextField,
    pub token_was_set: bool,
    pub allowed: TextField,
}

#[derive(Debug, Clone)]
pub struct McpFields {
    pub enabled: bool,
    pub http: bool,
    pub token_was_set: bool,
    pub agent_user: TextField,
    pub workdir: TextField,
    pub max_review_rounds: TextField,
    pub bind: TextField,
}

#[derive(Debug, Clone)]
pub struct LlmFields {
    pub url: TextField,
    pub coder_model: TextField,
    pub chat_model: TextField,
    /// Sempre começa vazio — campo de segredo, só troca `LLM_API_KEY` se
    /// algo for digitado.
    pub api_key: TextField,
    pub api_key_was_set: bool,
    pub coder_context: TextField,
    pub coder_temperature: TextField,
    pub chat_context: TextField,
    pub chat_temperature: TextField,
}

#[derive(Debug, Clone)]
pub struct Px0Fields {
    pub allowed_users: TextField,
    pub forgejo_token_set: bool,
    pub oauth_client_id_set: bool,
    pub oauth_client_secret_set: bool,
}

#[derive(Debug, Clone)]
pub struct ForgejoFields {
    pub url: TextField,
    pub public_url: TextField,
    /// Texto separado por vírgula; vira `Vec<String>` em `build_save_payload`.
    pub repos: TextField,
    pub base_branch: TextField,
    pub merge_style: TextField,
    pub owner_token_set: bool,
    pub webhook_secret_set: bool,
}

#[derive(Debug, Clone)]
pub struct ServerFields {
    pub host: TextField,
    pub port: TextField,
    pub paths_data: TextField,
    pub paths_workspace: TextField,
    pub queue_concurrency: TextField,
    pub queue_timeout_secs: TextField,
    pub api_key_set: bool,
}

#[derive(Debug, Clone)]
pub struct ExecutorFields {
    pub default: TextField,
    pub max_steps: TextField,
    pub phase_retries: TextField,
    pub shell_timeout_secs: TextField,
    pub max_phases: TextField,
    pub step_timeout_secs: TextField,
}

pub struct App {
    pub selected_area: usize,
    pub focus: Focus,
    pub screen: Screen,
    pub telegram: TelegramFields,
    pub mcp: McpFields,
    pub llm: LlmFields,
    pub px0: Px0Fields,
    pub forgejo: ForgejoFields,
    pub executor: ExecutorFields,
    pub server: ServerFields,
    pub llm_models: Vec<String>,
    pub llm_fetch_status: Option<String>,
    pub status: StatusSnapshot,
    pub monitor: MonitorView,
    pub wizard: Option<Wizard>,
    /// Aviso de uma linha no rodapé; some na próxima tecla.
    pub flash: Option<String>,
    /// A TUI foi aberta pelo launcher (`OREIA_ACTION_FILE` definido): as
    /// teclas de subir/parar/bootstrap funcionam de verdade.
    pub actions_enabled: bool,
    /// Milissegundos desde que a TUI abriu — o relógio das animações do
    /// mascote. O loop de eventos atualiza; os testes escolhem o quadro.
    pub anim_ms: u64,
    /// Retrato do que seria gravado logo ao abrir — para saber se há edição pendente.
    baseline: String,
}

impl App {
    pub fn new(state: &CurrentState) -> Self {
        let mut app = Self::build(state);
        app.baseline = app.fingerprint();
        app
    }

    /// Sai da abertura: para o assistente, se ele estiver esperando, ou para o painel.
    pub fn leave_splash(&mut self) {
        if self.screen == Screen::Splash {
            self.screen = if self.wizard.is_some() { Screen::Wizard } else { Screen::Editing };
        }
    }

    fn fingerprint(&self) -> String {
        format!("{:?}", self.build_save_payload())
    }

    /// Algum campo das áreas de configuração foi editado e ainda não foi salvo?
    pub fn is_dirty(&self) -> bool {
        self.fingerprint() != self.baseline
    }

    /// Incorpora um ciclo de coleta do monitor (ver `super::monitor`).
    pub fn absorb(&mut self, sample: Sample) {
        for c in &sample.snapshot.containers {
            if let Some(cpu) = c.cpu_pct {
                push_capped(self.monitor.cpu_history.entry(c.service.clone()).or_default(), cpu);
            }
            if let Some(mem) = c.mem_pct {
                push_capped(self.monitor.mem_history.entry(c.service.clone()).or_default(), mem);
            }
        }
        if let Some(health) = sample.health {
            for h in &health {
                // "Desligado de propósito" não entra na conta de disponibilidade.
                if h.state == HealthState::Off {
                    continue;
                }
                let point = (h.state != HealthState::Down).then(|| h.latency_ms.unwrap_or(0));
                push_capped(self.monitor.health_history.entry(h.id.clone()).or_default(), point);
                let counts = self.monitor.health_counts.entry(h.id.clone()).or_insert((0, 0));
                counts.0 += u32::from(h.state == HealthState::Up);
                counts.1 += 1;
            }
            self.monitor.health = health;
        }
        self.status = sample.snapshot;
        self.monitor.api = sample.api;
        self.monitor.usage = sample.usage;
        self.monitor.updated_at = Some(sample.at);
        self.monitor.took_ms = sample.took_ms;
    }

    fn build(state: &CurrentState) -> Self {
        Self {
            selected_area: 0,
            focus: Focus::AreaList,
            screen: Screen::Editing,

            telegram: TelegramFields {
                enabled: state.telegram_enabled,
                token: TextField::default(),
                token_was_set: state.telegram_token_set,
                allowed: TextField::from(state.telegram_allowed.clone()),
            },
            mcp: McpFields {
                enabled: state.mcp_block_present,
                http: state.mcp_http,
                token_was_set: state.mcp_token_set,
                agent_user: TextField::from(state.mcp_agent_user.clone()),
                workdir: TextField::from(state.mcp_workdir.clone()),
                max_review_rounds: TextField::from(state.mcp_max_review_rounds.clone()),
                bind: TextField::from(state.mcp_bind.clone()),
            },
            llm: LlmFields {
                url: TextField::from(state.llm_url.clone()),
                coder_model: TextField::from(state.llm_coder_model.clone()),
                chat_model: TextField::from(state.llm_chat_model.clone()),
                api_key: TextField::default(),
                api_key_was_set: state.llm_api_key_set,
                coder_context: TextField::from(state.models_coder_context.clone()),
                coder_temperature: TextField::from(state.models_coder_temperature.clone()),
                chat_context: TextField::from(state.models_chat_context.clone()),
                chat_temperature: TextField::from(state.models_chat_temperature.clone()),
            },
            px0: Px0Fields {
                allowed_users: TextField::from(state.px0_allowed_users.clone()),
                forgejo_token_set: state.px0_forgejo_token_set,
                oauth_client_id_set: state.px0_oauth_client_id_set,
                oauth_client_secret_set: state.px0_oauth_client_secret_set,
            },
            forgejo: ForgejoFields {
                url: TextField::from(state.forgejo_url.clone()),
                public_url: TextField::from(state.forgejo_public_url.clone()),
                repos: TextField::from(state.forgejo_repos.clone()),
                base_branch: TextField::from(state.forgejo_base_branch.clone()),
                merge_style: TextField::from(state.forgejo_merge_style.clone()),
                owner_token_set: state.forgejo_owner_token_set,
                webhook_secret_set: state.forgejo_webhook_secret_set,
            },
            executor: ExecutorFields {
                default: TextField::from(state.executor_default.clone()),
                max_steps: TextField::from(state.executor_max_steps.clone()),
                phase_retries: TextField::from(state.executor_phase_retries.clone()),
                shell_timeout_secs: TextField::from(state.executor_shell_timeout_secs.clone()),
                max_phases: TextField::from(state.executor_max_phases.clone()),
                step_timeout_secs: TextField::from(state.executor_step_timeout_secs.clone()),
            },
            server: ServerFields {
                host: TextField::from(state.server_host.clone()),
                port: TextField::from(state.server_port.clone()),
                paths_data: TextField::from(state.paths_data.clone()),
                paths_workspace: TextField::from(state.paths_workspace.clone()),
                queue_concurrency: TextField::from(state.queue_concurrency.clone()),
                queue_timeout_secs: TextField::from(state.queue_timeout_secs.clone()),
                api_key_set: state.rest_api_key_set,
            },
            llm_models: Vec::new(),
            llm_fetch_status: None,
            status: StatusSnapshot::default(),
            monitor: MonitorView::default(),
            wizard: None,
            flash: None,
            actions_enabled: false,
            anim_ms: 0,
            baseline: String::new(),
        }
    }

    pub fn selected_area_kind(&self) -> AreaKind {
        AreaKind::ALL[self.selected_area]
    }

    /// Quantos campos navegáveis (`Tab`) a área tem — usado pela navegação
    /// em `event::handle_key` e pelo painel em `ui::draw`. `Status` não tem
    /// nenhum: é só leitura, atualiza sozinha.
    pub fn field_count(area: AreaKind) -> usize {
        match area {
            AreaKind::Status | AreaKind::Health | AreaKind::Usage => 0,
            AreaKind::Px0 => 1,
            AreaKind::Forgejo => 5,
            AreaKind::Mcp => 6,
            AreaKind::Telegram => 3,
            AreaKind::Llm => 8,
            AreaKind::Executor => 6,
            AreaKind::Server => 6,
        }
    }

    /// Converte o estado atual da TUI no par que `setup::apply_answers` e
    /// `setup::toml_edits::apply` sabem gravar. Campos de texto vazios nunca
    /// tocam o que já está no arquivo (mesma regra em todo lugar: vazio =
    /// não muda); campos numéricos vazios idem, e um valor não-vazio que não
    /// parseia devolve `Err` com uma mensagem pronta pra mostrar na tela de
    /// resumo — nada é gravado nesse caso.
    pub fn build_save_payload(&self) -> Result<(Answers, TomlEdits), String> {
        let mcp_http = self.mcp.enabled && self.mcp.http;
        let answers = Answers {
            telegram: TelegramAnswers {
                token: self.telegram.token.value.clone(),
                allowed: self.telegram.allowed.value.clone(),
            },
            llm: LlmAnswers {
                url: self.llm.url.value.clone(),
                model: self.llm.coder_model.value.clone(),
                chat_model: self.llm.chat_model.value.clone(),
                api_key: self.llm.api_key.value.clone(),
            },
            forgejo: ForgejoAnswers::default(),
            mcp: McpAnswers { enabled: Some(self.mcp.enabled), http: Some(mcp_http) },
            px0: Px0Answers { allowed_users: self.px0.allowed_users.value.clone() },
            ports: Default::default(),
        };

        let mcp_max_review_rounds =
            parse_field::<usize>("MCP → rodadas de revisão", &self.mcp.max_review_rounds.value)?;
        let server_port = parse_field::<u16>("Servidor → porta", &self.server.port.value)?;
        let queue_concurrency =
            parse_field::<usize>("Servidor → fila: concorrência", &self.server.queue_concurrency.value)?;
        let queue_timeout_secs = parse_field::<u64>("Servidor → fila: timeout", &self.server.queue_timeout_secs.value)?;
        let models_coder_context = parse_field::<u32>("LLM → contexto (código)", &self.llm.coder_context.value)?;
        let models_coder_temperature =
            parse_field::<f64>("LLM → temperatura (código)", &self.llm.coder_temperature.value)?;
        let models_chat_context = parse_field::<u32>("LLM → contexto (chat)", &self.llm.chat_context.value)?;
        let models_chat_temperature = parse_field::<f64>("LLM → temperatura (chat)", &self.llm.chat_temperature.value)?;

        let executor_max_steps = parse_field::<usize>("Executor → rodadas por fase", &self.executor.max_steps.value)?;
        let executor_phase_retries =
            parse_field::<usize>("Executor → retentativas no degrau", &self.executor.phase_retries.value)?;
        let executor_shell_timeout_secs =
            parse_field::<u64>("Executor → timeout shell (s)", &self.executor.shell_timeout_secs.value)?;
        let executor_max_phases = parse_field::<usize>("Executor → máximo de fases", &self.executor.max_phases.value)?;
        let executor_step_timeout_secs =
            parse_field::<u64>("Executor → timeout degrau (s)", &self.executor.step_timeout_secs.value)?;

        let toml_edits = TomlEdits {
            telegram_enabled: Some(self.telegram.enabled),
            mcp_enabled: Some(self.mcp.enabled),
            mcp_http: self.mcp.enabled.then_some(self.mcp.http),
            mcp_agent_user: non_empty(&self.mcp.agent_user.value),
            mcp_workdir: non_empty(&self.mcp.workdir.value),
            mcp_max_review_rounds,
            mcp_bind: non_empty(&self.mcp.bind.value),

            forgejo_url: non_empty(&self.forgejo.url.value),
            forgejo_public_url: non_empty(&self.forgejo.public_url.value),
            forgejo_repos: non_empty_list(&self.forgejo.repos.value),
            forgejo_base_branch: non_empty(&self.forgejo.base_branch.value),
            forgejo_merge_style: non_empty(&self.forgejo.merge_style.value),

            server_host: non_empty(&self.server.host.value),
            server_port,
            paths_data: non_empty(&self.server.paths_data.value),
            paths_workspace: non_empty(&self.server.paths_workspace.value),
            queue_concurrency,
            queue_timeout_secs,

            executor_default: non_empty(&self.executor.default.value),
            executor_max_steps,
            executor_phase_retries,
            executor_shell_timeout_secs,
            executor_max_phases,
            executor_step_timeout_secs,

            models_coder_context,
            models_coder_temperature,
            models_chat_context,
            models_chat_temperature,
        };
        Ok((answers, toml_edits))
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn non_empty_list(s: &str) -> Option<Vec<String>> {
    let items: Vec<String> = s.split(',').map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).collect();
    (!items.is_empty()).then_some(items)
}

fn parse_field<T: std::str::FromStr>(label: &str, text: &str) -> Result<Option<T>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed.parse::<T>().map(Some).map_err(|_| format!("{label}: \"{trimmed}\" não é um valor válido"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_state() -> CurrentState {
        CurrentState::default()
    }

    #[test]
    fn new_app_preloads_fields_from_current_state_without_exposing_secrets() {
        let mut state = blank_state();
        state.telegram_enabled = false;
        state.telegram_token_set = true;
        state.telegram_allowed = "1,2".into();
        state.llm_url = "http://x/v1".into();
        state.llm_api_key_set = true;
        state.px0_allowed_users = "dono".into();
        state.forgejo_url = "http://forgejo:3000".into();
        state.server_port = "3120".into();

        let app = App::new(&state);
        assert!(!app.telegram.enabled);
        assert!(app.telegram.token_was_set);
        assert_eq!(app.telegram.token.value, "", "campo de segredo deve começar vazio");
        assert_eq!(app.telegram.allowed.value, "1,2");
        assert_eq!(app.llm.url.value, "http://x/v1");
        assert!(app.llm.api_key_was_set);
        assert_eq!(app.llm.api_key.value, "");
        assert_eq!(app.px0.allowed_users.value, "dono");
        assert_eq!(app.forgejo.url.value, "http://forgejo:3000");
        assert_eq!(app.server.port.value, "3120");
    }

    #[test]
    fn build_save_payload_leaves_untouched_secrets_and_blank_fields_empty() {
        let app = App::new(&blank_state());
        let (answers, edits) = app.build_save_payload().unwrap();
        assert_eq!(answers.telegram.token, "");
        assert_eq!(answers.llm.api_key, "");
        assert_eq!(edits.forgejo_url, None);
        assert_eq!(edits.mcp_agent_user, None);
        assert_eq!(edits.models_coder_context, None);
    }

    #[test]
    fn build_save_payload_reflects_mcp_enabled_and_http() {
        let mut app = App::new(&blank_state());
        app.mcp.enabled = true;
        app.mcp.http = true;

        let (answers, edits) = app.build_save_payload().unwrap();
        assert_eq!(answers.mcp.enabled, Some(true));
        assert_eq!(answers.mcp.http, Some(true));
        assert_eq!(edits.mcp_enabled, Some(true));
        assert_eq!(edits.mcp_http, Some(true));
    }

    #[test]
    fn mcp_http_is_forced_off_when_mcp_is_disabled() {
        let mut app = App::new(&blank_state());
        app.mcp.enabled = false;
        app.mcp.http = true; // valor "fantasma" de quando estava ligado

        let (answers, edits) = app.build_save_payload().unwrap();
        assert_eq!(answers.mcp.http, Some(false));
        assert_eq!(edits.mcp_http, None, "sem bloco [mcp], não dá pra setar http");
    }

    #[test]
    fn telegram_enabled_toggle_flows_into_toml_edits() {
        let mut app = App::new(&blank_state());
        app.telegram.enabled = false;
        let (_, edits) = app.build_save_payload().unwrap();
        assert_eq!(edits.telegram_enabled, Some(false));
    }

    #[test]
    fn field_count_matches_each_area_panel() {
        assert_eq!(App::field_count(AreaKind::Status), 0);
        assert_eq!(App::field_count(AreaKind::Health), 0);
        assert_eq!(App::field_count(AreaKind::Usage), 0);
        assert_eq!(App::field_count(AreaKind::Px0), 1);
        assert_eq!(App::field_count(AreaKind::Forgejo), 5);
        assert_eq!(App::field_count(AreaKind::Mcp), 6);
        assert_eq!(App::field_count(AreaKind::Telegram), 3);
        assert_eq!(App::field_count(AreaKind::Llm), 8);
        assert_eq!(App::field_count(AreaKind::Executor), 6);
        assert_eq!(App::field_count(AreaKind::Server), 6);
    }

    #[test]
    fn dirty_only_after_an_edit_and_clean_again_when_undone() {
        let mut app = App::new(&blank_state());
        assert!(!app.is_dirty());
        app.px0.allowed_users.push('x');
        assert!(app.is_dirty());
        app.px0.allowed_users.backspace();
        assert!(!app.is_dirty());
    }

    fn sample(health: Option<Vec<ServiceHealth>>, cpu: f64) -> Sample {
        use crate::setup::tui::status::ContainerRow;
        Sample {
            snapshot: StatusSnapshot {
                containers: vec![ContainerRow {
                    service: "forgejo".into(),
                    state: "running".into(),
                    cpu_pct: Some(cpu),
                    mem_pct: Some(1.0),
                    ..Default::default()
                }],
                ..Default::default()
            },
            api: None,
            health,
            usage: UsageSummary::default(),
            at: Local::now(),
            took_ms: 7,
        }
    }

    fn health(id: &str, state: HealthState, latency: Option<u64>) -> ServiceHealth {
        ServiceHealth { id: id.into(), name: id.into(), kind: "x", state, latency_ms: latency, detail: String::new() }
    }

    #[test]
    fn absorb_accumulates_history_and_uptime_and_ignores_services_that_are_off() {
        let mut app = App::new(&blank_state());
        app.absorb(sample(
            Some(vec![health("forgejo", HealthState::Up, Some(12)), health("telegram", HealthState::Off, None)]),
            1.5,
        ));
        app.absorb(sample(Some(vec![health("forgejo", HealthState::Down, None)]), 2.5));
        // Ciclo sem checagem de saúde: mantém a última lista, só atualiza containers.
        app.absorb(sample(None, 3.5));

        let m = &app.monitor;
        assert_eq!(m.cpu_history["forgejo"].iter().copied().collect::<Vec<_>>(), vec![1.5, 2.5, 3.5]);
        assert_eq!(m.health_history["forgejo"].iter().copied().collect::<Vec<_>>(), vec![Some(12), None]);
        assert_eq!(m.uptime_pct("forgejo"), Some(50.0));
        assert_eq!(m.uptime_pct("telegram"), None);
        assert!(!m.health_history.contains_key("telegram"));
        assert_eq!(m.health.len(), 1, "a lista é a da última checagem que rodou");
        assert_eq!(m.took_ms, 7);
        assert!(app.status.any_running());
    }

    #[test]
    fn history_is_capped() {
        let mut app = App::new(&blank_state());
        for i in 0..(HISTORY_LEN + 5) {
            app.absorb(sample(None, i as f64));
        }
        let cpu = &app.monitor.cpu_history["forgejo"];
        assert_eq!(cpu.len(), HISTORY_LEN);
        assert_eq!(cpu.front().copied(), Some(5.0));
    }

    #[test]
    fn forgejo_repos_text_becomes_a_trimmed_list() {
        let mut app = App::new(&blank_state());
        app.forgejo.repos = TextField::from(" acme/app ,acme/site ,, ");
        let (_, edits) = app.build_save_payload().unwrap();
        assert_eq!(edits.forgejo_repos, Some(vec!["acme/app".to_string(), "acme/site".to_string()]));
    }

    #[test]
    fn invalid_numeric_field_fails_with_a_clear_message_before_writing_anything() {
        let mut app = App::new(&blank_state());
        app.server.port = TextField::from("abcd");
        let err = app.build_save_payload().unwrap_err();
        assert!(err.contains("porta"), "{err}");
        assert!(err.contains("abcd"), "{err}");

        let mut app2 = App::new(&blank_state());
        app2.executor.max_steps = TextField::from("xyz");
        let err2 = app2.build_save_payload().unwrap_err();
        assert!(err2.contains("Executor"), "{err2}");
        assert!(err2.contains("xyz"), "{err2}");
    }

    #[test]
    fn valid_numeric_fields_parse_into_the_right_types() {
        let mut app = App::new(&blank_state());
        app.server.port = TextField::from("4000");
        app.mcp.max_review_rounds = TextField::from("5");
        app.llm.coder_temperature = TextField::from("0.3");
        app.executor.default = TextField::from("opencode");
        app.executor.max_steps = TextField::from("6");
        app.executor.phase_retries = TextField::from("3");
        app.executor.shell_timeout_secs = TextField::from("150");
        app.executor.max_phases = TextField::from("12");
        app.executor.step_timeout_secs = TextField::from("450");

        let (_, edits) = app.build_save_payload().unwrap();
        assert_eq!(edits.server_port, Some(4000));
        assert_eq!(edits.mcp_max_review_rounds, Some(5));
        assert_eq!(edits.models_coder_temperature, Some(0.3));
        assert_eq!(edits.executor_default, Some("opencode".to_string()));
        assert_eq!(edits.executor_max_steps, Some(6));
        assert_eq!(edits.executor_phase_retries, Some(3));
        assert_eq!(edits.executor_shell_timeout_secs, Some(150));
        assert_eq!(edits.executor_max_phases, Some(12));
        assert_eq!(edits.executor_step_timeout_secs, Some(450));
    }
}
