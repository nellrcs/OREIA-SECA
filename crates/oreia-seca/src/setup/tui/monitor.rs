//! Coleta em segundo plano do que o painel mostra: containers, fila do
//! orquestrador, saúde dos serviços de terceiros (Forgejo, LLM, Telegram,
//! px0, Docker...) e o resumo de uso.
//!
//! A interface nunca espera por rede nem por `docker stats` (que leva ~2s):
//! [`spawn`] roda um laço no runtime do tokio e entrega cada [`Sample`] por
//! um canal; a thread da TUI só faz `try_recv`. O laço dorme entre ciclos e
//! acorda antes se alguém pedir (`r` na tela).
//!
//! Os endereços são sempre os do ponto de vista DESTE processo — veja
//! [`status::probe_host`] e [`reachable_url`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use tokio::sync::Notify;

use super::status::{self, ContainerRow, StatusSnapshot};
use super::usage::{self, UsageSummary};

/// Containers e fila: a cada ciclo.
const CYCLE: Duration = Duration::from_secs(5);
/// Saúde dos serviços (inclui chamadas a APIs de terceiros): a cada N ciclos,
/// para não ficar batendo em servidor alheio a cada 5s.
const HEALTH_EVERY: u32 = 3;
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    /// Respondeu como esperado.
    Up,
    /// Respondeu, mas com algo errado (HTTP 5xx, modelo fora da lista, token recusado).
    Degraded,
    /// Não respondeu.
    Down,
    /// Não configurado ou desligado de propósito — não conta como problema.
    Off,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceHealth {
    /// Identificador estável (chave do histórico): `forgejo`, `llm-coder`...
    pub id: String,
    pub name: String,
    /// O que é, em uma palavra: `git`, `LLM`, `canal`...
    pub kind: &'static str,
    pub state: HealthState,
    pub latency_ms: Option<u64>,
    pub detail: String,
}

impl ServiceHealth {
    fn new(
        id: &str,
        name: &str,
        kind: &'static str,
        state: HealthState,
        latency_ms: Option<u64>,
        detail: String,
    ) -> Self {
        Self { id: id.into(), name: name.into(), kind, state, latency_ms, detail }
    }

    fn off(id: &str, name: &str, kind: &'static str, detail: &str) -> Self {
        Self::new(id, name, kind, HealthState::Off, None, detail.into())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoleRow {
    pub role: String,
    pub model: String,
    pub ready: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelRow {
    pub key: String,
    pub model: String,
    pub base_url: String,
    /// `ok`, `unreachable` ou `not_configured` (como a API devolve).
    pub state: String,
}

/// `GET /api/status` do orquestrador, já decodificado.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApiStatus {
    pub running: u64,
    pub queued: u64,
    pub completed: u64,
    pub failed: u64,
    pub concurrency: u64,
    pub pending_approvals: usize,
    pub active_work: Vec<String>,
    pub agents: Vec<String>,
    pub roles: Vec<RoleRow>,
    pub models: Vec<ModelRow>,
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub snapshot: StatusSnapshot,
    /// `None` = o orquestrador não respondeu (parado, subindo, ou token errado).
    pub api: Option<ApiStatus>,
    /// `None` = este ciclo não rodou as checagens de saúde (ver [`HEALTH_EVERY`]).
    pub health: Option<Vec<ServiceHealth>>,
    pub usage: UsageSummary,
    pub at: DateTime<Local>,
    pub took_ms: u64,
}

/// `.env` lido uma vez por ciclo (em vez de reabrir o arquivo a cada chave).
#[derive(Debug, Default, Clone)]
pub struct EnvMap(BTreeMap<String, String>);

impl EnvMap {
    pub fn load(path: &Path) -> Self {
        Self::parse(&std::fs::read_to_string(path).unwrap_or_default())
    }

    pub fn parse(text: &str) -> Self {
        let map = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), strip_inline_comment(v).to_string()))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        Self(map)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    fn port(&self, key: &str, default: u16) -> u16 {
        self.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }
}

