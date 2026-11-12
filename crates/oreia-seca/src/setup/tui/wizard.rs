//! Assistente de primeira instalação: em vez das ~40 opções das áreas de
//! configuração, cinco perguntas em sequência — de onde vem o modelo de IA,
//! qual modelo, qual repositório, Telegram (opcional) e uma revisão. Tudo o
//! mais fica no padrão e pode ser ajustado depois nas áreas normais.
//!
//! Abre sozinho quando ainda não há nada configurado (ver [`is_first_run`]),
//! ou com `oreia-seca setup --wizard`. No final devolve o mesmo par
//! (`Answers`, `TomlEdits`) que o resto do setup grava — o assistente só
//! decide como PERGUNTAR, nunca como aplicar.
//!
//! [`handle_key`] é pura (não lê terminal nem rede): o que precisa de rede
//! (listar modelos) ou do sistema (portas ocupadas) volta como
//! [`WizardOutcome`] para `super::mod` executar e devolver o resultado.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::setup::current_state::CurrentState;
use crate::setup::toml_edits::TomlEdits;
use crate::setup::{
    parse_repos, validate_owner_user, Answers, ForgejoAnswers, LlmAnswers, PortsAnswers, Px0Answers, TelegramAnswers,
};

use super::app::TextField;
use super::monitor::EnvMap;

pub struct Provider {
    pub label: &'static str,
    pub hint: &'static str,
    /// Pré-preenche o campo de URL; `<ip>` precisa ser trocado antes de seguir.
    pub url: &'static str,
    pub needs_key: bool,
    /// "Configurar depois": pula as telas de conexão e de modelo.
    pub skip: bool,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        label: "Ollama neste computador",
        hint: "Gratuito, roda na sua máquina. Deixe o Ollama aberto com um modelo já baixado (ex.: ollama pull qwen2.5-coder).",
        url: "http://host.docker.internal:11434/v1",
        needs_key: false,
        skip: false,
    },
    Provider {
        label: "LM Studio neste computador",
        hint: "Gratuito, roda na sua máquina. No LM Studio, aba Developer: carregue um modelo e ligue o servidor local.",
        url: "http://host.docker.internal:1234/v1",
        needs_key: false,
        skip: false,
    },
    Provider {
        label: "Ollama ou LM Studio em outra máquina da rede",
        hint: "Troque <ip> pelo endereço da máquina (ex.: 192.168.1.50). Porta 11434 = Ollama, 1234 = LM Studio.",
        url: "http://<ip>:11434/v1",
        needs_key: false,
        skip: false,
    },
    Provider {
        label: "OpenRouter",
        hint: "Pago por uso, centenas de modelos numa chave só. Crie a chave em https://openrouter.ai/keys",
        url: "https://openrouter.ai/api/v1",
        needs_key: true,
        skip: false,
    },
    Provider {
        label: "Google Gemini",
        hint: "Crie a chave em https://aistudio.google.com/apikey",
        url: "https://generativelanguage.googleapis.com/v1beta/openai",
        needs_key: true,
        skip: false,
    },
    Provider {
        label: "OpenAI",
        hint: "Crie a chave em https://platform.openai.com/api-keys",
        url: "https://api.openai.com/v1",
        needs_key: true,
        skip: false,
    },
    Provider {
        label: "Outro servidor compatível com OpenAI",
        hint: "Qualquer servidor que responda em /v1/chat/completions (llama.cpp, vLLM, LiteLLM...).",
        url: "",
        needs_key: false,
        skip: false,
    },
    Provider {
        label: "Configurar depois",
        hint: "Instala tudo agora. Os agentes só começam a trabalhar depois que você escolher um modelo na área LLM.",
        url: "",
        needs_key: false,
        skip: true,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Provider,
    Connection,
    Model,
    Project,
    Telegram,
    Review,
}

impl Step {
    /// Posição na barra de progresso (`Welcome` não conta como passo).
    fn number(self) -> usize {
        match self {
            Step::Welcome => 0,
            Step::Provider | Step::Connection => 1,
            Step::Model => 2,
            Step::Project => 3,
            Step::Telegram => 4,
            Step::Review => 5,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Step::Welcome => "Bem-vindo",
            Step::Provider => "De onde vem o modelo de IA?",
            Step::Connection => "Conexão com o servidor do modelo",
            Step::Model => "Qual modelo os agentes vão usar?",
            Step::Project => "Seu projeto no Forgejo",
            Step::Telegram => "Telegram (opcional)",
            Step::Review => "Revisão",
        }
    }
}

const TOTAL_STEPS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortPlan {
    pub label: &'static str,
    pub var: &'static str,
    pub wanted: u16,
    pub chosen: u16,
}

impl PortPlan {
    pub fn changed(&self) -> bool {
        self.wanted != self.chosen
    }
}

/// Portas que o projeto publica no host, com o padrão de cada uma.
pub const PORT_SPECS: &[(&str, &str, u16)] = &[
    ("Forgejo (site)", "FORGEJO_HTTP_PORT", 3000),
    ("Forgejo (git por SSH)", "FORGEJO_SSH_PORT", 2222),
    ("API do OREIA-SECA", "OREIA_SECA_PORT", 3120),
    ("px0 (revisão de PRs)", "PX0_PORT", 7777),
];

