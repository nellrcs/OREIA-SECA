//! TUI de `oreia-seca setup` (ratatui + crossterm). Três coisas numa tela só:
//!
//! - **Assistente de primeira instalação** ([`wizard`]): cinco perguntas e
//!   pronto. Abre sozinho quando ainda não há nada configurado.
//! - **Painel** — áreas Status, Saúde e Uso, só de leitura, alimentadas por
//!   um coletor em segundo plano ([`monitor`]): containers com CPU/memória,
//!   fila do orquestrador, saúde e latência de cada serviço de terceiros, e
//!   o consumo de tokens/custo dos agentes.
//! - **Configuração** — px0, Forgejo, MCP, Telegram, LLM, Executor e
//!   Servidor, gravando em `.env`/`oreia-seca.toml` (via `apply_answers` e
//!   `toml_edits::apply`, os mesmos do modo não interativo).
//!
//! A TUI nunca chama `docker compose`: ela pode estar rodando num container
//! avulso que não tem o plugin nem os caminhos do host. Subir, parar e o
//! bootstrap são pedidos ao launcher `./oreia` — a TUI grava a ação no
//! arquivo de `OREIA_ACTION_FILE` e sai; o launcher executa no host e reabre
//! o painel (ver [`app::LauncherAction`]). Aberta sem o launcher, as mesmas
//! teclas só dizem qual comando rodar.

mod app;
mod event;
mod mascot;
pub mod monitor;
mod status;
mod ui;
mod usage;
mod wizard;

