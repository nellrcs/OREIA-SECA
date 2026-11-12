//! Estado dos containers e checklist de pré-requisitos do painel.
//!
//! Só usa o binário `docker` puro (`docker ps` filtrando pela label do
//! projeto Compose + `docker stats`) — nunca `docker compose`. Isso é o que
//! deixa o painel funcionar igual nos dois modos de rodar a TUI: nativa no
//! host (com `cargo`) ou dentro do container avulso do launcher `./oreia`
//! (sem `cargo`), que monta só o `docker.sock` e não tem o plugin `compose`
//! nem os caminhos do host para resolver os volumes do `docker-compose.yml`.
//! Tudo aqui é leitura: subir/parar/bootstrap continuam sendo do launcher
//! (ver `super::LauncherAction`).

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::setup::env_file::read_env_value;

/// Mesmo nome de `name:` no `docker-compose.yml`. `COMPOSE_PROJECT_NAME` no
/// ambiente tem precedência, igual ao próprio Compose.
pub fn compose_project() -> String {
    std::env::var("COMPOSE_PROJECT_NAME").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "oreia-seca".into())
}

/// Host onde as portas publicadas do projeto respondem, do ponto de vista
/// DESTE processo: `localhost` rodando nativo, `host.docker.internal` quando
/// o launcher roda a TUI num container (ele define `OREIA_HOST`).
pub fn probe_host() -> String {
    std::env::var("OREIA_HOST").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "localhost".into())
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContainerRow {
    pub service: String,
    /// `running`, `exited`, `restarting`...
    pub state: String,
    /// Texto do Docker: `Up 3 minutes (healthy)`, `Exited (1) 2 hours ago`.
    pub status_text: String,
    pub cpu_pct: Option<f64>,
    pub mem_usage: Option<String>,
    pub mem_pct: Option<f64>,
}

impl ContainerRow {
    pub fn is_running(&self) -> bool {
        self.state == "running"
    }