/// Para cada porta desejada, a primeira livre a partir dela (sem repetir uma
/// já escolhida para outro serviço). Sem nenhuma livre nas 50 seguintes,
/// mantém a desejada — o `up` avisa depois.
pub fn plan_ports(wanted: &[(&'static str, &'static str, u16)], busy: &dyn Fn(u16) -> bool) -> Vec<PortPlan> {
    let mut taken: Vec<u16> = Vec::new();
    wanted
        .iter()
        .map(|&(label, var, port)| {
            let chosen = (port..=port.saturating_add(50)).find(|p| !taken.contains(p) && !busy(*p)).unwrap_or(port);
            taken.push(chosen);
            PortPlan { label, var, wanted: port, chosen }
        })
        .collect()
}

pub enum WizardOutcome {
    Continue,
    Cancel,
    /// Buscar `/models` em `url` e devolver por [`Wizard::on_models`].
    FetchModels {
        url: String,
        api_key: Option<String>,
    },
    /// Conferir portas ocupadas e devolver por [`Wizard::on_ports`].
    PlanPorts,
    /// Gravar ([`Wizard::payload`]) e seguir para a instalação.
    Finish,
}

pub struct Wizard {
    pub step: Step,
    pub provider: usize,
    /// Campo em foco dentro do passo atual (0-based).
    pub field: usize,
    pub url: TextField,
    pub api_key: TextField,
    pub api_key_was_set: bool,
    pub llm_skipped: bool,
    pub models: Vec<String>,
    /// Texto digitado na tela de modelo: filtra a lista, ou é o próprio nome
    /// do modelo quando nada na lista combina (ou quando não há lista).
    pub model_filter: TextField,
    pub model_idx: usize,
    pub fetch_status: Option<String>,
    pub repo: TextField,
    pub owner: TextField,
    /// O bootstrap já criou a conta do dono — não dá mais para trocar o nome.
    pub owner_locked: bool,
    pub tg_token: TextField,
    pub tg_token_was_set: bool,
    pub tg_allowed: TextField,
    /// Já havia alguém em `PX0_ALLOWED_USERS` — não sobrescreve com o dono.
    px0_already_allowed: bool,
    pub current_ports: Vec<(&'static str, &'static str, u16)>,
    pub ports: Vec<PortPlan>,
    pub error: Option<String>,
}

/// Primeira instalação = ainda não existe `.env`, ou existe mas sem LLM e sem
/// bootstrap (alguém só copiou o `.env.example`).
pub fn is_first_run(env_exists: bool, env: &EnvMap) -> bool {
    !env_exists || (env.get("FORGEJO_OWNER_TOKEN").is_none() && env.get("LLM_CODER_URL").is_none())
}

fn text(value: &str) -> TextField {
    TextField { value: value.to_string() }
}

impl Wizard {
    pub fn new(state: &CurrentState, env: &EnvMap) -> Self {
        let repo =
            if state.forgejo_repos.trim().is_empty() { "minha-org/meu-sistema" } else { state.forgejo_repos.trim() };
        // Reabrindo o assistente com um LLM já configurado: começa no provedor que combina.
        let provider = PROVIDERS.iter().position(|p| !p.url.is_empty() && p.url == state.llm_url).unwrap_or(0);
        let current_ports =
            PORT_SPECS.iter().map(|&(l, v, d)| (l, v, env.get(v).and_then(|p| p.parse().ok()).unwrap_or(d))).collect();
        Self {
            step: Step::Welcome,
            provider,
            field: 0,
            url: text(&state.llm_url),
            api_key: TextField::default(),
            api_key_was_set: state.llm_api_key_set,
            llm_skipped: false,
            models: Vec::new(),
            model_filter: text(&state.llm_coder_model),
            model_idx: 0,
            fetch_status: None,
            repo: text(repo),
            owner: text(env.get("FORGEJO_OWNER_USER").unwrap_or("dono")),
            owner_locked: state.forgejo_owner_token_set,
            tg_token: TextField::default(),
            tg_token_was_set: state.telegram_token_set,
            tg_allowed: text(&state.telegram_allowed),
            px0_already_allowed: !state.px0_allowed_users.trim().is_empty(),
            current_ports,
            ports: Vec::new(),
            error: None,
        }
    }

