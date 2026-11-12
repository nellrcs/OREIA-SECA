//! Mapeamento de tecla → ação. `handle_key` é pura (recebe um `KeyEvent` de
//! dados, não lê o terminal) e por isso testável sem ratatui — ao contrário
//! do loop de `crossterm::event::read()` em si, que fica em `mod.rs`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, AreaKind, Focus, LauncherAction, Screen, TextField};
use super::wizard::{self, WizardOutcome};

/// O que `mod.rs` deve fazer depois de uma tecla. `FetchModels` devolve os
/// dados em vez de buscar aqui dentro, para `handle_key` continuar síncrona.
pub enum AppOutcome {
    Continue,
    Save,
    Cancel,
    FetchModels {
        url: String,
        api_key: Option<String>,
    },
    RefreshStatus,
    /// Sair e deixar o launcher executar (subir, parar, bootstrap...).
    Action(LauncherAction),
    /// Abrir o assistente de primeira instalação por cima do painel.
    OpenWizard,
    /// Tecla tratada pelo assistente; `mod.rs` executa o que ele pedir.
    Wizard(WizardOutcome),
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> AppOutcome {
    app.flash = None;
    if app.screen == Screen::Wizard {
        return match app.wizard.as_mut() {
            Some(w) => AppOutcome::Wizard(wizard::handle_key(w, key)),
            None => {
                app.screen = Screen::Editing;
                AppOutcome::Continue
            }
        };
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return AppOutcome::Cancel;
    }
    if app.screen == Screen::Editing
        && (key.code == KeyCode::F(2)
            || (key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL)))
    {
        app.screen = Screen::Summary;
        return AppOutcome::Continue;
    }

    match app.screen {
        Screen::Wizard => AppOutcome::Continue, // tratado acima
        Screen::Summary => handle_summary_key(app, key),
        Screen::Editing => match app.focus {
            Focus::AreaList => handle_area_list_key(app, key),
            Focus::Field(idx) => handle_field_key(app, idx, key),
        },
    }
}

fn handle_summary_key(app: &mut App, key: KeyEvent) -> AppOutcome {
    match key.code {
        KeyCode::Enter | KeyCode::Char('s') | KeyCode::Char('S') => AppOutcome::Save,
        KeyCode::Esc => {
            app.screen = Screen::Editing;
            AppOutcome::Continue
        }
        _ => AppOutcome::Continue,
    }
}