use std::io::{self, IsTerminal, Stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{Event, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::toml_edits::TomlEdits;
use super::{apply_answers, current_state, print_report, toml_edits, Answers};
use app::{App, LauncherAction, Screen};
use event::AppOutcome;
use monitor::EnvMap;
use wizard::{Wizard, WizardOutcome};

/// Intervalo de espera por tecla — também o atraso máximo para um dado novo
/// do monitor aparecer na tela.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Na abertura a espera é mais curta, para a animação sair fluida.
const SPLASH_POLL_INTERVAL: Duration = Duration::from_millis(60);

/// A abertura aparece uma vez por sessão: o launcher reabre o painel depois
/// de cada ação (subir, parar, logs...) com `OREIA_SPLASH=0`, para não
/// repetir. A mesma variável no `.env`/ambiente desliga de vez.
fn splash_enabled() -> bool {
    !matches!(std::env::var("OREIA_SPLASH").as_deref().map(str::trim), Ok("0" | "false" | "off" | "no"))
}

type Payload = (Answers, TomlEdits);

/// Como a TUI terminou.
enum Exit {
    /// Saiu sem gravar nada.
    Quit,
    /// Gravar, e (se houver launcher) pedir a ação em seguida.
    Save(Box<Payload>, LauncherAction),
    /// Só pedir uma ação ao launcher.
    Action(LauncherAction),
}

struct Paths {
    env: PathBuf,
    toml: PathBuf,
}

pub async fn run(
    env_path: &Path,
    env_example_path: &Path,
    toml_path: &Path,
    toml_example_path: &Path,
    bootstrap_script: &Path,
    force_wizard: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        io::stdout().is_terminal() && io::stdin().is_terminal(),
        "a tela interativa precisa de um terminal de verdade. Para configurar sem terminal (script, CI, agente de IA), \
         use \"oreia-seca setup --answers ARQUIVO.toml\" — veja docs/instalacao.md#instalação-automatizada"
    );

    let state = current_state::load(env_path, toml_path);
    let first_run = force_wizard || wizard::is_first_run(env_path.exists(), &EnvMap::load(env_path));
    let action_file = std::env::var("OREIA_ACTION_FILE").ok().filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    let handle = tokio::runtime::Handle::current();
    let paths = Paths { env: env_path.to_path_buf(), toml: toml_path.to_path_buf() };
    let has_launcher = action_file.is_some();

    // O loop de eventos de uma TUI é síncrono e bloqueante (`event::poll`/
    // `read()`); roda no pool de threads bloqueantes do tokio para poder
    // chamar `Handle::block_on` dali (ilegal direto de dentro de uma
    // `async fn` rodando no runtime) quando precisa buscar `/models` do LLM.
    let exit = tokio::task::spawn_blocking(move || run_blocking(state, handle, paths, first_run, has_launcher))
        .await
        .map_err(|e| anyhow::anyhow!("a tela encerrou de forma inesperada: {e}"))??;

    match exit {
        Exit::Quit => println!("Nada foi alterado."),
        Exit::Action(action) => hand_off(action_file.as_deref(), action)?,
        Exit::Save(payload, action) => {
            let (answers, edits) = *payload;
            toml_edits::apply(toml_path, toml_example_path, &edits)?;
            let report = apply_answers(&answers, env_path, env_example_path, bootstrap_script).await?;
            print_report(&report);
            hand_off(action_file.as_deref(), action)?;
        }
    }
    Ok(())
}

/// `oreia-seca saude`: a mesma checagem da área Saúde do painel, uma vez só e
/// em texto puro — para script, CI, cron ou um terminal sem tela interativa.
/// Devolve `false` se algum serviço configurado está fora do ar ou com problema.
pub async fn print_health(env_path: &Path, toml_path: &Path) -> bool {
    use monitor::HealthState;
    let sample = monitor::collect(env_path, toml_path, true).await;
    let health = sample.health.unwrap_or_default();
    println!(
        "Saúde dos serviços do OREIA-SECA
"
    );
    for h in &health {
        let mark = match h.state {
            HealthState::Up => "✅",
            HealthState::Degraded => "⚠️ ",
            HealthState::Down => "❌",
            HealthState::Off => "⚪",
        };
        let latency = h.latency_ms.map(|ms| format!("{ms} ms")).unwrap_or_else(|| "—".into());
        println!("  {mark} {:<16} {:>8}   {}", h.name, latency, h.detail);
    }
    if let Some(api) = &sample.api {
        println!(
            "
  Fila: {} rodando · {} na espera · {} concluída(s) · {} falha(s) · {} aguardando aprovação",
            api.running, api.queued, api.completed, api.failed, api.pending_approvals
        );
    }
    let problems = health.iter().filter(|h| matches!(h.state, HealthState::Down | HealthState::Degraded)).count();
    println!();
    if sample.snapshot.error.is_none() && !sample.snapshot.any_running() {
        println!("Os serviços do OREIA-SECA estão parados — suba com \"./oreia up\" (ou \"./oreia install\", se for a primeira vez).");
    }
    if problems == 0 {
        println!("Tudo o que está configurado respondeu. (⚪ = desligado ou opcional, não é problema)");
    } else {
        println!("{problems} serviço(s) precisam de atenção — veja docs/problemas.md, ou o painel: ./oreia");
    }
    problems == 0
}

/// Com launcher: grava a ação para ele executar. Sem launcher: diz o comando.
fn hand_off(action_file: Option<&Path>, action: LauncherAction) -> anyhow::Result<()> {
    match action_file {
        Some(path) => std::fs::write(path, action.as_str())
            .map_err(|e| anyhow::anyhow!("não consegui gravar {}: {e}", path.display())),
        None => {
            match action {
                LauncherAction::Install => println!(
                    "\nConfiguração gravada. Para instalar de fato (Forgejo, contas, repositório e orquestrador): \
                     \"./oreia install\"."
                ),
                LauncherAction::Saved => println!(
                    "\nNada é reiniciado sozinho — rode \"./oreia restart\" (ou \"docker compose restart oreia-seca\") \
                     para aplicar."
                ),
                other => println!("\nRode \"{}\" no terminal.", other.manual_command()),
            }
            Ok(())
        }
    }
}

fn short_error(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    if text.contains("error sending request") || text.contains("tcp connect") || text.contains("dns error") {
        "sem resposta nesse endereço".to_string()
    } else if text.contains("401") || text.contains("403") {
        "a chave da API foi recusada".to_string()
    } else {
        text
    }
}

fn fetch_models(handle: &tokio::runtime::Handle, url: &str, api_key: Option<&str>) -> anyhow::Result<Vec<String>> {
    let in_container = status::probe_host() != "localhost";
    let reachable = monitor::reachable_url(url, in_container);
    handle.block_on(super::llm::fetch_models(&reachable, api_key))
}

fn run_blocking(
    state: current_state::CurrentState,
    handle: tokio::runtime::Handle,
    paths: Paths,
    first_run: bool,
    has_launcher: bool,
) -> anyhow::Result<Exit> {
    install_panic_hook();
    let mut app = App::new(&state);
    app.actions_enabled = has_launcher;
    if first_run {
        app.wizard = Some(Wizard::new(&state, &EnvMap::load(&paths.env)));
        app.screen = Screen::Wizard;
    }
    // Assistente aberto no lugar do painel (primeira instalação): cancelar sai
    // da TUI. Aberto POR CIMA do painel (tecla 'a'): cancelar volta ao painel.
    let mut wizard_standalone = first_run;
    if splash_enabled() {
        app.screen = Screen::Splash;
    }
    let splash_ms = mascot::splash_duration_ms();
    let opened = std::time::Instant::now();

    let (tx, rx) = std::sync::mpsc::channel();
    let monitor = monitor::spawn(&handle, paths.env.clone(), paths.toml.clone(), tx);
    let mut guard = TerminalGuard::new()?;

    loop {
        while let Ok(sample) = rx.try_recv() {
            app.absorb(sample);
        }
        app.anim_ms = opened.elapsed().as_millis() as u64;
        if app.screen == Screen::Splash && app.anim_ms >= splash_ms {
            app.leave_splash();
        }
        guard.terminal.draw(|frame| ui::draw(frame, &app))?;

        let wait = if app.screen == Screen::Splash { SPLASH_POLL_INTERVAL } else { POLL_INTERVAL };
        if !crossterm::event::poll(wait)? {
            continue;
        }
        let Event::Key(key) = crossterm::event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match event::handle_key(&mut app, key) {
            AppOutcome::Continue => {}
            // Em caso de erro, a própria tela de resumo (`ui::draw_summary`)
            // já mostra a mensagem ao recalcular `build_save_payload`; aqui
            // só decide se sai do loop ou não.
            AppOutcome::Save => {
                if let Ok(payload) = app.build_save_payload() {
                    return Ok(Exit::Save(Box::new(payload), LauncherAction::Saved));
                }
            }
            AppOutcome::Cancel => return Ok(Exit::Quit),
            AppOutcome::Action(action) => return Ok(Exit::Action(action)),
            AppOutcome::RefreshStatus => {
                monitor.refresh_now();
                app.flash = Some("Atualizando…".into());
            }
            AppOutcome::FetchModels { url, api_key } => {
                app.llm_fetch_status = Some(format!("buscando modelos em {url}..."));
                guard.terminal.draw(|frame| ui::draw(frame, &app))?;
                match fetch_models(&handle, &url, api_key.as_deref()) {
                    Ok(models) if models.is_empty() => {
                        app.llm_fetch_status = Some(format!("{url}/models respondeu sem nenhum modelo listado."));
                    }
                    Ok(models) => {
                        app.llm_fetch_status = Some(format!("{} modelo(s) encontrado(s).", models.len()));
                        app.llm_models = models;
                    }
                    Err(e) => {
                        app.llm_fetch_status = Some(format!("não consegui listar os modelos: {}", short_error(&e)));
                        app.llm_models.clear();
                    }
                }
            }
            AppOutcome::OpenWizard => {
                // Relê do disco: o que estiver salvo é o ponto de partida do assistente.
                let fresh = current_state::load(&paths.env, &paths.toml);
                app.wizard = Some(Wizard::new(&fresh, &EnvMap::load(&paths.env)));
                app.screen = Screen::Wizard;
                wizard_standalone = false;
            }
            AppOutcome::Wizard(outcome) => match outcome {
                WizardOutcome::Continue => {}
                WizardOutcome::Cancel if wizard_standalone => return Ok(Exit::Quit),
                WizardOutcome::Cancel => {
                    app.wizard = None;
                    app.screen = Screen::Editing;
                    app.flash = Some("Assistente fechado — nada foi gravado.".into());
                }
                WizardOutcome::FetchModels { url, api_key } => {
                    guard.terminal.draw(|frame| ui::draw(frame, &app))?;
                    let result = fetch_models(&handle, &url, api_key.as_deref()).map_err(|e| short_error(&e));
                    if let Some(w) = app.wizard.as_mut() {
                        w.on_models(result);
                    }
                }
                WizardOutcome::PlanPorts => {
                    // Com os nossos containers já no ar, as portas "ocupadas" somos nós mesmos.
                    let ours_running = status::project_running();
                    let host = status::probe_host();
                    if let Some(w) = app.wizard.as_mut() {
                        let plan =
                            wizard::plan_ports(&w.current_ports, &|p| !ours_running && status::is_port_open(&host, p));
                        w.on_ports(plan);
                    }
                }
                WizardOutcome::Finish => {
                    if let Some(Ok(payload)) = app.wizard.as_ref().map(Wizard::payload) {
                        return Ok(Exit::Save(Box::new(payload), LauncherAction::Install));
                    }
                }
            },
        }
    }
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        crossterm::execute!(stdout, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
    }
}

/// Defesa extra sobre o `Drop` do guard: garante o terminal restaurado mesmo
/// se algo entrar em panic no meio do loop de eventos. Instalado uma vez por
/// invocação de `oreia-seca setup` — não precisa ser desfeito, o processo
/// termina logo depois.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
        original(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hand_off_writes_the_action_name_for_the_launcher() {
        let path = std::env::temp_dir().join(format!("oreia-seca-action-{}", uuid::Uuid::new_v4().simple()));
        hand_off(Some(&path), LauncherAction::Install).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "install");
        hand_off(Some(&path), LauncherAction::Up).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "up");
        std::fs::remove_file(&path).ok();

        // Sem launcher não há arquivo nenhum para gravar — só imprime o comando.
        hand_off(None, LauncherAction::Saved).unwrap();
    }

    #[test]
    fn short_error_turns_network_noise_into_one_phrase() {
        let net = anyhow::anyhow!("error sending request for url (http://x/v1/models)");
        assert_eq!(short_error(&net), "sem resposta nesse endereço");
        let auth = anyhow::anyhow!("HTTP status client error (401 Unauthorized) for url (http://x)");
        assert!(short_error(&auth).contains("recusada"));
        assert_eq!(short_error(&anyhow::anyhow!("outra coisa")), "outra coisa");
    }
}