    pub fn provider(&self) -> &'static Provider {
        &PROVIDERS[self.provider]
    }

    /// Modelos da lista que combinam com o que foi digitado (sem diferenciar maiúsculas).
    pub fn visible_models(&self) -> Vec<&str> {
        let needle = self.model_filter.value.trim().to_lowercase();
        self.models
            .iter()
            .map(String::as_str)
            .filter(|m| needle.is_empty() || m.to_lowercase().contains(&needle))
            .collect()
    }

    /// O modelo que vale agora: o destacado na lista, ou o texto digitado se
    /// nada combina (modelo que o servidor não lista, ou servidor fora do ar).
    pub fn chosen_model(&self) -> String {
        let visible = self.visible_models();
        match visible.get(self.model_idx.min(visible.len().saturating_sub(1))) {
            Some(m) => (*m).to_string(),
            None => self.model_filter.value.trim().to_string(),
        }
    }

    /// Resultado da busca de modelos pedida por [`WizardOutcome::FetchModels`].
    pub fn on_models(&mut self, result: Result<Vec<String>, String>) {
        match result {
            Ok(models) if !models.is_empty() => {
                self.fetch_status = Some(format!("O servidor respondeu com {} modelo(s).", models.len()));
                // Mantém o modelo já configurado selecionado, se ele estiver na lista.
                let previous = self.model_filter.value.trim().to_string();
                self.model_idx = models.iter().position(|m| *m == previous).unwrap_or(0);
                self.model_filter.value.clear();
                self.models = models;
            }
            Ok(_) => {
                self.models.clear();
                self.fetch_status =
                    Some("O servidor respondeu, mas não listou nenhum modelo — digite o nome do modelo abaixo.".into());
            }
            Err(e) => {
                self.models.clear();
                self.fetch_status = Some(format!(
                    "Não consegui falar com o servidor ({e}). Esc volta para corrigir o endereço — ou digite o nome do \
                     modelo e siga mesmo assim (dá para testar depois na área Saúde)."
                ));
            }
        }
        self.step = Step::Model;
        self.field = 0;
        self.error = None;
    }

    /// Resultado da conferência de portas pedida por [`WizardOutcome::PlanPorts`].
    pub fn on_ports(&mut self, ports: Vec<PortPlan>) {
        self.ports = ports;
        self.step = Step::Review;
        self.field = 0;
        self.error = None;
    }

    fn go(&mut self, step: Step) -> WizardOutcome {
        self.step = step;
        self.field = 0;
        self.error = None;
        WizardOutcome::Continue
    }

    fn fail(&mut self, message: impl Into<String>) -> WizardOutcome {
        self.error = Some(message.into());
        WizardOutcome::Continue
    }

    fn previous_step(&self) -> Option<Step> {
        Some(match self.step {
            Step::Welcome => return None,
            Step::Provider => Step::Welcome,
            Step::Connection => Step::Provider,
            Step::Model => Step::Connection,
            Step::Project if self.llm_skipped => Step::Provider,
            Step::Project => Step::Model,
            Step::Telegram => Step::Project,
            Step::Review => Step::Telegram,
        })
    }

    fn field_count(&self) -> usize {
        match self.step {
            Step::Connection | Step::Project | Step::Telegram => 2,
            _ => 1,
        }
    }

    fn current_text_field(&mut self) -> Option<&mut TextField> {
        match (self.step, self.field) {
            (Step::Connection, 0) => Some(&mut self.url),
            (Step::Connection, 1) => Some(&mut self.api_key),
            (Step::Model, _) => Some(&mut self.model_filter),
            (Step::Project, 0) => Some(&mut self.repo),
            (Step::Project, 1) if !self.owner_locked => Some(&mut self.owner),
            (Step::Telegram, 0) => Some(&mut self.tg_token),
            (Step::Telegram, 1) => Some(&mut self.tg_allowed),
            _ => None,
        }
    }

    /// Enter no último campo do passo: valida e avança.
    fn submit_step(&mut self) -> WizardOutcome {
        match self.step {
            Step::Welcome => self.go(Step::Provider),
            Step::Provider => {
                let p = self.provider();
                self.llm_skipped = p.skip;
                if p.skip {
                    return self.go(Step::Project);
                }
                // Só troca a URL se o provedor mudou — voltar e avançar não apaga o que foi digitado.
                let typed_is_a_preset = self.url.value.is_empty() || PROVIDERS.iter().any(|o| o.url == self.url.value);
                if typed_is_a_preset {
                    self.url.value = p.url.to_string();
                }
                self.go(Step::Connection)
            }
            Step::Connection => {
                let url = self.url.value.trim().trim_end_matches('/').to_string();
                if url.is_empty() {
                    return self.fail("Informe o endereço do servidor (ex.: http://192.168.1.50:11434/v1).");
                }
                if url.contains("<ip>") {
                    return self.fail("Troque <ip> pelo endereço da máquina onde o modelo roda (ex.: 192.168.1.50).");
                }
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return self.fail("O endereço precisa começar com http:// ou https://.");
                }
                if self.provider().needs_key && self.api_key.value.trim().is_empty() && !self.api_key_was_set {
                    self.field = 1;
                    return self.fail("Este provedor exige uma chave de API — cole a sua no campo abaixo.");
                }
                self.url.value = url.clone();
                self.error = None;
                let api_key = (!self.api_key.value.trim().is_empty()).then(|| self.api_key.value.trim().to_string());
                self.fetch_status = Some(format!("Buscando os modelos em {url} ..."));
                WizardOutcome::FetchModels { url, api_key }
            }
            Step::Model => {
                if self.chosen_model().is_empty() {
                    return self.fail("Escolha um modelo da lista ou digite o nome dele.");
                }
                self.go(Step::Project)
            }
            Step::Project => {
                match parse_repos(&self.repo.value) {
                    Ok(repos) if repos.is_empty() => {
                        self.field = 0;
                        return self.fail("Informe o repositório no formato dono/repo (ex.: minha-org/meu-sistema).");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        self.field = 0;
                        return self.fail(e);
                    }
                }
                if !self.owner_locked {
                    if let Err(e) = validate_owner_user(self.owner.value.trim()) {
                        self.field = 1;
                        return self.fail(e);
                    }
                }
                self.go(Step::Telegram)
            }
            Step::Telegram => {
                let token = self.tg_token.value.trim();
                if !token.is_empty() && !token.contains(':') {
                    self.field = 0;
                    return self
                        .fail("O token do BotFather tem o formato 123456789:ABC... — confira se copiou inteiro.");
                }
                let allowed = self.tg_allowed.value.trim();
                if allowed.split(',').map(str::trim).any(|id| !id.is_empty() && id.parse::<i64>().is_err()) {
                    self.field = 1;
                    return self.fail("Os IDs são números (ex.: 987654321), separados por vírgula.");
                }
                self.error = None;
                WizardOutcome::PlanPorts
            }
            Step::Review => match self.payload() {
                Ok(_) => WizardOutcome::Finish,
                Err(e) => self.fail(e),
            },
        }
    }

    /// O que gravar. Só o que o assistente perguntou — o resto do
    /// `oreia-seca.toml` (copiado do exemplo) fica como está.
    pub fn payload(&self) -> Result<(Answers, TomlEdits), String> {
        let repos = parse_repos(&self.repo.value)?;
        if repos.is_empty() {
            return Err("nenhum repositório informado".into());
        }
        let owner = self.owner.value.trim().to_string();
        if !self.owner_locked {
            validate_owner_user(&owner)?;
        }

        let llm = if self.llm_skipped {
            LlmAnswers::default()
        } else {
            let model = self.chosen_model();
            if model.is_empty() {
                return Err("nenhum modelo escolhido".into());
            }
            LlmAnswers {
                url: self.url.value.trim().to_string(),
                model,
                chat_model: String::new(),
                api_key: self.api_key.value.trim().to_string(),
            }
        };

        let port_of = |var: &str| self.ports.iter().find(|p| p.var == var && p.changed()).map(|p| p.chosen);
        let ports = PortsAnswers {
            forgejo_http: port_of("FORGEJO_HTTP_PORT"),
            forgejo_ssh: port_of("FORGEJO_SSH_PORT"),
            api: port_of("OREIA_SECA_PORT"),
            px0: port_of("PX0_PORT"),
        };

        let token = self.tg_token.value.trim().to_string();
        let telegram_on = !token.is_empty() || self.tg_token_was_set;
        let answers = Answers {
            telegram: TelegramAnswers { token, allowed: self.tg_allowed.value.trim().to_string() },
            llm,
            forgejo: ForgejoAnswers {
                bootstrap: false,
                owner_user: if self.owner_locked { String::new() } else { owner.clone() },
                repos: repos.join(", "),
            },
            // Sem isso o px0 bloqueia todo mundo depois de instalar — a pegadinha
            // mais comum de uma primeira instalação.
            px0: Px0Answers { allowed_users: if self.px0_already_allowed { String::new() } else { owner } },
            ports: ports.clone(),
            ..Default::default()
        };
        let edits = TomlEdits {
            // Sem token, desliga o canal em vez de deixar um aviso a cada boot.
            telegram_enabled: Some(telegram_on),
            forgejo_repos: Some(repos),
            forgejo_public_url: ports.forgejo_http.map(|p| format!("http://localhost:{p}")),
            ..Default::default()
        };
        Ok((answers, edits))
    }
}