/// `3121   # comentário` → `3121` (o `.env.example` tem linhas assim).
fn strip_inline_comment(value: &str) -> &str {
    let value = value.trim();
    match value.find(" #") {
        Some(i) => value[..i].trim(),
        None => value,
    }
}

/// As URLs de LLM no `.env` são escritas do ponto de vista do container do
/// orquestrador (`host.docker.internal` = "o computador onde o Docker roda").
/// Com a TUI rodando nativa nesse mesmo computador, isso é `localhost` — e no
/// Linux `host.docker.internal` nem resolve fora de um container.
pub fn reachable_url(url: &str, in_container: bool) -> String {
    if in_container {
        url.to_string()
    } else {
        url.replace("host.docker.internal", "localhost")
    }
}

fn in_container() -> bool {
    std::env::var("OREIA_HOST").is_ok_and(|v| !v.trim().is_empty())
}

/// Mensagem curta para a coluna "Detalhe". `without_url` porque a URL do
/// Telegram carrega o token do bot — nunca pode ir parar na tela.
fn net_error(e: reqwest::Error) -> String {
    let e = e.without_url();
    if e.is_timeout() {
        "tempo esgotado".into()
    } else if e.is_connect() {
        "sem resposta (conexão recusada ou host inacessível)".into()
    } else {
        format!("erro: {e}")
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

async fn probe_http(
    client: &reqwest::Client,
    id: &str,
    name: &str,
    kind: &'static str,
    url: &str,
    shown_url: &str,
) -> ServiceHealth {
    let started = Instant::now();
    match client.get(url).send().await {
        Ok(resp) => {
            let ms = elapsed_ms(started);
            let code = resp.status();
            // 3xx conta como "no ar": o px0 redireciona para o login.
            if code.is_success() || code.is_redirection() {
                ServiceHealth::new(id, name, kind, HealthState::Up, Some(ms), shown_url.to_string())
            } else {
                ServiceHealth::new(
                    id,
                    name,
                    kind,
                    HealthState::Degraded,
                    Some(ms),
                    format!("HTTP {} em {shown_url}", code.as_u16()),
                )
            }
        }
        Err(e) => ServiceHealth::new(id, name, kind, HealthState::Down, None, net_error(e)),
    }
}

#[derive(serde::Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(serde::Deserialize)]
struct ModelEntry {
    id: String,
}

fn url_host(url: &str) -> &str {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or(url)
}

async fn probe_llm(
    client: &reqwest::Client,
    id: &str,
    name: &str,
    url: Option<&str>,
    model: Option<&str>,
    api_key: Option<&str>,
    in_container: bool,
) -> ServiceHealth {
    let Some(url) = url else {
        return ServiceHealth::off(id, name, "LLM", "não configurado — área LLM, ou \"./oreia install\"");
    };
    let target = format!("{}/models", reachable_url(url, in_container).trim_end_matches('/'));
    let mut req = client.get(&target);
    if let Some(k) = api_key {
        req = req.bearer_auth(k);
    }
    let started = Instant::now();
    let host = url_host(url);
    match req.send().await {
        Ok(resp) => {
            let ms = elapsed_ms(started);
            let code = resp.status();
            if code == reqwest::StatusCode::UNAUTHORIZED || code == reqwest::StatusCode::FORBIDDEN {
                return ServiceHealth::new(
                    id,
                    name,
                    "LLM",
                    HealthState::Degraded,
                    Some(ms),
                    format!("{host}: chave da API recusada (HTTP {})", code.as_u16()),
                );
            }
            if !code.is_success() {
                return ServiceHealth::new(
                    id,
                    name,
                    "LLM",
                    HealthState::Degraded,
                    Some(ms),
                    format!("{host}: HTTP {}", code.as_u16()),
                );
            }
            let ids: Vec<String> = resp
                .json::<ModelsResponse>()
                .await
                .map(|m| m.data.into_iter().map(|e| e.id).collect())
                .unwrap_or_default();
            let (state, detail) = llm_detail(host, model, &ids);
            ServiceHealth::new(id, name, "LLM", state, Some(ms), detail)
        }
        Err(e) => ServiceHealth::new(id, name, "LLM", HealthState::Down, None, format!("{host}: {}", net_error(e))),
    }
}

/// O servidor respondeu — o modelo configurado está na lista dele?
fn llm_detail(host: &str, model: Option<&str>, available: &[String]) -> (HealthState, String) {
    match model {
        None => (HealthState::Degraded, format!("{host}: no ar, mas nenhum modelo escolhido")),
        // Alguns servidores não listam nada em /models — não dá para afirmar que falta.
        Some(m) if available.is_empty() => (HealthState::Up, format!("{host} · {m}")),
        Some(m) if available.iter().any(|a| a == m) => {
            (HealthState::Up, format!("{host} · {m} · {} modelo(s) no servidor", available.len()))
        }
        Some(m) => (HealthState::Degraded, format!("{host}: modelo \"{m}\" não está na lista do servidor")),
    }
}

async fn probe_telegram(client: &reqwest::Client, token: Option<&str>) -> ServiceHealth {
    let (id, name) = ("telegram", "Telegram");
    let Some(token) = token else {
        return ServiceHealth::off(id, name, "canal", "sem token — opcional (área Telegram)");
    };
    let started = Instant::now();
    match client.get(format!("https://api.telegram.org/bot{token}/getMe")).send().await {
        Ok(resp) => {
            let ms = elapsed_ms(started);
            let code = resp.status();
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            if code.is_success() && body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                let bot = body.pointer("/result/username").and_then(|v| v.as_str()).unwrap_or("bot");
                ServiceHealth::new(id, name, "canal", HealthState::Up, Some(ms), format!("@{bot}"))
            } else if code == reqwest::StatusCode::UNAUTHORIZED {
                ServiceHealth::new(
                    id,
                    name,
                    "canal",
                    HealthState::Degraded,
                    Some(ms),
                    "token recusado pelo Telegram — confira com o @BotFather".into(),
                )
            } else {
                ServiceHealth::new(
                    id,
                    name,
                    "canal",
                    HealthState::Degraded,
                    Some(ms),
                    format!("HTTP {}", code.as_u16()),
                )
            }
        }
        Err(e) => ServiceHealth::new(id, name, "canal", HealthState::Down, None, net_error(e)),
    }
}

fn container_health(
    id: &str,
    name: &str,
    kind: &'static str,
    service: &str,
    containers: &[ContainerRow],
) -> ServiceHealth {
    match containers.iter().find(|c| c.service == service) {
        None => ServiceHealth::off(id, name, kind, "container ainda não criado — tecle 'u' para subir"),
        Some(c) if !c.is_running() => {
            ServiceHealth::new(id, name, kind, HealthState::Down, None, c.status_text.clone())
        }
        Some(c) => match c.healthy() {
            Some(false) => ServiceHealth::new(id, name, kind, HealthState::Degraded, None, c.status_text.clone()),
            _ => ServiceHealth::new(id, name, kind, HealthState::Up, None, c.status_text.clone()),
        },
    }
}

fn docker_health(result: &Result<Vec<ContainerRow>, String>, took_ms: u64) -> ServiceHealth {
    match result {
        Ok(rows) => {
            let running = rows.iter().filter(|c| c.is_running()).count();
            let detail = format!("{running} de {} container(s) do projeto rodando", rows.len());
            ServiceHealth::new("docker", "Docker", "infra", HealthState::Up, Some(took_ms), detail)
        }
        Err(e) => ServiceHealth::new("docker", "Docker", "infra", HealthState::Down, None, e.clone()),
    }
}

struct Targets {
    host: String,
    in_container: bool,
    forgejo_port: u16,
    api_port: u16,
    px0_port: u16,
    mcp_port: u16,
}

impl Targets {
    fn from_env(env: &EnvMap) -> Self {
        Self {
            host: status::probe_host(),
            in_container: in_container(),
            forgejo_port: env.port("FORGEJO_HTTP_PORT", 3000),
            api_port: env.port("OREIA_SECA_PORT", 3120),
            px0_port: env.port("PX0_PORT", 7777),
            mcp_port: env.port("MCP_HTTP_PORT", 3121),
        }
    }

    fn local(&self, port: u16) -> String {
        format!("http://{}:{port}", self.host)
    }
}

async fn probe_all(
    client: &reqwest::Client,
    env: &EnvMap,
    t: &Targets,
    docker: &Result<Vec<ContainerRow>, String>,
    docker_ms: u64,
) -> Vec<ServiceHealth> {
    let containers: &[ContainerRow] = docker.as_deref().unwrap_or_default();
    let shown = |port: u16| format!("localhost:{port}");

    let api_url = format!("{}/health", t.local(t.api_port));
    let forgejo_url = format!("{}/api/healthz", t.local(t.forgejo_port));
    let px0_url = format!("{}/", t.local(t.px0_port));
    let (api_shown, forgejo_shown, px0_shown) = (shown(t.api_port), shown(t.forgejo_port), shown(t.px0_port));

    let coder_url = env.get("LLM_CODER_URL");
    let chat_url = env.get("LLM_CHAT_URL");
    let key = env.get("LLM_API_KEY");
    // Mesmo servidor e mesmo modelo para os dois papéis: uma linha só.
    let same_llm = chat_url == coder_url && env.get("LLM_CHAT_MODEL") == env.get("LLM_CODER_MODEL");

    let (api, forgejo, px0, coder, chat, telegram) = tokio::join!(
        probe_http(client, "api", "Orquestrador", "API", &api_url, &api_shown),
        probe_http(client, "forgejo", "Forgejo", "git", &forgejo_url, &forgejo_shown),
        probe_http(client, "px0", "px0", "revisão", &px0_url, &px0_shown),
        probe_llm(client, "llm-coder", "LLM · código", coder_url, env.get("LLM_CODER_MODEL"), key, t.in_container),
        async {
            if same_llm {
                None
            } else {
                Some(
                    probe_llm(
                        client,
                        "llm-chat",
                        "LLM · conversa",
                        chat_url,
                        env.get("LLM_CHAT_MODEL"),
                        key,
                        t.in_container,
                    )
                    .await,
                )
            }
        },
        probe_telegram(client, env.get("TELEGRAM_TOKEN")),
    );

    let mcp = if env.get("MCP_TOKEN").is_some() {
        let host = t.host.clone();
        let port = t.mcp_port;
        let open = tokio::task::spawn_blocking(move || status::is_port_open(&host, port)).await.unwrap_or(false);
        if open {
            ServiceHealth::new("mcp-http", "MCP (HTTP)", "MCP", HealthState::Up, None, format!("localhost:{port}"))
        } else {
            ServiceHealth::new(
                "mcp-http",
                "MCP (HTTP)",
                "MCP",
                HealthState::Down,
                None,
                format!("porta {port} fechada — rode \"./oreia mcp-http\" depois de subir"),
            )
        }
    } else {
        ServiceHealth::off("mcp-http", "MCP (HTTP)", "MCP", "transporte HTTP desligado — opcional (área MCP)")
    };

    let mut all = vec![
        docker_health(docker, docker_ms),
        api,
        forgejo,
        container_health("postgres", "Postgres", "banco", "postgres", containers),
        px0,
        coder,
    ];
    all.extend(chat);
    all.push(telegram);
    all.push(mcp);
    all
}

async fn fetch_api_status(client: &reqwest::Client, base: &str, api_key: Option<&str>) -> Option<ApiStatus> {
    let mut req = client.get(format!("{base}/api/status"));
    if let Some(k) = api_key {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    Some(parse_api_status(&json))
}

/// Espelha `Maker::status_json` (`crates/oreia-seca/src/maker/mod.rs`). Campo
/// ausente vira zero/vazio — uma versão mais antiga do orquestrador não deve
/// derrubar o painel.
pub fn parse_api_status(json: &serde_json::Value) -> ApiStatus {
    let num = |path: &str| json.pointer(path).and_then(|v| v.as_u64()).unwrap_or(0);
    let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|s| s.as_str()).unwrap_or_default().to_string();
    let strings = |key: &str| -> Vec<String> {
        json.get(key)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str()).map(str::to_string).collect())
            .unwrap_or_default()
    };
    let list = |key: &str| json.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();

    ApiStatus {
        running: num("/queue/running"),
        queued: num("/queue/queued"),
        completed: num("/queue/completed"),
        failed: num("/queue/failed"),
        concurrency: num("/queue/concurrency"),
        pending_approvals: list("pending_approvals").len(),
        active_work: strings("active_work"),
        agents: strings("agents"),
        roles: list("roles")
            .iter()
            .map(|r| RoleRow {
                role: text(r, "role"),
                model: text(r, "model"),
                ready: r.get("ready").and_then(|v| v.as_bool()).unwrap_or(false),
            })
            .collect(),
        models: list("models")
            .iter()
            .map(|m| ModelRow {
                key: text(m, "key"),
                model: text(m, "model"),
                base_url: text(m, "base_url"),
                state: text(m, "state"),
            })
            .collect(),
    }
}