    /// `Some(true/false)` só quando o container declara healthcheck.
    pub fn healthy(&self) -> Option<bool> {
        if self.status_text.contains("(healthy)") {
            Some(true)
        } else if self.status_text.contains("(unhealthy)") {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckLevel {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrerequisiteCheck {
    pub label: String,
    pub level: CheckLevel,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct StatusSnapshot {
    pub forgejo_url: String,
    pub px0_url: String,
    pub api_url: String,
    pub containers: Vec<ContainerRow>,
    /// `docker` não respondeu — nunca pânico, só um aviso na tela.
    pub error: Option<String>,
    /// Checklist de requisitos mínimos para subir os containers.
    pub checks: Vec<PrerequisiteCheck>,
}

impl StatusSnapshot {
    pub fn is_ready_for_up(&self) -> bool {
        !self.checks.iter().any(|c| c.level == CheckLevel::Fail)
    }

    pub fn any_running(&self) -> bool {
        self.containers.iter().any(ContainerRow::is_running)
    }
}

/// Lê containers + CPU/memória; devolve também quanto o `docker ps` levou (a
/// "latência" do Docker na área Saúde — sem contar o `docker stats`, que
/// sozinho leva ~2s amostrando). Bloqueante — quem chama é `super::monitor`,
/// fora da thread da interface.
pub fn collect_containers() -> Result<(Vec<ContainerRow>, u64), String> {
    let started = std::time::Instant::now();
    let services = run_docker_ps()?;
    let ps_ms = started.elapsed().as_millis() as u64;
    let running: Vec<&str> = services.iter().filter(|s| s.state == "running").map(|s| s.name.as_str()).collect();
    let stats = if running.is_empty() { StatsByName::new() } else { run_docker_stats(&running).unwrap_or_default() };
    Ok((join_services_with_stats(services, &stats), ps_ms))
}

/// Algum container do projeto está rodando? Só `docker ps` (rápido) — sem o
/// `docker stats` de [`collect_containers`]. Docker fora do ar conta como "não".
pub fn project_running() -> bool {
    run_docker_ps().is_ok_and(|services| services.iter().any(|s| s.state == "running"))
}

pub fn is_port_open(host: &str, port: u16) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(mut addrs) = (host, port).to_socket_addrs() else { return false };
    addrs.next().is_some_and(|addr| std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok())
}

fn ok(label: &str, message: impl Into<String>) -> PrerequisiteCheck {
    PrerequisiteCheck { label: label.into(), level: CheckLevel::Ok, message: message.into() }
}

fn warn(label: &str, message: impl Into<String>) -> PrerequisiteCheck {
    PrerequisiteCheck { label: label.into(), level: CheckLevel::Warn, message: message.into() }
}

fn fail(label: &str, message: impl Into<String>) -> PrerequisiteCheck {
    PrerequisiteCheck { label: label.into(), level: CheckLevel::Fail, message: message.into() }
}

pub fn evaluate_prerequisites(
    env_path: &Path,
    toml_path: &Path,
    containers: &[ContainerRow],
    host: &str,
) -> Vec<PrerequisiteCheck> {
    let mut checks = Vec::new();

    // 1. Arquivos base
    if env_path.exists() && toml_path.exists() {
        checks.push(ok("Arquivos de configuração", ".env e oreia-seca.toml presentes"));
    } else {
        checks.push(fail(
            "Arquivos de configuração",
            "faltam .env ou oreia-seca.toml — salve (Ctrl+S) para criá-los, ou rode \"./oreia install\"",
        ));
    }

    // 2. Bootstrap do Forgejo
    let owner_token = read_env_value(env_path, "FORGEJO_OWNER_TOKEN").unwrap_or_default();
    let db_pass = read_env_value(env_path, "FORGEJO_DB_PASSWORD").unwrap_or_default();
    if !owner_token.is_empty() && !db_pass.is_empty() {
        checks.push(ok("Bootstrap do Forgejo", "contas e tokens de acesso já gerados"));
    } else {
        checks
            .push(fail("Bootstrap do Forgejo", "pendente — tecle 'b' aqui, ou rode \"./oreia bootstrap\" no terminal"));
    }

    // 3. Repositórios
    let repos = read_env_value(env_path, "FORGEJO_REPOS").unwrap_or_default();
    if !repos.trim().is_empty() {
        checks.push(ok("Repositórios", format!("configurado: {repos}")));
    } else {
        checks.push(fail("Repositórios", "nenhum repositório em FORGEJO_REPOS (preencha na área Forgejo)"));
    }

    // 4. Portas de rede
    let fj_port: u16 = read_env_value(env_path, "FORGEJO_HTTP_PORT").and_then(|v| v.parse().ok()).unwrap_or(3000);
    let api_port: u16 = read_env_value(env_path, "OREIA_SECA_PORT").and_then(|v| v.parse().ok()).unwrap_or(3120);
    if containers.iter().any(ContainerRow::is_running) {
        checks.push(ok("Portas de rede", format!("{fj_port} e {api_port} em uso pelos containers do OREIA-SECA")));
    } else if is_port_open(host, fj_port) {
        checks.push(fail(
            "Portas de rede",
            format!("porta {fj_port} ocupada por outro programa — mude FORGEJO_HTTP_PORT no .env"),
        ));
    } else if is_port_open(host, api_port) {
        checks.push(fail(
            "Portas de rede",
            format!("porta {api_port} ocupada por outro programa — mude OREIA_SECA_PORT no .env"),
        ));
    } else {
        checks.push(ok("Portas de rede", format!("{fj_port} e {api_port} livres")));
    }

    // 5. LLM
    let coder_model = read_env_value(env_path, "LLM_CODER_MODEL").unwrap_or_default();
    let has_remote_key =
        ["OPENROUTER_KEY", "ANTHROPIC_API_KEY", "GEMINI_KEY"].iter().any(|k| read_env_value(env_path, k).is_some());
    if !coder_model.is_empty() {
        checks.push(ok("Modelo de IA (LLM)", format!("configurado: {coder_model}")));
    } else if has_remote_key {
        checks.push(ok("Modelo de IA (LLM)", "chave de API remota presente"));
    } else {
        checks
            .push(warn("Modelo de IA (LLM)", "nenhum configurado — os agentes não vão conseguir trabalhar (área LLM)"));
    }

    // 6. px0 allowlist
    let px0_allowed = read_env_value(env_path, "PX0_ALLOWED_USERS").unwrap_or_default();
    if !px0_allowed.trim().is_empty() {
        checks.push(ok("Acesso ao px0", format!("autorizados: {px0_allowed}")));
    } else {
        checks.push(warn("Acesso ao px0", "lista vazia bloqueia todo mundo na revisão de PRs (área px0)"));
    }

    checks
}

/// Traduz a falha de chamar/usar o `docker` numa frase que diz o que fazer —
/// em vez do erro cru do SO ou do daemon.
fn docker_spawn_error(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        "Docker não encontrado (comando \"docker\" fora do PATH). Instale o Docker Desktop (Windows/Mac) ou o \
         Docker Engine (Linux) — veja docs/instalacao.md#requisitos — e tecle 'r'."
            .into()
    } else {
        format!("não consegui chamar \"docker\": {e}")
    }
}

fn docker_daemon_error(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("cannot connect to the docker daemon")
        || lower.contains("error during connect")
        || lower.contains("is the docker daemon running")
        || lower.contains("no such file or directory")
    {
        "O Docker está instalado mas não está respondendo — abra o Docker Desktop (ou inicie o serviço: \"sudo \
         systemctl start docker\") e tecle 'r'."
            .into()
    } else if lower.contains("permission denied") {
        "Sem permissão para falar com o Docker — no Linux, adicione seu usuário ao grupo \"docker\" (\"sudo usermod \
         -aG docker $USER\", depois saia e entre de novo) e tecle 'r'."
            .into()
    } else {
        format!("\"docker ps\" falhou: {}", stderr.trim())
    }
}

struct ServiceInfo {
    service: String,
    name: String,
    state: String,
    status_text: String,
}

const PS_FORMAT: &str = "{{.Label \"com.docker.compose.service\"}}\t{{.Names}}\t{{.State}}\t{{.Status}}";

fn run_docker_ps() -> Result<Vec<ServiceInfo>, String> {
    let filter = format!("label=com.docker.compose.project={}", compose_project());
    let output = Command::new("docker")
        .args(["ps", "-a", "--filter", &filter, "--format", PS_FORMAT])
        .output()
        .map_err(|e| docker_spawn_error(&e))?;
    if !output.status.success() {
        return Err(docker_daemon_error(&String::from_utf8_lossy(&output.stderr)));
    }
    Ok(parse_docker_ps(&String::from_utf8_lossy(&output.stdout)))
}

/// Ordem fixa na tela: primeiro o que você mais olha; o resto em ordem alfabética.
fn service_rank(service: &str) -> usize {
    ["oreia-seca", "forgejo", "postgres", "px0", "forgejo-tls"].iter().position(|s| *s == service).unwrap_or(usize::MAX)
}

fn parse_docker_ps(stdout: &str) -> Vec<ServiceInfo> {
    let mut services: Vec<ServiceInfo> = stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let service = parts.next()?.trim();
            let name = parts.next()?.trim();
            let state = parts.next()?.trim();
            let status_text = parts.next().unwrap_or_default().trim();
            (!service.is_empty() && !name.is_empty()).then(|| ServiceInfo {
                service: service.to_string(),
                name: name.to_string(),
                state: state.to_string(),
                status_text: status_text.to_string(),
            })
        })
        .collect();
    services.sort_by(|a, b| service_rank(&a.service).cmp(&service_rank(&b.service)).then(a.service.cmp(&b.service)));
    services
}

#[derive(Debug, Clone, Default)]
struct ContainerStat {
    cpu_pct: Option<f64>,
    mem_usage: Option<String>,
    mem_pct: Option<f64>,
}

type StatsByName = HashMap<String, ContainerStat>;

/// Só dos NOSSOS containers (passados por nome) — numa máquina com dezenas de
/// outros containers, `docker stats` sem filtro amostraria todos à toa.
fn run_docker_stats(names: &[&str]) -> Result<StatsByName, String> {
    let output = Command::new("docker")
        .args(["stats", "--no-stream", "--format", "{{json .}}"])
        .args(names)
        .output()
        .map_err(|e| format!("não consegui chamar \"docker stats\": {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(parse_docker_stats(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_pct(v: Option<&serde_json::Value>) -> Option<f64> {
    v.and_then(|s| s.as_str()).and_then(|s| s.trim().trim_end_matches('%').parse::<f64>().ok())
}

fn parse_docker_stats(stdout: &str) -> StatsByName {
    let mut map = HashMap::new();
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(name) = v.get("Name").and_then(|s| s.as_str()) else { continue };
        let mem_usage = v.get("MemUsage").and_then(|s| s.as_str()).map(str::to_string);
        map.insert(
            name.to_string(),
            ContainerStat { cpu_pct: parse_pct(v.get("CPUPerc")), mem_usage, mem_pct: parse_pct(v.get("MemPerc")) },
        );
    }
    map
}

fn join_services_with_stats(services: Vec<ServiceInfo>, stats: &StatsByName) -> Vec<ContainerRow> {
    services
        .into_iter()
        .map(|s| {
            let stat = stats.get(&s.name).cloned().unwrap_or_default();
            ContainerRow {
                service: s.service,
                state: s.state,
                status_text: s.status_text,
                cpu_pct: stat.cpu_pct,
                mem_usage: stat.mem_usage,
                mem_pct: stat.mem_pct,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_spawn_error_says_what_to_install_when_docker_is_missing() {
        let e = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory (os error 2)");
        let msg = docker_spawn_error(&e);
        assert!(msg.contains("Docker Desktop"), "{msg}");
        assert!(!msg.contains("os error 2"), "deve substituir o erro cru do SO, não só anexar: {msg}");
    }

    #[test]
    fn docker_spawn_error_keeps_the_raw_message_for_other_io_errors() {
        let e = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        assert!(docker_spawn_error(&e).contains("permission denied"));
    }

    #[test]
    fn docker_daemon_error_explains_a_stopped_daemon_and_a_permission_problem() {
        let stopped = docker_daemon_error("Cannot connect to the Docker daemon at unix:///var/run/docker.sock.");
        assert!(stopped.contains("não está respondendo"), "{stopped}");
        let windows =
            docker_daemon_error("error during connect: Get \"http://%2F%2F.%2Fpipe%2FdockerDesktopLinuxEngine\"");
        assert!(windows.contains("Docker Desktop"), "{windows}");
        let perm = docker_daemon_error("permission denied while trying to connect to the Docker daemon socket");
        assert!(perm.contains("grupo \"docker\""), "{perm}");
        assert!(docker_daemon_error("algo novo").contains("algo novo"));
    }

    #[test]
    fn parse_docker_ps_reads_tab_separated_lines_in_a_stable_order() {
        let stdout = "px0\toreia-seca-px0-1\texited\tExited (1) 2 hours ago\n\
                      forgejo\toreia-seca-forgejo-1\trunning\tUp 3 minutes (healthy)\n\
                      \n\
                      oreia-seca\toreia-seca-oreia-seca-1\trunning\tUp 3 minutes (unhealthy)\n";
        let services = parse_docker_ps(stdout);
        let order: Vec<&str> = services.iter().map(|s| s.service.as_str()).collect();
        assert_eq!(order, vec!["oreia-seca", "forgejo", "px0"]);
        assert_eq!(services[1].name, "oreia-seca-forgejo-1");
        assert_eq!(services[1].state, "running");
        assert_eq!(services[2].status_text, "Exited (1) 2 hours ago");
    }

    #[test]
    fn parse_docker_ps_skips_lines_without_a_compose_service() {
        assert!(parse_docker_ps("\tavulso\trunning\tUp\n").is_empty());
    }

    #[test]
    fn container_row_reads_health_from_the_status_text() {
        let row = |status: &str| ContainerRow { status_text: status.into(), ..Default::default() };
        assert_eq!(row("Up 3 minutes (healthy)").healthy(), Some(true));
        assert_eq!(row("Up 3 minutes (unhealthy)").healthy(), Some(false));
        assert_eq!(row("Up 3 minutes").healthy(), None);
    }

    #[test]
    fn parse_docker_stats_reads_cpu_and_memory() {
        let stdout = "{\"Name\":\"oreia-seca-forgejo-1\",\"CPUPerc\":\"1.23%\",\"MemUsage\":\"63.1MiB / 15.59GiB\",\"MemPerc\":\"0.40%\"}\n";
        let stats = parse_docker_stats(stdout);
        let stat = stats.get("oreia-seca-forgejo-1").unwrap();
        assert_eq!(stat.cpu_pct, Some(1.23));
        assert_eq!(stat.mem_pct, Some(0.40));
        assert_eq!(stat.mem_usage.as_deref(), Some("63.1MiB / 15.59GiB"));
    }

    #[test]
    fn join_services_with_stats_matches_by_container_name() {
        let svc = |service: &str, name: &str| ServiceInfo {
            service: service.into(),
            name: name.into(),
            state: "running".into(),
            status_text: "Up".into(),
        };
        let services = vec![svc("forgejo", "oreia-seca-forgejo-1"), svc("px0", "oreia-seca-px0-1")];
        let mut stats = HashMap::new();
        stats.insert(
            "oreia-seca-forgejo-1".to_string(),
            ContainerStat { cpu_pct: Some(2.5), mem_usage: Some("100MiB / 1GiB".to_string()), mem_pct: Some(9.8) },
        );

        let rows = join_services_with_stats(services, &stats);
        assert_eq!(rows[0].cpu_pct, Some(2.5));
        assert_eq!(rows[1].cpu_pct, None, "px0 não tinha entrada em stats, deve ficar None, não quebrar");
    }

    fn test_dir(label: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("oreia-seca-status-{label}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn evaluate_prerequisites_flags_missing_bootstrap_and_repos() {
        let tmp = test_dir("missing");
        let env_file = tmp.join(".env");
        let toml_file = tmp.join("oreia-seca.toml");
        // Portas improváveis de estarem em uso na máquina de quem roda os testes.
        std::fs::write(&env_file, "FORGEJO_HTTP_PORT=39871\nOREIA_SECA_PORT=39872\n").unwrap();
        std::fs::write(&toml_file, "").unwrap();

        let checks = evaluate_prerequisites(&env_file, &toml_file, &[], "127.0.0.1");
        let bootstrap_check = checks.iter().find(|c| c.label == "Bootstrap do Forgejo").unwrap();
        assert_eq!(bootstrap_check.level, CheckLevel::Fail);

        let repos_check = checks.iter().find(|c| c.label == "Repositórios").unwrap();
        assert_eq!(repos_check.level, CheckLevel::Fail);

        let snapshot = StatusSnapshot { checks, ..Default::default() };
        assert!(!snapshot.is_ready_for_up(), "não deve permitir UP sem bootstrap e repos");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn evaluate_prerequisites_passes_when_required_fields_present() {
        let tmp = test_dir("present");
        let env_file = tmp.join(".env");
        let toml_file = tmp.join("oreia-seca.toml");
        std::fs::write(
            &env_file,
            "FORGEJO_OWNER_TOKEN=tok123\nFORGEJO_DB_PASSWORD=secret\nFORGEJO_REPOS=org/repo\nLLM_CODER_MODEL=qwen\n",
        )
        .unwrap();
        std::fs::write(&toml_file, "").unwrap();

        let running = ContainerRow { service: "forgejo".into(), state: "running".into(), ..Default::default() };
        let checks = evaluate_prerequisites(&env_file, &toml_file, &[running], "127.0.0.1");

        for label in ["Bootstrap do Forgejo", "Repositórios", "Portas de rede", "Modelo de IA (LLM)"] {
            let check = checks.iter().find(|c| c.label == label).unwrap();
            assert_eq!(check.level, CheckLevel::Ok, "{label}: {}", check.message);
        }

        let snapshot = StatusSnapshot { checks, ..Default::default() };
        assert!(snapshot.is_ready_for_up(), "deve permitir UP com campos obrigatórios preenchidos");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn evaluate_prerequisites_detects_a_port_taken_by_something_else() {
        let tmp = test_dir("port");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let env_file = tmp.join(".env");
        std::fs::write(&env_file, format!("FORGEJO_HTTP_PORT={port}\nOREIA_SECA_PORT=39873\n")).unwrap();

        let checks = evaluate_prerequisites(&env_file, &tmp.join("oreia-seca.toml"), &[], "127.0.0.1");
        let ports = checks.iter().find(|c| c.label == "Portas de rede").unwrap();
        assert_eq!(ports.level, CheckLevel::Fail);
        assert!(ports.message.contains(&port.to_string()), "{}", ports.message);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