pub fn handle_key(w: &mut Wizard, key: KeyEvent) -> WizardOutcome {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return WizardOutcome::Cancel;
    }
    match key.code {
        KeyCode::Esc => {
            return match w.previous_step() {
                Some(step) => w.go(step),
                None => WizardOutcome::Cancel,
            };
        }
        KeyCode::Enter => {
            if w.field + 1 < w.field_count() {
                w.field += 1;
                return WizardOutcome::Continue;
            }
            return w.submit_step();
        }
        KeyCode::Tab => {
            w.field = (w.field + 1) % w.field_count();
            return WizardOutcome::Continue;
        }
        KeyCode::BackTab => {
            w.field = (w.field + w.field_count() - 1) % w.field_count();
            return WizardOutcome::Continue;
        }
        _ => {}
    }

    match (w.step, key.code) {
        (Step::Provider, KeyCode::Up) => w.provider = (w.provider + PROVIDERS.len() - 1) % PROVIDERS.len(),
        (Step::Provider, KeyCode::Down) => w.provider = (w.provider + 1) % PROVIDERS.len(),
        (Step::Model, KeyCode::Up) => w.model_idx = w.model_idx.saturating_sub(1),
        (Step::Model, KeyCode::Down) => {
            let last = w.visible_models().len().saturating_sub(1);
            w.model_idx = (w.model_idx + 1).min(last);
        }
        (Step::Connection | Step::Project | Step::Telegram, KeyCode::Up) => w.field = w.field.saturating_sub(1),
        (Step::Connection | Step::Project | Step::Telegram, KeyCode::Down) => {
            w.field = (w.field + 1).min(w.field_count() - 1);
        }
        (step, KeyCode::Char(c)) => {
            if let Some(field) = w.current_text_field() {
                field.push(c);
                w.error = None;
                if step == Step::Model {
                    w.model_idx = 0;
                }
            }
        }
        (step, KeyCode::Backspace) => {
            if let Some(field) = w.current_text_field() {
                field.backspace();
                if step == Step::Model {
                    w.model_idx = 0;
                }
            }
        }
        _ => {}
    }
    WizardOutcome::Continue
}

// ── Desenho ──────────────────────────────────────────────────────────────────

const ACCENT: Color = Color::Cyan;

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn plain(s: impl Into<String>) -> Line<'static> {
    Line::from(s.into())
}

fn muted(s: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(s.into(), dim()))
}

fn input_line(label: &str, shown: String, focused: bool) -> Vec<Line<'static>> {
    let marker = if focused { "▸ " } else { "  " };
    let value_style = if focused { bold().fg(Color::White) } else { Style::default() };
    let cursor = if focused { "▏" } else { "" };
    vec![
        Line::from(vec![
            Span::styled(marker.to_string(), Style::default().fg(ACCENT)),
            Span::styled(label.to_string(), Style::default().fg(ACCENT)),
        ]),
        Line::from(vec![
            Span::raw("    "),
            Span::styled(shown, value_style),
            Span::styled(cursor.to_string(), Style::default().fg(ACCENT)),
        ]),
    ]
}

fn secret_shown(typed: &str, was_set: bool, empty_hint: &str) -> String {
    if !typed.is_empty() {
        "•".repeat(typed.chars().count().min(48))
    } else if was_set {
        "(já definido — digite para trocar)".to_string()
    } else {
        empty_hint.to_string()
    }
}