/// Um ciclo completo de coleta. Nunca falha: o que não respondeu vira
/// "fora do ar" no próprio [`Sample`].
pub async fn collect(env_path: &Path, toml_path: &Path, with_health: bool) -> Sample {
    let started = Instant::now();
    let env = EnvMap::load(env_path);
    let targets = Targets::from_env(&env);

    let collected = tokio::task::spawn_blocking(status::collect_containers)
        .await
        .unwrap_or_else(|e| Err(format!("a coleta de containers falhou: {e}")));
    let docker_ms = collected.as_ref().map(|(_, ms)| *ms).unwrap_or(0);
    let docker: Result<Vec<ContainerRow>, String> = collected.map(|(rows, _)| rows);

    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default();

    let api = fetch_api_status(&client, &targets.local(targets.api_port), env.get("REST_API_KEY")).await;
    let health = if with_health { Some(probe_all(&client, &env, &targets, &docker, docker_ms).await) } else { None };

    let (env_owned, toml_owned, host) = (env_path.to_path_buf(), toml_path.to_path_buf(), targets.host.clone());
    let containers = docker.clone().unwrap_or_default();
    let (checks, usage) = tokio::task::spawn_blocking(move || {
        let checks = status::evaluate_prerequisites(&env_owned, &toml_owned, &containers, &host);
        let paths_data = crate::setup::current_state::load(&env_owned, &toml_owned).paths_data;
        (checks, usage::load(&usage::candidate_paths(&paths_data), Utc::now()))
    })
    .await
    .unwrap_or_default();

    let snapshot = StatusSnapshot {
        forgejo_url: format!("http://localhost:{}", targets.forgejo_port),
        px0_url: format!("http://localhost:{}", targets.px0_port),
        api_url: format!("http://localhost:{}", targets.api_port),
        error: docker.as_ref().err().cloned(),
        containers: docker.unwrap_or_default(),
        checks,
    };

    Sample { snapshot, api, health, usage, at: Local::now(), took_ms: elapsed_ms(started) }
}