fn handle_area_list_key(app: &mut App, key: KeyEvent) -> AppOutcome {
    let len = AreaKind::ALL.len();
    match key.code {
        // Só nas áreas de monitoramento (Status/Saúde/Uso), onde não há campo
        // de texto disputando as letras. `u`/`d`/`i`/`b`/`l` NÃO chamam o
        // Docker daqui: a TUI pode estar rodando num container sem `docker
        // compose` (ver `super::status`). Elas pedem ao launcher `./oreia` —
        // a TUI sai, ele executa no host e reabre o painel (`LauncherAction`).
        // Vem ANTES das teclas de navegação porque `l` (logs) também é o
        // "direita" do vim; `j`/`k` continuam navegando.
        KeyCode::Char(c) if app.selected_area_kind().is_monitor() && !matches!(c, 'j' | 'k') => {
            return match c.to_ascii_lowercase() {
                'r' => AppOutcome::RefreshStatus,
                'u' => request_action(app, LauncherAction::Up),
                'd' => request_action(app, LauncherAction::Down),
                'i' => request_action(app, LauncherAction::Restart),
                'b' => request_action(app, LauncherAction::Bootstrap),
                'l' => request_action(app, LauncherAction::Logs),
                'a' => AppOutcome::OpenWizard,
                'q' => AppOutcome::Cancel,
                _ => AppOutcome::Continue,
            };
        }
        KeyCode::Up | KeyCode::Char('k') => app.selected_area = (app.selected_area + len - 1) % len,
        KeyCode::Down | KeyCode::Char('j') => app.selected_area = (app.selected_area + 1) % len,
        KeyCode::Enter | KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
            // `Status` não tem campo nenhum — é só leitura, atualiza sozinha.
            if App::field_count(app.selected_area_kind()) > 0 {
                app.focus = Focus::Field(0);
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => return AppOutcome::Cancel,
        _ => {}
    }
    AppOutcome::Continue
}

/// Só deixa a TUI sair para o launcher quando isso não perde nada (sem edição
/// pendente) e quando há um launcher do outro lado para executar.
fn request_action(app: &mut App, action: LauncherAction) -> AppOutcome {
    if app.is_dirty() {
        app.flash = Some("Há alterações não salvas — salve antes com Ctrl+S (ou saia com Esc para descartar).".into());
        AppOutcome::Continue
    } else if !app.actions_enabled {
        app.flash = Some(format!(
            "Esta tela foi aberta sem o launcher — saia (q) e rode \"{}\" no terminal.",
            action.manual_command()
        ));
        AppOutcome::Continue
    } else {
        AppOutcome::Action(action)
    }
}

fn handle_field_key(app: &mut App, idx: usize, key: KeyEvent) -> AppOutcome {
    if key.code == KeyCode::Esc {
        app.focus = Focus::AreaList;
        return AppOutcome::Continue;
    }

    let leaving = matches!(key.code, KeyCode::Tab | KeyCode::Down | KeyCode::BackTab | KeyCode::Up | KeyCode::Enter);
    if leaving {
        let area = app.selected_area_kind();
        let count = App::field_count(area);
        let fetch = (area == AreaKind::Llm && idx == 0 && !app.llm.url.value.is_empty()).then(|| {
            let url = app.llm.url.value.clone();
            let api_key = (!app.llm.api_key.value.is_empty()).then(|| app.llm.api_key.value.clone());
            (url, api_key)
        });

        let forward = !matches!(key.code, KeyCode::BackTab | KeyCode::Up);
        app.focus = Focus::Field(if forward { (idx + 1) % count } else { (idx + count - 1) % count });

        return match fetch {
            Some((url, api_key)) => AppOutcome::FetchModels { url, api_key },
            None => AppOutcome::Continue,
        };
    }

    match app.selected_area_kind() {
        AreaKind::Status | AreaKind::Health | AreaKind::Usage => {}
        AreaKind::Px0 => handle_px0_field(app, key),
        AreaKind::Forgejo => handle_forgejo_field(app, idx, key),
        AreaKind::Mcp => handle_mcp_field(app, idx, key),
        AreaKind::Telegram => handle_telegram_field(app, idx, key),
        AreaKind::Llm => handle_llm_field(app, idx, key),
        AreaKind::Executor => handle_executor_field(app, idx, key),
        AreaKind::Server => handle_server_field(app, idx, key),
    }
    AppOutcome::Continue
}

fn handle_px0_field(app: &mut App, key: KeyEvent) {
    edit_text(&mut app.px0.allowed_users, key);
}

fn handle_forgejo_field(app: &mut App, idx: usize, key: KeyEvent) {
    match idx {
        0 => edit_text(&mut app.forgejo.url, key),
        1 => edit_text(&mut app.forgejo.public_url, key),
        2 => edit_text(&mut app.forgejo.repos, key),
        3 => edit_text(&mut app.forgejo.base_branch, key),
        4 => edit_text(&mut app.forgejo.merge_style, key),
        _ => {}
    }
}

fn handle_mcp_field(app: &mut App, idx: usize, key: KeyEvent) {
    match idx {
        0 if key.code == KeyCode::Char(' ') => app.mcp.enabled = !app.mcp.enabled,
        1 if key.code == KeyCode::Char(' ') => app.mcp.http = !app.mcp.http,
        2 => edit_text(&mut app.mcp.agent_user, key),
        3 => edit_text(&mut app.mcp.workdir, key),
        4 => edit_text(&mut app.mcp.max_review_rounds, key),
        5 => edit_text(&mut app.mcp.bind, key),
        _ => {}
    }
}

fn handle_telegram_field(app: &mut App, idx: usize, key: KeyEvent) {
    match idx {
        0 if key.code == KeyCode::Char(' ') => app.telegram.enabled = !app.telegram.enabled,
        1 => edit_text(&mut app.telegram.token, key),
        2 => edit_text(&mut app.telegram.allowed, key),
        _ => {}
    }
}

fn handle_llm_field(app: &mut App, idx: usize, key: KeyEvent) {
    match idx {
        0 => edit_text(&mut app.llm.url, key),
        1 => edit_text(&mut app.llm.coder_model, key),
        2 => edit_text(&mut app.llm.chat_model, key),
        3 => edit_text(&mut app.llm.api_key, key),
        4 => edit_text(&mut app.llm.coder_context, key),
        5 => edit_text(&mut app.llm.coder_temperature, key),
        6 => edit_text(&mut app.llm.chat_context, key),
        7 => edit_text(&mut app.llm.chat_temperature, key),
        _ => {}
    }
}

fn handle_server_field(app: &mut App, idx: usize, key: KeyEvent) {
    match idx {
        0 => edit_text(&mut app.server.host, key),
        1 => edit_text(&mut app.server.port, key),
        2 => edit_text(&mut app.server.paths_data, key),
        3 => edit_text(&mut app.server.paths_workspace, key),
        4 => edit_text(&mut app.server.queue_concurrency, key),
        5 => edit_text(&mut app.server.queue_timeout_secs, key),
        _ => {}
    }
}

pub const EXECUTOR_PRESETS: &[&str] = &["builtin", "opencode", "codex", "aider", "goose"];

fn cycle_executor_preset(field: &mut TextField) {
    let current = field.value.trim();
    let next = match EXECUTOR_PRESETS.iter().position(|&p| p == current) {
        Some(i) => EXECUTOR_PRESETS[(i + 1) % EXECUTOR_PRESETS.len()],
        None => EXECUTOR_PRESETS[0],
    };
    field.value = next.to_string();
}

fn handle_executor_field(app: &mut App, idx: usize, key: KeyEvent) {
    match idx {
        0 => {
            if key.code == KeyCode::Char(' ') {
                cycle_executor_preset(&mut app.executor.default);
            } else {
                edit_text(&mut app.executor.default, key);
            }
        }
        1 => edit_text(&mut app.executor.max_steps, key),
        2 => edit_text(&mut app.executor.phase_retries, key),
        3 => edit_text(&mut app.executor.shell_timeout_secs, key),
        4 => edit_text(&mut app.executor.max_phases, key),
        5 => edit_text(&mut app.executor.step_timeout_secs, key),
        _ => {}
    }
}

fn edit_text(field: &mut TextField, key: KeyEvent) {
    match key.code {
        KeyCode::Char(c) => field.push(c),
        KeyCode::Backspace => field.backspace(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::current_state::CurrentState;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn app() -> App {
        App::new(&CurrentState::default())
    }

    fn area_index(kind: AreaKind) -> usize {
        AreaKind::ALL.iter().position(|a| *a == kind).unwrap()
    }

    #[test]
    fn ctrl_c_cancels_from_anywhere() {
        let mut a = app();
        a.focus = Focus::Field(0);
        assert!(matches!(handle_key(&mut a, ctrl(KeyCode::Char('c'))), AppOutcome::Cancel));
    }

    #[test]
    fn esc_on_area_list_cancels() {
        let mut a = app();
        assert!(matches!(handle_key(&mut a, key(KeyCode::Esc)), AppOutcome::Cancel));
    }

    #[test]
    fn esc_inside_a_field_returns_to_the_area_list_without_cancelling() {
        let mut a = app();
        a.focus = Focus::Field(0);
        assert!(matches!(handle_key(&mut a, key(KeyCode::Esc)), AppOutcome::Continue));
        assert_eq!(a.focus, Focus::AreaList);
    }

    #[test]
    fn enter_on_the_status_area_does_not_enter_field_mode() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Status);
        handle_key(&mut a, key(KeyCode::Enter));
        assert_eq!(a.focus, Focus::AreaList, "Status não tem campo nenhum pra entrar");
    }

    #[test]
    fn down_walks_the_monitor_areas_and_enter_moves_into_the_px0_panel() {
        let mut a = app();
        for expected in [AreaKind::Health, AreaKind::Usage, AreaKind::Px0] {
            handle_key(&mut a, key(KeyCode::Down));
            assert_eq!(a.selected_area_kind(), expected);
        }
        handle_key(&mut a, key(KeyCode::Enter));
        assert_eq!(a.focus, Focus::Field(0));
    }

    #[test]
    fn action_keys_reach_the_launcher_only_from_monitor_areas() {
        let mut a = app();
        a.actions_enabled = true;
        for (c, expected) in [
            ('u', LauncherAction::Up),
            ('d', LauncherAction::Down),
            ('i', LauncherAction::Restart),
            ('b', LauncherAction::Bootstrap),
            ('L', LauncherAction::Logs),
        ] {
            a.selected_area = area_index(AreaKind::Health);
            match handle_key(&mut a, key(KeyCode::Char(c))) {
                AppOutcome::Action(action) => assert_eq!(action, expected),
                _ => panic!("esperava Action para '{c}'"),
            }
        }
        a.selected_area = area_index(AreaKind::Forgejo);
        assert!(matches!(handle_key(&mut a, key(KeyCode::Char('u'))), AppOutcome::Continue));
    }

    #[test]
    fn action_keys_explain_themselves_without_a_launcher() {
        let mut a = app();
        assert!(matches!(handle_key(&mut a, key(KeyCode::Char('u'))), AppOutcome::Continue));
        assert!(a.flash.as_deref().unwrap().contains("./oreia up"), "{:?}", a.flash);
        handle_key(&mut a, key(KeyCode::Down));
        assert!(a.flash.is_none(), "o aviso some na próxima tecla");
    }

    #[test]
    fn action_keys_refuse_to_drop_unsaved_edits() {
        let mut a = app();
        a.actions_enabled = true;
        a.px0.allowed_users.push('x');
        assert!(matches!(handle_key(&mut a, key(KeyCode::Char('d'))), AppOutcome::Continue));
        assert!(a.flash.as_deref().unwrap().contains("não salvas"));
    }

    #[test]
    fn a_opens_the_wizard_and_its_keys_are_routed_to_it() {
        let mut a = app();
        assert!(matches!(handle_key(&mut a, key(KeyCode::Char('a'))), AppOutcome::OpenWizard));

        a.wizard = Some(crate::setup::tui::wizard::Wizard::new(
            &CurrentState::default(),
            &crate::setup::tui::monitor::EnvMap::default(),
        ));
        a.screen = Screen::Wizard;
        assert!(matches!(handle_key(&mut a, key(KeyCode::Enter)), AppOutcome::Wizard(WizardOutcome::Continue)));
        assert!(matches!(handle_key(&mut a, key(KeyCode::Char('q'))), AppOutcome::Wizard(WizardOutcome::Continue)));
        assert!(matches!(handle_key(&mut a, ctrl(KeyCode::Char('c'))), AppOutcome::Wizard(WizardOutcome::Cancel)));
    }

    #[test]
    fn space_toggles_mcp_enabled_and_tab_cycles_through_all_six_fields() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Mcp);
        a.focus = Focus::Field(0);
        assert!(!a.mcp.enabled);
        handle_key(&mut a, key(KeyCode::Char(' ')));
        assert!(a.mcp.enabled);

        handle_key(&mut a, key(KeyCode::Tab));
        assert_eq!(a.focus, Focus::Field(1));
        handle_key(&mut a, key(KeyCode::Char(' ')));
        assert!(a.mcp.http);

        for expected in 2..6 {
            handle_key(&mut a, key(KeyCode::Tab));
            assert_eq!(a.focus, Focus::Field(expected));
        }
        handle_key(&mut a, key(KeyCode::Tab));
        assert_eq!(a.focus, Focus::Field(0), "tab no último campo volta pro primeiro");
    }

    #[test]
    fn typing_edits_mcp_workdir_field() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Mcp);
        a.focus = Focus::Field(3); // workdir
        for c in "ws".chars() {
            handle_key(&mut a, key(KeyCode::Char(c)));
        }
        assert_eq!(a.mcp.workdir.value, "ws");
    }

    #[test]
    fn typing_edits_forgejo_repos_field() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Forgejo);
        a.focus = Focus::Field(2); // repos
        for c in "a/b".chars() {
            handle_key(&mut a, key(KeyCode::Char(c)));
        }
        assert_eq!(a.forgejo.repos.value, "a/b");
    }

    #[test]
    fn typing_edits_server_port_field() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Server);
        a.focus = Focus::Field(1); // port
        for c in "4000".chars() {
            handle_key(&mut a, key(KeyCode::Char(c)));
        }
        assert_eq!(a.server.port.value, "4000");
    }

    #[test]
    fn typing_edits_telegram_allowed_field() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Telegram);
        a.focus = Focus::Field(2); // allowed
        handle_key(&mut a, key(KeyCode::Char('1')));
        handle_key(&mut a, key(KeyCode::Char(',')));
        handle_key(&mut a, key(KeyCode::Char('2')));
        handle_key(&mut a, key(KeyCode::Backspace));
        assert_eq!(a.telegram.allowed.value, "1,");
    }

    #[test]
    fn leaving_llm_url_field_with_a_value_triggers_fetch_models() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Llm);
        a.focus = Focus::Field(0);
        for c in "http://x/v1".chars() {
            handle_key(&mut a, key(KeyCode::Char(c)));
        }
        match handle_key(&mut a, key(KeyCode::Tab)) {
            AppOutcome::FetchModels { url, api_key } => {
                assert_eq!(url, "http://x/v1");
                assert_eq!(api_key, None);
            }
            _ => panic!("esperava FetchModels"),
        }
        assert_eq!(a.focus, Focus::Field(1));
    }

    #[test]
    fn leaving_llm_url_field_empty_does_not_trigger_fetch() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Llm);
        a.focus = Focus::Field(0);
        assert!(matches!(handle_key(&mut a, key(KeyCode::Tab)), AppOutcome::Continue));
    }

    #[test]
    fn typing_edits_llm_coder_temperature_field() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Llm);
        a.focus = Focus::Field(5); // coder_temperature
        for c in "0.3".chars() {
            handle_key(&mut a, key(KeyCode::Char(c)));
        }
        assert_eq!(a.llm.coder_temperature.value, "0.3");
    }

    #[test]
    fn ctrl_s_jumps_to_summary_and_enter_saves() {
        let mut a = app();
        handle_key(&mut a, ctrl(KeyCode::Char('s')));
        assert_eq!(a.screen, Screen::Summary);
        assert!(matches!(handle_key(&mut a, key(KeyCode::Enter)), AppOutcome::Save));
    }

    #[test]
    fn esc_on_summary_goes_back_to_editing_without_saving() {
        let mut a = app();
        a.screen = Screen::Summary;
        assert!(matches!(handle_key(&mut a, key(KeyCode::Esc)), AppOutcome::Continue));
        assert_eq!(a.screen, Screen::Editing);
    }

    #[test]
    fn r_triggers_refresh_status_only_on_the_monitor_areas() {
        let mut a = app();
        for kind in [AreaKind::Status, AreaKind::Health, AreaKind::Usage] {
            a.selected_area = area_index(kind);
            assert!(matches!(handle_key(&mut a, key(KeyCode::Char('r'))), AppOutcome::RefreshStatus));
        }

        a.selected_area = area_index(AreaKind::Px0);
        assert!(matches!(handle_key(&mut a, key(KeyCode::Char('r'))), AppOutcome::Continue));
    }

    #[test]
    fn space_cycles_executor_preset_and_typing_edits_fields() {
        let mut a = app();
        a.selected_area = area_index(AreaKind::Executor);
        a.focus = Focus::Field(0);
        a.executor.default.value = "builtin".to_string();

        handle_key(&mut a, key(KeyCode::Char(' ')));
        assert_eq!(a.executor.default.value, "opencode");
        handle_key(&mut a, key(KeyCode::Char(' ')));
        assert_eq!(a.executor.default.value, "codex");
        handle_key(&mut a, key(KeyCode::Char(' ')));
        assert_eq!(a.executor.default.value, "aider");
        handle_key(&mut a, key(KeyCode::Char(' ')));
        assert_eq!(a.executor.default.value, "goose");
        handle_key(&mut a, key(KeyCode::Char(' ')));
        assert_eq!(a.executor.default.value, "builtin");

        // Digitação de outro campo (ex.: max_steps)
        a.focus = Focus::Field(1);
        a.executor.max_steps.value.clear();
        handle_key(&mut a, key(KeyCode::Char('5')));
        assert_eq!(a.executor.max_steps.value, "5");
    }
}