fn progress_line(step: Step) -> Line<'static> {
    let n = step.number();
    let mut spans = Vec::new();
    for i in 1..=TOTAL_STEPS {
        let (mark, style) = if i < n {
            ("●", Style::default().fg(Color::Green))
        } else if i == n {
            ("●", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        } else {
            ("○", dim())
        };
        spans.push(Span::styled(mark, style));
        if i < TOTAL_STEPS {
            spans.push(Span::styled("──", dim()));
        }
    }
    if n > 0 {
        spans.push(Span::styled(format!("   passo {n} de {TOTAL_STEPS}"), dim()));
    }
    Line::from(spans)
}

fn body_lines(w: &Wizard, list_height: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    match w.step {
        Step::Welcome => {
            lines.push(plain("O OREIA-SECA é um time de agentes de IA que pega issues no seu Forgejo, programa num"));
            lines.push(plain("branch, abre o pull request e espera VOCÊ aprovar."));
            lines.push(plain(""));
            lines.push(plain("Este assistente faz 4 perguntas rápidas e depois instala tudo sozinho:"));
            lines.push(plain(""));
            for item in [
                "1. De onde vem o modelo de IA (Ollama, LM Studio, OpenRouter...)",
                "2. Qual modelo usar",
                "3. O nome do seu repositório",
                "4. Telegram, para acompanhar e aprovar pelo celular (opcional)",
            ] {
                lines.push(Line::from(vec![Span::styled("  ▪ ", Style::default().fg(ACCENT)), Span::raw(item)]));
            }
            lines.push(plain(""));
            lines.push(plain(
                "Em seguida ele sobe o Forgejo (seu servidor git particular), cria as contas dos agentes,",
            ));
            lines.push(plain("o repositório e liga tudo. A primeira vez leva alguns minutos (baixar e compilar)."));
            lines.push(plain(""));
            lines.push(muted("Nada aqui é definitivo: tudo pode ser mudado depois, nesta mesma tela."));
        }
        Step::Provider => {
            lines.push(plain("Os agentes precisam de um modelo de linguagem. Ele nunca roda junto com o OREIA-SECA:"));
            lines.push(plain("fica no seu computador, em outra máquina da rede, ou num serviço na nuvem."));
            lines.push(plain(""));
            for (i, p) in PROVIDERS.iter().enumerate() {
                let selected = i == w.provider;
                let marker = if selected { "▸ " } else { "  " };
                let style = if selected { bold().fg(Color::Black).bg(ACCENT) } else { Style::default() };
                lines.push(Line::from(vec![
                    Span::styled(marker.to_string(), Style::default().fg(ACCENT)),
                    Span::styled(format!(" {} ", p.label), style),
                ]));
            }
            lines.push(plain(""));
            lines.push(muted(w.provider().hint));
        }
        Step::Connection => {
            lines.push(muted(w.provider().hint));
            lines.push(plain(""));
            let url_shown = if w.url.value.is_empty() {
                "(ex.: http://192.168.1.50:11434/v1)".to_string()
            } else {
                w.url.value.clone()
            };
            lines.extend(input_line("Endereço do servidor (termina em /v1)", url_shown, w.field == 0));
            lines.push(plain(""));
            let key_label = if w.provider().needs_key {
                "Chave da API"
            } else {
                "Chave da API (deixe vazio se o servidor não pedir)"
            };
            lines.extend(input_line(
                key_label,
                secret_shown(&w.api_key.value, w.api_key_was_set, "(vazio)"),
                w.field == 1,
            ));
            if w.url.value.contains("host.docker.internal") {
                lines.push(plain(""));
                lines.push(muted(
                    "host.docker.internal = \"este computador\", visto de dentro do Docker. Não troque por localhost.",
                ));
            }
        }
        Step::Model => {
            if let Some(status) = &w.fetch_status {
                lines.push(muted(status.clone()));
                lines.push(plain(""));
            }
            let label = if w.models.is_empty() {
                "Nome do modelo"
            } else {
                "Digite para filtrar (ou o nome exato, se não estiver na lista)"
            };
            let shown = if w.model_filter.value.is_empty() { String::new() } else { w.model_filter.value.clone() };
            lines.extend(input_line(label, shown, true));
            lines.push(plain(""));
            let visible = w.visible_models();
            if !visible.is_empty() {
                let idx = w.model_idx.min(visible.len() - 1);
                let window = list_height.max(3);
                let start = idx.saturating_sub(window / 2).min(visible.len().saturating_sub(window));
                for (i, m) in visible.iter().enumerate().skip(start).take(window) {
                    let selected = i == idx;
                    let style = if selected { bold().fg(Color::Black).bg(ACCENT) } else { Style::default() };
                    lines.push(Line::from(vec![
                        Span::styled(if selected { "▸ " } else { "  " }, Style::default().fg(ACCENT)),
                        Span::styled(format!(" {m} "), style),
                    ]));
                }
                lines.push(muted(format!("  {} de {} modelo(s)", visible.len(), w.models.len())));
            } else if !w.models.is_empty() {
                lines.push(muted(format!(
                    "  nada na lista combina — Enter usa \"{}\" assim mesmo",
                    w.model_filter.value.trim()
                )));
            }
            lines.push(plain(""));
            lines.push(muted(
                "Vale para planejar, programar e conversar. Um modelo diferente só para a conversa: área LLM, depois.",
            ));
        }
        Step::Project => {
            lines.push(plain(
                "O Forgejo é o seu servidor git particular (como um GitHub só seu). O assistente cria nele",
            ));
            lines.push(plain(
                "o repositório onde os agentes vão trabalhar e a sua conta de dono — a que aprova os PRs.",
            ));
            lines.push(plain(""));
            lines.extend(input_line("Repositório (dono/repo)", w.repo.value.clone(), w.field == 0));
            lines.push(muted("    vários? separe por vírgula: minha-org/api, minha-org/site"));
            lines.push(plain(""));
            if w.owner_locked {
                lines.push(Line::from(vec![
                    Span::styled("  Seu usuário no Forgejo: ", Style::default().fg(ACCENT)),
                    Span::raw(w.owner.value.clone()),
                ]));
                lines.push(muted("    (conta já criada por uma instalação anterior — não dá para trocar aqui)"));
            } else {
                lines.extend(input_line("Seu usuário no Forgejo", w.owner.value.clone(), w.field == 1));
                lines.push(muted("    a senha é gerada sozinha e aparece no final da instalação"));
            }
        }
        Step::Telegram => {
            lines.push(plain("Com um bot do Telegram você recebe os avisos (\"PR #7 aberto\") e responde do celular"));
            lines
                .push(plain("(/merge 7, /changes 7 ...). Sem ele, tudo continua funcionando pelo Forgejo e pelo px0."));
            lines.push(plain(""));
            lines.push(muted(
                "Para criar: fale com @BotFather no Telegram, mande /newbot e copie o token que ele devolver.",
            ));
            lines.push(plain(""));
            lines.extend(input_line(
                "Token do bot (Enter em branco = pular o Telegram)",
                secret_shown(&w.tg_token.value, w.tg_token_was_set, "(vazio — pular)"),
                w.field == 0,
            ));
            lines.push(plain(""));
            let allowed =
                if w.tg_allowed.value.is_empty() { "(vazio)".to_string() } else { w.tg_allowed.value.clone() };
            lines.extend(input_line("Seu ID numérico do Telegram", allowed, w.field == 1));
            lines.push(muted(
                "    não sabe? deixe vazio: depois de instalar, mande \"oi\" ao bot e ele responde com o seu ID",
            ));
        }
        Step::Review => {
            let row = |label: &str, value: String| {
                Line::from(vec![Span::styled(format!("  {label:<22}"), Style::default().fg(ACCENT)), Span::raw(value)])
            };
            if w.llm_skipped {
                lines.push(row("Modelo de IA", "configurar depois (área LLM)".into()));
            } else {
                lines.push(row("Modelo de IA", w.chosen_model()));
                lines.push(row("Servidor do modelo", w.url.value.clone()));
            }
            lines.push(row("Repositório", w.repo.value.trim().to_string()));
            lines.push(row("Seu usuário (dono)", w.owner.value.trim().to_string()));
            let telegram = if !w.tg_token.value.trim().is_empty() {
                "ligado (token novo)"
            } else if w.tg_token_was_set {
                "ligado (token já existente)"
            } else {
                "pulado — dá para ligar depois"
            };
            lines.push(row("Telegram", telegram.into()));
            lines.push(plain(""));
            lines.push(Line::from(Span::styled("  Endereços depois de instalar", bold())));
            for p in &w.ports {
                let note = if p.changed() {
                    Span::styled(
                        format!("   (a {} já estava em uso nesta máquina)", p.wanted),
                        Style::default().fg(Color::Yellow),
                    )
                } else {
                    Span::raw("")
                };
                let address = if p.var == "FORGEJO_SSH_PORT" {
                    format!("porta {}", p.chosen)
                } else {
                    format!("http://localhost:{}", p.chosen)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<22}", p.label), Style::default().fg(ACCENT)),
                    Span::raw(address),
                    note,
                ]));
            }
            lines.push(plain(""));
            lines
                .push(plain("Ao confirmar, o assistente grava a configuração (.env e oreia-seca.toml) e a instalação"));
            lines.push(plain("continua sozinha: sobe o Forgejo, cria contas e repositório, e liga o orquestrador."));
        }
    }
    lines
}