/// Controle do laço em segundo plano. Soltar o `Monitor` encerra o laço.
pub struct Monitor {
    wake: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Monitor {
    /// Pede um ciclo completo agora (com as checagens de saúde), sem esperar o intervalo.
    pub fn refresh_now(&self) {
        self.wake.notify_one();
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn spawn(handle: &tokio::runtime::Handle, env_path: PathBuf, toml_path: PathBuf, tx: Sender<Sample>) -> Monitor {
    let wake = Arc::new(Notify::new());
    let wake_loop = wake.clone();
    let task = handle.spawn(async move {
        let mut cycle: u32 = 0;
        let mut forced = false;
        loop {
            let with_health = forced || cycle.is_multiple_of(HEALTH_EVERY);
            let sample = collect(&env_path, &toml_path, with_health).await;
            if tx.send(sample).is_err() {
                return; // a TUI fechou
            }
            cycle = cycle.wrapping_add(1);
            forced = tokio::select! {
                _ = tokio::time::sleep(CYCLE) => false,
                _ = wake_loop.notified() => true,
            };
        }
    });
    Monitor { wake, task }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_map_skips_comments_blanks_and_empty_values() {
        let env = EnvMap::parse("# comentário\n\nA=1\nB=\n  C = dois  \nMCP_HTTP_PORT=3121   # só se ligar\n");
        assert_eq!(env.get("A"), Some("1"));
        assert_eq!(env.get("B"), None);
        assert_eq!(env.get("C"), Some("dois"));
        assert_eq!(env.port("MCP_HTTP_PORT", 1), 3121);
        assert_eq!(env.port("FALTA", 3000), 3000);
        assert_eq!(env.port("C", 3000), 3000, "valor não numérico cai no padrão");
    }

    #[test]
    fn reachable_url_swaps_the_docker_host_alias_only_outside_a_container() {
        let url = "http://host.docker.internal:1234/v1";
        assert_eq!(reachable_url(url, false), "http://localhost:1234/v1");
        assert_eq!(reachable_url(url, true), url);
        assert_eq!(reachable_url("https://openrouter.ai/api/v1", false), "https://openrouter.ai/api/v1");
    }

    #[test]
    fn llm_detail_flags_a_model_missing_from_the_server_list() {
        let list = vec!["qwen".to_string(), "llama".to_string()];
        assert_eq!(llm_detail("h:1", Some("qwen"), &list).0, HealthState::Up);
        let (state, detail) = llm_detail("h:1", Some("outro"), &list);
        assert_eq!(state, HealthState::Degraded);
        assert!(detail.contains("outro"), "{detail}");
        assert_eq!(llm_detail("h:1", Some("qwen"), &[]).0, HealthState::Up, "lista vazia não prova que falta");
        assert_eq!(llm_detail("h:1", None, &list).0, HealthState::Degraded);
    }

    #[test]
    fn container_health_maps_docker_states() {
        let row = |state: &str, status: &str| ContainerRow {
            service: "postgres".into(),
            state: state.into(),
            status_text: status.into(),
            ..Default::default()
        };
        let check = |rows: &[ContainerRow]| container_health("postgres", "Postgres", "banco", "postgres", rows).state;
        assert_eq!(check(&[]), HealthState::Off);
        assert_eq!(check(&[row("running", "Up 1 minute (healthy)")]), HealthState::Up);
        assert_eq!(check(&[row("running", "Up 1 minute (unhealthy)")]), HealthState::Degraded);
        assert_eq!(check(&[row("exited", "Exited (1) 1 minute ago")]), HealthState::Down);
    }

    #[test]
    fn docker_health_reports_the_daemon_error_as_down() {
        assert_eq!(docker_health(&Err("parado".into()), 0).state, HealthState::Down);
        let rows = vec![ContainerRow { state: "running".into(), ..Default::default() }, ContainerRow::default()];
        let up = docker_health(&Ok(rows), 42);
        assert_eq!(up.state, HealthState::Up);
        assert_eq!(up.latency_ms, Some(42));
        assert!(up.detail.contains("1 de 2"), "{}", up.detail);
    }

    #[test]
    fn parse_api_status_reads_queue_roles_and_models() {
        let json = serde_json::json!({
            "queue": {"running": 1, "queued": 2, "failed": 3, "completed": 10, "concurrency": 1},
            "pending_approvals": [{"id": "a", "question": "q"}],
            "roles": [{"role": "planner", "model": "coder", "ready": true}],
            "models": [{"key": "coder", "state": "ok", "base_url": "http://x/v1", "model": "qwen"}],
            "active_work": ["acme/app#12"],
            "agents": ["dev-back", "reviewer"],
        });
        let s = parse_api_status(&json);
        assert_eq!((s.running, s.queued, s.failed, s.completed), (1, 2, 3, 10));
        assert_eq!(s.pending_approvals, 1);
        assert_eq!(s.roles, vec![RoleRow { role: "planner".into(), model: "coder".into(), ready: true }]);
        assert_eq!(s.models[0].state, "ok");
        assert_eq!(s.active_work, vec!["acme/app#12".to_string()]);
        assert_eq!(s.agents.len(), 2);
    }

    #[test]
    fn parse_api_status_tolerates_missing_fields() {
        assert_eq!(parse_api_status(&serde_json::json!({})), ApiStatus::default());
    }

    async fn serve(app: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn probe_http_distinguishes_up_redirect_error_and_down() {
        use axum::http::StatusCode;
        use axum::routing::get;
        let base = serve(
            axum::Router::new()
                .route("/ok", get(|| async { "ok" }))
                .route("/login", get(|| async { (StatusCode::FOUND, [("location", "/x")]) }))
                .route("/boom", get(|| async { StatusCode::INTERNAL_SERVER_ERROR })),
        )
        .await;
        let c = client();
        let up = probe_http(&c, "a", "A", "API", &format!("{base}/ok"), "localhost:1").await;
        assert_eq!(up.state, HealthState::Up);
        assert!(up.latency_ms.is_some());
        assert_eq!(probe_http(&c, "a", "A", "API", &format!("{base}/login"), "x").await.state, HealthState::Up);
        let boom = probe_http(&c, "a", "A", "API", &format!("{base}/boom"), "x").await;
        assert_eq!(boom.state, HealthState::Degraded);
        assert!(boom.detail.contains("500"), "{}", boom.detail);

        // Porta que ninguém escuta: abre e fecha um listener só para descobrir uma livre.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let down = probe_http(&c, "a", "A", "API", &format!("http://127.0.0.1:{free}/"), "x").await;
        assert_eq!(down.state, HealthState::Down);
        assert_eq!(down.latency_ms, None);
    }

    #[tokio::test]
    async fn probe_llm_checks_the_configured_model_against_the_server_list() {
        use axum::routing::get;
        let base = serve(axum::Router::new().route(
            "/v1/models",
            get(|| async { axum::Json(serde_json::json!({"data": [{"id": "qwen"}, {"id": "llama"}]})) }),
        ))
        .await;
        let url = format!("{base}/v1");
        let c = client();

        let ok = probe_llm(&c, "llm-coder", "LLM", Some(&url), Some("qwen"), None, true).await;
        assert_eq!(ok.state, HealthState::Up, "{}", ok.detail);
        assert!(ok.detail.contains("2 modelo(s)"), "{}", ok.detail);

        let missing = probe_llm(&c, "llm-coder", "LLM", Some(&url), Some("gpt"), None, true).await;
        assert_eq!(missing.state, HealthState::Degraded);

        let off = probe_llm(&c, "llm-coder", "LLM", None, None, None, true).await;
        assert_eq!(off.state, HealthState::Off);
    }

    #[tokio::test]
    async fn probe_llm_reports_a_rejected_api_key() {
        use axum::http::StatusCode;
        use axum::routing::get;
        let base = serve(axum::Router::new().route("/v1/models", get(|| async { StatusCode::UNAUTHORIZED }))).await;
        let h = probe_llm(&client(), "llm-coder", "LLM", Some(&format!("{base}/v1")), Some("m"), Some("k"), true).await;
        assert_eq!(h.state, HealthState::Degraded);
        assert!(h.detail.contains("chave"), "{}", h.detail);
    }
}