fn footer(step: Step) -> &'static str {
    match step {
        Step::Welcome => "Enter: começar · Esc: sair sem instalar",
        Step::Provider => "↑/↓: escolher · Enter: continuar · Esc: voltar",
        Step::Model => "↑/↓: escolher · digite para filtrar · Enter: continuar · Esc: voltar",
        Step::Review => "Enter: gravar e instalar · Esc: voltar e corrigir · Ctrl+C: sair sem instalar",
        _ => "digite para preencher · Tab/↑/↓: trocar de campo · Enter: continuar · Esc: voltar",
    }
}

pub fn draw(frame: &mut Frame, w: &Wizard) {
    let area = centered(frame.area(), 100, 34);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let header = vec![
        Line::from(vec![
            Span::styled("OREIA-SECA", bold().fg(ACCENT)),
            Span::styled("  ·  primeira instalação", dim()),
        ]),
        progress_line(w.step),
    ];
    frame.render_widget(Paragraph::new(header).block(Block::default().borders(Borders::ALL)), rows[0]);

    // Altura útil da lista de modelos: o corpo menos bordas, status, campo e rodapé da própria tela.
    let list_height = (rows[1].height as usize).saturating_sub(12);
    let mut lines = body_lines(w, list_height);
    if let Some(err) = &w.error {
        lines.push(plain(""));
        lines.push(Line::from(Span::styled(
            format!("✗ {err}"),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(format!(" {} ", w.step.title())).title_style(bold())),
        rows[1],
    );

    frame.render_widget(
        Paragraph::new(footer(w.step)).style(dim()).block(Block::default().borders(Borders::ALL)),
        rows[2],
    );
}

/// Caixa centralizada de no máximo `max_w` × `max_h` — a tela inteira num terminal pequeno.
fn centered(area: Rect, max_w: u16, max_h: u16) -> Rect {
    let w = area.width.min(max_w);
    let h = area.height.min(max_h);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn wizard() -> Wizard {
        Wizard::new(&CurrentState::default(), &EnvMap::default())
    }

    fn type_text(w: &mut Wizard, s: &str) {
        for c in s.chars() {
            handle_key(w, key(KeyCode::Char(c)));
        }
    }

    fn clear(w: &mut Wizard, n: usize) {
        for _ in 0..n {
            handle_key(w, key(KeyCode::Backspace));
        }
    }

    fn enter(w: &mut Wizard) -> WizardOutcome {
        handle_key(w, key(KeyCode::Enter))
    }

    #[test]
    fn first_run_is_no_env_or_an_untouched_copy_of_the_example() {
        assert!(is_first_run(false, &EnvMap::default()));
        assert!(is_first_run(true, &EnvMap::parse("TELEGRAM_TOKEN=\nFORGEJO_HTTP_PORT=3000\n")));
        assert!(!is_first_run(true, &EnvMap::parse("LLM_CODER_URL=http://x/v1\n")));
        assert!(!is_first_run(true, &EnvMap::parse("FORGEJO_OWNER_TOKEN=abc\n")));
    }

    #[test]
    fn plan_ports_moves_only_the_busy_ones_and_never_repeats_a_choice() {
        let wanted = [("a", "A", 3000), ("b", "B", 3001), ("c", "C", 7777)];
        let plan = plan_ports(&wanted, &|p| p == 3000);
        // 3000 ocupada → 3001; a segunda queria 3001, que acabou de ser dada à primeira → 3002.
        assert_eq!(plan.iter().map(|p| p.chosen).collect::<Vec<_>>(), vec![3001, 3002, 7777]);
        assert!(plan[0].changed() && plan[1].changed() && !plan[2].changed());

        let free = plan_ports(&wanted, &|_| false);
        assert!(free.iter().all(|p| !p.changed()));
    }

    #[test]
    fn plan_ports_keeps_the_wanted_port_when_nothing_nearby_is_free() {
        let plan = plan_ports(&[("a", "A", 3000)], &|_| true);
        assert_eq!(plan[0].chosen, 3000);
    }

    #[test]
    fn local_provider_walks_to_a_fetch_and_then_to_the_model_list() {
        let mut w = wizard();
        enter(&mut w); // boas-vindas
        assert_eq!(w.step, Step::Provider);
        enter(&mut w); // Ollama neste computador
        assert_eq!(w.step, Step::Connection);
        assert_eq!(w.url.value, "http://host.docker.internal:11434/v1");

        enter(&mut w); // sai do campo da URL para o da chave
        assert_eq!(w.field, 1);
        match enter(&mut w) {
            WizardOutcome::FetchModels { url, api_key } => {
                assert_eq!(url, "http://host.docker.internal:11434/v1");
                assert_eq!(api_key, None);
            }
            _ => panic!("esperava FetchModels"),
        }

        w.on_models(Ok(vec!["llama3".into(), "qwen2.5-coder:14b".into(), "qwen3".into()]));
        assert_eq!(w.step, Step::Model);
        type_text(&mut w, "QWEN");
        assert_eq!(w.visible_models(), vec!["qwen2.5-coder:14b", "qwen3"]);
        handle_key(&mut w, key(KeyCode::Down));
        handle_key(&mut w, key(KeyCode::Down)); // não passa do fim da lista
        assert_eq!(w.chosen_model(), "qwen3");
        enter(&mut w);
        assert_eq!(w.step, Step::Project);
    }

    #[test]
    fn connection_step_rejects_a_placeholder_ip_and_a_missing_key() {
        let mut w = wizard();
        w.step = Step::Connection;
        w.provider = 2; // outra máquina: URL com <ip>
        w.url.value = PROVIDERS[2].url.to_string();
        w.field = 1;
        assert!(matches!(enter(&mut w), WizardOutcome::Continue));
        assert!(w.error.as_deref().unwrap().contains("<ip>"));

        w.provider = 3; // OpenRouter: exige chave
        w.url.value = PROVIDERS[3].url.to_string();
        w.field = 1;
        assert!(matches!(enter(&mut w), WizardOutcome::Continue));
        assert!(w.error.as_deref().unwrap().contains("chave"));

        type_text(&mut w, "sk-or-123");
        assert!(w.error.is_none(), "digitar limpa o erro");
        match enter(&mut w) {
            WizardOutcome::FetchModels { api_key, .. } => assert_eq!(api_key.as_deref(), Some("sk-or-123")),
            _ => panic!("esperava FetchModels"),
        }
    }

    #[test]
    fn a_failed_fetch_still_lets_you_type_the_model_name() {
        let mut w = wizard();
        w.step = Step::Connection;
        w.on_models(Err("sem resposta".into()));
        assert_eq!(w.step, Step::Model);
        assert!(w.fetch_status.as_deref().unwrap().contains("sem resposta"));
        assert!(matches!(enter(&mut w), WizardOutcome::Continue));
        assert!(w.error.is_some(), "modelo vazio não avança");

        type_text(&mut w, "meu-modelo");
        assert_eq!(w.chosen_model(), "meu-modelo");
        enter(&mut w);
        assert_eq!(w.step, Step::Project);
    }

    #[test]
    fn a_previously_configured_model_stays_selected_after_the_fetch() {
        let state = CurrentState { llm_coder_model: "qwen3".into(), ..Default::default() };
        let mut w = Wizard::new(&state, &EnvMap::default());
        w.on_models(Ok(vec!["llama3".into(), "qwen3".into()]));
        assert_eq!(w.chosen_model(), "qwen3");
    }

    #[test]
    fn configure_later_skips_straight_to_the_project_and_back() {
        let mut w = wizard();
        w.step = Step::Provider;
        w.provider = PROVIDERS.iter().position(|p| p.skip).unwrap();
        enter(&mut w);
        assert_eq!(w.step, Step::Project);
        assert!(w.llm_skipped);
        handle_key(&mut w, key(KeyCode::Esc));
        assert_eq!(w.step, Step::Provider, "Esc volta para onde veio, não para a tela de modelo");
    }

    #[test]
    fn project_step_validates_repo_and_owner() {
        let mut w = wizard();
        w.step = Step::Project;
        assert_eq!(w.repo.value, "minha-org/meu-sistema");
        assert_eq!(w.owner.value, "dono");

        clear(&mut w, 40);
        type_text(&mut w, "sem-barra");
        w.field = 1;
        enter(&mut w);
        assert_eq!(w.step, Step::Project);
        assert_eq!(w.field, 0, "o foco volta para o campo com erro");
        assert!(w.error.as_deref().unwrap().contains("dono/repo"));

        clear(&mut w, 40);
        type_text(&mut w, "acme/loja");
        w.field = 1;
        clear(&mut w, 10);
        type_text(&mut w, "admin");
        enter(&mut w);
        assert!(w.error.as_deref().unwrap().contains("reservado"));

        clear(&mut w, 10);
        type_text(&mut w, "ana");
        enter(&mut w);
        assert_eq!(w.step, Step::Telegram);
    }

    #[test]
    fn telegram_step_is_optional_but_validates_what_was_typed() {
        let mut w = wizard();
        w.step = Step::Telegram;
        enter(&mut w); // token em branco → campo dos IDs
        assert!(matches!(enter(&mut w), WizardOutcome::PlanPorts), "tudo em branco = pular");

        w.field = 0;
        type_text(&mut w, "token-sem-dois-pontos");
        w.field = 1;
        assert!(matches!(enter(&mut w), WizardOutcome::Continue));
        assert!(w.error.as_deref().unwrap().contains("BotFather"));

        clear(&mut w, 40);
        type_text(&mut w, "123:ABC");
        w.field = 1;
        type_text(&mut w, "abc");
        assert!(matches!(enter(&mut w), WizardOutcome::Continue));
        assert!(w.error.as_deref().unwrap().contains("números"));
    }

    fn finished() -> Wizard {
        let mut w = wizard();
        w.url.value = "http://host.docker.internal:11434/v1".into();
        w.on_models(Ok(vec!["qwen3".into()]));
        w.repo.value = "acme/loja, acme/site".into();
        w.owner.value = "ana".into();
        w.on_ports(plan_ports(PORT_SPECS, &|p| p == 3000));
        w
    }

    #[test]
    fn payload_carries_llm_repo_owner_px0_and_only_the_changed_ports() {
        let w = finished();
        let (answers, edits) = w.payload().unwrap();
        assert_eq!(answers.llm.url, "http://host.docker.internal:11434/v1");
        assert_eq!(answers.llm.model, "qwen3");
        assert_eq!(answers.forgejo.owner_user, "ana");
        assert_eq!(answers.forgejo.repos, "acme/loja, acme/site");
        assert!(!answers.forgejo.bootstrap, "quem roda o bootstrap é o launcher, não a TUI");
        assert_eq!(answers.px0.allowed_users, "ana", "o dono já entra liberado no px0");
        assert_eq!(answers.ports, PortsAnswers { forgejo_http: Some(3001), ..Default::default() });
        assert_eq!(edits.forgejo_repos, Some(vec!["acme/loja".to_string(), "acme/site".to_string()]));
        assert_eq!(edits.forgejo_public_url.as_deref(), Some("http://localhost:3001"));
        assert_eq!(edits.telegram_enabled, Some(false), "sem token, o canal fica desligado");
    }

    #[test]
    fn payload_respects_what_a_previous_install_already_decided() {
        let state = CurrentState {
            forgejo_owner_token_set: true,
            px0_allowed_users: "chefe".into(),
            telegram_token_set: true,
            ..Default::default()
        };
        let mut w = Wizard::new(&state, &EnvMap::parse("FORGEJO_OWNER_USER=chefe\n"));
        w.llm_skipped = true;
        w.on_ports(plan_ports(PORT_SPECS, &|_| false));
        let (answers, edits) = w.payload().unwrap();
        assert_eq!(answers.forgejo.owner_user, "", "dono travado: não tenta trocar");
        assert_eq!(answers.px0.allowed_users, "", "lista do px0 já existia: não sobrescreve");
        assert_eq!(answers.llm.url, "", "LLM pulado não toca no que havia");
        assert_eq!(edits.telegram_enabled, Some(true), "token antigo mantém o canal ligado");
        assert_eq!(edits.forgejo_public_url, None);
    }

    #[test]
    fn review_enter_finishes_and_esc_walks_back_without_losing_data() {
        let mut w = finished();
        assert_eq!(w.step, Step::Review);
        handle_key(&mut w, key(KeyCode::Esc));
        assert_eq!(w.step, Step::Telegram);
        assert_eq!(w.owner.value, "ana");
        w.step = Step::Review;
        assert!(matches!(enter(&mut w), WizardOutcome::Finish));
    }

    #[test]
    fn esc_on_the_welcome_screen_and_ctrl_c_anywhere_cancel() {
        let mut w = wizard();
        assert!(matches!(handle_key(&mut w, key(KeyCode::Esc)), WizardOutcome::Cancel));
        w.step = Step::Project;
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(handle_key(&mut w, ctrl_c), WizardOutcome::Cancel));
    }
}
