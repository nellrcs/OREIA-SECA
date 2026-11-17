//! Renderização da TUI de `oreia-seca setup`. Só lê `App`, nunca o modifica.
//! Tudo aqui desenha num `Frame`; os testes no fim do arquivo usam o
//! `TestBackend` do ratatui para conferir que cada tela cabe e mostra o que
//! deve, sem precisar de um terminal de verdade.

use chrono::Local;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::app::{App, AreaKind, Focus, Screen};
use super::mascot;
use super::monitor::{HealthState, ServiceHealth};
use super::status::{CheckLevel, ContainerRow};
use super::usage::{human_cost, human_duration, human_tokens};
use super::wizard;

pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Splash => mascot::draw_splash(frame, app.anim_ms),
        Screen::Editing => draw_editing(frame, app),
        Screen::Summary => draw_summary(frame, app),
        Screen::Wizard => match &app.wizard {
            Some(w) => wizard::draw(frame, w),
            None => draw_editing(frame, app),
        },
    }
}

fn draw_editing(frame: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(3)])
        .split(frame.area());
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(20), Constraint::Min(0)])
        .split(root[1]);

    draw_header(frame, root[0], app);
    draw_area_list(frame, cols[0], app);
    draw_panel(frame, cols[1], app);
    draw_help(frame, root[2], app);
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn titled(title: &str) -> Block<'static> {
    Block::default().borders(Borders::ALL).title(format!(" {title} "))
}

/// Uma linha no topo com o resumo que interessa de relance, visível em qualquer área.
fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let m = &app.monitor;
    let mut spans = vec![Span::styled(" OREIA-SECA ", bold().fg(Color::Black).bg(Color::Cyan)), Span::raw("  ")];

    let checked: Vec<&ServiceHealth> = m.health.iter().filter(|h| h.state != HealthState::Off).collect();
    if checked.is_empty() {
        spans.push(Span::styled("verificando serviços…", dim()));
    } else {
        let up = checked.iter().filter(|h| h.state == HealthState::Up).count();
        let color = if up == checked.len() {
            Color::Green
        } else if up == 0 {
            Color::Red
        } else {
            Color::Yellow
        };
        spans.push(Span::styled(format!("● {up}/{} serviços no ar", checked.len()), Style::default().fg(color)));
    }
    if let Some(api) = &m.api {
        spans.push(Span::styled("  ·  ", dim()));
        spans.push(Span::raw(format!("fila: {} rodando, {} na espera", api.running, api.queued)));
        if api.pending_approvals > 0 {
            spans.push(Span::styled(
                format!("  ·  {} aprovação(ões) pendente(s)", api.pending_approvals),
                Style::default().fg(Color::Yellow),
            ));
        }
    }
    if let Some(at) = m.updated_at {
        spans.push(Span::styled(format!("  ·  atualizado {}", at.format("%H:%M:%S")), dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_area_list(frame: &mut Frame, area: Rect, app: &App) {
    let mut items: Vec<ListItem> = Vec::new();
    for (i, kind) in AreaKind::ALL.iter().enumerate() {
        // Dois grupos: o que você OLHA e o que você CONFIGURA.
        if *kind == AreaKind::Status {
            items.push(ListItem::new(Line::from(Span::styled("PAINEL", dim().add_modifier(Modifier::BOLD)))));
        } else if *kind == AreaKind::Px0 {
            items.push(ListItem::new(""));
            items.push(ListItem::new(Line::from(Span::styled("CONFIGURAR", dim().add_modifier(Modifier::BOLD)))));
        }
        let (mark, color) = area_indicator(*kind, app);
        let selected = i == app.selected_area;
        let style = if selected && app.focus == Focus::AreaList {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if selected {
            bold()
        } else {
            Style::default()
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!(" {mark} "), Style::default().fg(color)),
            Span::styled(kind.title(), style),
        ])));
    }
    let rows = items.len() as u16;
    frame.render_widget(List::new(items).block(Block::default().borders(Borders::ALL)), area);
    draw_sidebar_mascot(frame, area, rows, app);
}

/// O mascote no pé da lista de áreas, quando sobra altura. Em Status ele
/// mostra o estado da obra (martelando se há tarefa rodando, parado se não);
/// em cada outra área, uma atividade que tem a ver com ela.
fn draw_sidebar_mascot(frame: &mut Frame, area: Rect, list_rows: u16, app: &App) {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let needed = list_rows + 1 + mascot::HEIGHT + 1;
    if inner.height < needed || inner.width < mascot::WIDTH {
        return;
    }
    let running = app.monitor.api.as_ref().map(|a| a.running).unwrap_or(0);
    let activity = match app.selected_area_kind() {
        AreaKind::Status => None,
        AreaKind::Health => Some((mascot::Act::Pulse, "medindo o pulso")),
        AreaKind::Usage => Some((mascot::Act::Coin, "fazendo as contas")),
        AreaKind::Px0 => Some((mascot::Act::Review, "revisando o PR")),
        AreaKind::Forgejo => Some((mascot::Act::Carry, "carregando tijolo")),
        AreaKind::Mcp => Some((mascot::Act::Wave, "dando um alô")),
        AreaKind::Telegram => Some((mascot::Act::Message, "mandando recado")),
        AreaKind::Llm => Some((mascot::Act::Idea, "tendo uma ideia")),
        AreaKind::Executor => Some((mascot::Act::Wrench, "apertando parafuso")),
        AreaKind::Server => Some((mascot::Act::Server, "olho no servidor")),
    };
    let (pose, caption, color) = if let Some((act, caption)) = activity {
        (mascot::act_pose(act, app.anim_ms), caption.to_string(), Color::DarkGray)
    } else if running > 0 {
        (mascot::working_pose(app.anim_ms), format!("{running} em obra"), Color::Yellow)
    } else if app.monitor.api.is_some() {
        (mascot::idle_pose(app.anim_ms), "esperando serviço".to_string(), Color::DarkGray)
    } else if app.monitor.updated_at.is_none() {
        (mascot::idle_pose(app.anim_ms), "conferindo…".to_string(), Color::DarkGray)
    } else {
        (mascot::Pose::Blink, "de folga (parado)".to_string(), Color::DarkGray)
    };
    let top = inner.y + inner.height - mascot::HEIGHT - 1;
    // Corpo do boneco ocupa as 10 primeiras colunas; o que ele segura, as 2 últimas.
    let x = inner.x + (inner.width.saturating_sub(10)) / 2;
    mascot::render(frame, inner, x.min(inner.x + inner.width - mascot::WIDTH), top, pose);
    let caption_rect = Rect { x: inner.x, y: top + mascot::HEIGHT, width: inner.width, height: 1 };
    frame.render_widget(
        Paragraph::new(Span::styled(caption, Style::default().fg(color))).alignment(ratatui::layout::Alignment::Center),
        caption_rect,
    );
}

/// `●`/`○` para liga/desliga de verdade; `⚠`/`✓` para px0 e Forgejo, que não
/// têm toggle — são painéis de acesso/config (ver `docs/px0.md`). As três
/// áreas do painel mostram o pior estado do que elas vigiam.
fn area_indicator(kind: AreaKind, app: &App) -> (&'static str, Color) {
    match kind {
        AreaKind::Status => {
            if app.status.error.is_some() {
                ("✖", Color::Red)
            } else if app.status.any_running() {
                ("●", Color::Green)
            } else {
                ("○", Color::DarkGray)
            }
        }
        AreaKind::Health => {
            let worst = |s: HealthState| app.monitor.health.iter().any(|h| h.state == s);
            if app.monitor.health.is_empty() {
                ("…", Color::DarkGray)
            } else if worst(HealthState::Down) {
                ("✖", Color::Red)
            } else if worst(HealthState::Degraded) {
                ("▲", Color::Yellow)
            } else {
                ("♥", Color::Green)
            }
        }
        AreaKind::Usage => ("▤", Color::Blue),
        AreaKind::Px0 => {
            if app.px0.allowed_users.value.is_empty() {
                ("⚠", Color::Yellow)
            } else {
                ("✓", Color::Green)
            }
        }
        AreaKind::Forgejo => {
            if app.forgejo.url.value.is_empty() {
                ("○", Color::DarkGray)
            } else {
                ("✓", Color::Green)
            }
        }
        AreaKind::Mcp => {
            if app.mcp.enabled {
                ("●", Color::Green)
            } else {
                ("○", Color::DarkGray)
            }
        }
        AreaKind::Telegram => {
            if app.telegram.enabled {
                ("●", Color::Green)
            } else {
                ("○", Color::DarkGray)
            }
        }
        AreaKind::Llm => {
            if app.llm.url.value.is_empty() {
                ("○", Color::DarkGray)
            } else {
                ("●", Color::Green)
            }
        }
        AreaKind::Executor => ("●", Color::Green),
        AreaKind::Server => ("●", Color::Green),
    }
}

fn draw_panel(frame: &mut Frame, area: Rect, app: &App) {
    let focus_idx = match app.focus {
        Focus::Field(i) => Some(i),
        Focus::AreaList => None,
    };
    let lines = match app.selected_area_kind() {
        AreaKind::Status => return draw_status(frame, area, app),
        AreaKind::Health => return draw_health(frame, area, app),
        AreaKind::Usage => return draw_usage(frame, area, app),
        AreaKind::Px0 => px0_lines(app, focus_idx),
        AreaKind::Forgejo => forgejo_lines(app, focus_idx),
        AreaKind::Mcp => mcp_lines(app, focus_idx),
        AreaKind::Telegram => telegram_lines(app, focus_idx),
        AreaKind::Llm => llm_lines(app, focus_idx),
        AreaKind::Executor => executor_lines(app, focus_idx),
        AreaKind::Server => server_lines(app, focus_idx),
    };
    let title = app.selected_area_kind().title();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(titled(title)), area);
}

// ── Mini-gráficos ────────────────────────────────────────────────────────────

const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Série → barras de 1 caractere, as `width` mais recentes, alinhadas à
/// direita (o ponto mais novo fica sempre na última coluna). `None` = sem
/// resposta naquele ponto (`×`). A escala vai de 0 até o maior valor da
/// série, com `floor` de mínimo — para um serviço estável em 3 ms não
/// parecer uma montanha-russa.
pub(crate) fn sparkline(values: &[Option<f64>], floor: f64, width: usize) -> String {
    let tail = &values[values.len().saturating_sub(width)..];
    let max = tail.iter().flatten().copied().fold(floor, f64::max).max(f64::MIN_POSITIVE);
    let mut out: String = " ".repeat(width - tail.len());
    for v in tail {
        out.push(match v {
            None => '×',
            Some(v) => BARS[((v / max) * 7.0).round().clamp(0.0, 7.0) as usize],
        });
    }
    out
}

fn series(history: Option<&std::collections::VecDeque<f64>>) -> Vec<Option<f64>> {
    history.map(|h| h.iter().map(|v| Some(*v)).collect()).unwrap_or_default()
}

const SPARK_W: usize = 16;

// ── Área Status ──────────────────────────────────────────────────────────────

fn state_span(health: Option<&ServiceHealth>) -> Span<'static> {
    match health {
        None => Span::styled("verificando…", dim()),
        Some(h) => {
            let (text, color) = match h.state {
                HealthState::Up => ("no ar", Color::Green),
                HealthState::Degraded => ("com problema", Color::Yellow),
                HealthState::Down => ("fora do ar", Color::Red),
                HealthState::Off => ("desligado", Color::DarkGray),
            };
            let latency = h.latency_ms.map(|ms| format!(" · {ms} ms")).unwrap_or_default();
            Span::styled(format!("● {text}{latency}"), Style::default().fg(color))
        }
    }
}

fn access_line(label: &str, url: &str, health: Option<&ServiceHealth>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {label:<16}"), Style::default().fg(Color::Cyan)),
        Span::styled(format!("{url:<26}"), bold()),
        state_span(health),
    ])
}

fn container_color(c: &ContainerRow) -> Color {
    match (c.is_running(), c.healthy()) {
        (true, Some(false)) => Color::Yellow,
        (true, _) => Color::Green,
        (false, _) => Color::Red,
    }
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let m = &app.monitor;
    let containers_h = (app.status.containers.len() as u16 + 3).clamp(5, 10);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Length(containers_h), Constraint::Min(6)])
        .split(area);

    // Acesso + fila
    let mut access = vec![
        access_line("Forgejo (git)", &app.status.forgejo_url, m.health_of("forgejo")),
        access_line("px0 (revisão)", &app.status.px0_url, m.health_of("px0")),
        access_line("API", &app.status.api_url, m.health_of("api")),
    ];
    match &m.api {
        Some(api) => {
            let mut spans = vec![
                Span::styled(format!(" {:<16}", "Fila"), Style::default().fg(Color::Cyan)),
                Span::raw(format!(
                    "{} rodando · {} na espera · {} concluída(s) · {} falha(s)",
                    api.running, api.queued, api.completed, api.failed
                )),
            ];
            if api.pending_approvals > 0 {
                spans.push(Span::styled(
                    format!("  ·  {} aguardando /approve", api.pending_approvals),
                    Style::default().fg(Color::Yellow),
                ));
            }
            access.push(Line::from(spans));
            let work =
                if api.active_work.is_empty() { "nada no momento".to_string() } else { api.active_work.join(", ") };
            access.push(Line::from(vec![
                Span::styled(format!(" {:<16}", "Em andamento"), Style::default().fg(Color::Cyan)),
                Span::styled(work, dim()),
            ]));
        }
        None => access.push(Line::from(vec![
            Span::styled(format!(" {:<16}", "Fila"), Style::default().fg(Color::Cyan)),
            Span::styled("o orquestrador não respondeu — parado, ou ainda subindo", dim()),
        ])),
    }
    frame.render_widget(Paragraph::new(access).block(titled("Acesso")), rows[0]);

    // Containers
    if let Some(err) = &app.status.error {
        frame.render_widget(
            Paragraph::new(err.clone())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: false })
                .block(titled("Containers")),
            rows[1],
        );
    } else if m.updated_at.is_none() {
        frame.render_widget(Paragraph::new(" lendo o Docker…").style(dim()).block(titled("Containers")), rows[1]);
    } else if app.status.containers.is_empty() {
        frame.render_widget(
            Paragraph::new(
                " Nenhum container do OREIA-SECA foi criado ainda.\n Tecle 'u' para subir tudo (ou 'b' para o bootstrap do Forgejo, se o checklist abaixo pedir).",
            )
            .wrap(Wrap { trim: false })
            .block(titled("Containers")),
            rows[1],
        );
    } else {
        let header =
            Row::new(vec!["Serviço", "Estado", "CPU", "CPU (histórico)", "Memória", "Mem. (histórico)"]).style(bold());
        let table_rows: Vec<Row> = app
            .status
            .containers
            .iter()
            .map(|c| {
                let color = container_color(c);
                let state = if c.status_text.is_empty() { c.state.clone() } else { c.status_text.clone() };
                Row::new(vec![
                    Cell::from(c.service.clone()).style(Style::default().fg(color)),
                    Cell::from(state).style(Style::default().fg(color)),
                    Cell::from(c.cpu_pct.map(|v| format!("{v:.1}%")).unwrap_or_else(|| "—".into())),
                    Cell::from(sparkline(&series(m.cpu_history.get(&c.service)), 10.0, SPARK_W))
                        .style(Style::default().fg(Color::Cyan)),
                    Cell::from(c.mem_usage.clone().unwrap_or_else(|| "—".into())),
                    Cell::from(sparkline(&series(m.mem_history.get(&c.service)), 5.0, SPARK_W))
                        .style(Style::default().fg(Color::Magenta)),
                ])
            })
            .collect();
        let widths = [
            Constraint::Length(13),
            Constraint::Min(22),
            Constraint::Length(7),
            Constraint::Length(SPARK_W as u16 + 1),
            Constraint::Length(20),
            Constraint::Length(SPARK_W as u16),
        ];
        frame.render_widget(Table::new(table_rows, widths).header(header).block(titled("Containers")), rows[1]);
    }

    // Checklist
    let mut check_lines = Vec::new();
    for c in &app.status.checks {
        let (icon, color) = match c.level {
            CheckLevel::Ok => ("✓", Color::Green),
            CheckLevel::Warn => ("!", Color::Yellow),
            CheckLevel::Fail => ("✗", Color::Red),
        };
        check_lines.push(Line::from(vec![
            Span::styled(format!(" {icon} "), Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}: ", c.label), bold()),
            Span::styled(c.message.clone(), Style::default().fg(Color::Gray)),
        ]));
    }
    if !app.status.checks.is_empty() {
        check_lines.push(Line::from(""));
        let stopped: Vec<&str> =
            app.status.containers.iter().filter(|c| !c.is_running()).map(|c| c.service.as_str()).collect();
        check_lines.push(if app.status.any_running() && !stopped.is_empty() {
            Line::from(vec![
                Span::styled(format!(" Parado: {}. ", stopped.join(", ")), bold().fg(Color::Yellow)),
                Span::raw("Tecle "),
                Span::styled("u", bold().fg(Color::Cyan)),
                Span::raw(" para subir de novo, ou "),
                Span::styled("l", bold().fg(Color::Cyan)),
                Span::raw(" para ver os logs."),
            ])
        } else if app.status.any_running() && app.status.is_ready_for_up() {
            Line::from(Span::styled(" Tudo certo — os serviços estão rodando.", Style::default().fg(Color::Green)))
        } else if app.status.is_ready_for_up() {
            Line::from(vec![
                Span::styled(" Pronto para subir: ", bold().fg(Color::Green)),
                Span::raw("tecle "),
                Span::styled("u", bold().fg(Color::Cyan)),
                Span::raw(" (ou rode \"./oreia up\" no terminal)."),
            ])
        } else {
            Line::from(vec![
                Span::styled(" Falta resolver: ", bold().fg(Color::Red)),
                Span::raw("os itens com "),
                Span::styled("✗", bold().fg(Color::Red)),
                Span::raw(" acima. Primeira instalação? Tecle "),
                Span::styled("a", bold().fg(Color::Cyan)),
                Span::raw(" para o assistente."),
            ])
        });
    }
    frame.render_widget(Paragraph::new(check_lines).wrap(Wrap { trim: false }).block(titled("Checklist")), rows[2]);
}

// ── Área Saúde ───────────────────────────────────────────────────────────────

fn health_mark(state: HealthState) -> (&'static str, Color) {
    match state {
        HealthState::Up => ("●", Color::Green),
        HealthState::Degraded => ("▲", Color::Yellow),
        HealthState::Down => ("✖", Color::Red),
        HealthState::Off => ("○", Color::DarkGray),
    }
}

fn model_state_label(state: &str) -> (&'static str, Color) {
    match state {
        "ok" => ("ok", Color::Green),
        "unreachable" => ("inacessível", Color::Red),
        "not_configured" => ("não configurado", Color::DarkGray),
        _ => ("?", Color::DarkGray),
    }
}

fn draw_health(frame: &mut Frame, area: Rect, app: &App) {
    let m = &app.monitor;
    let models_h = m.api.as_ref().map(|a| (a.models.len() as u16 + 3).clamp(4, 9)).unwrap_or(3);
    let problems: Vec<&ServiceHealth> =
        m.health.iter().filter(|h| matches!(h.state, HealthState::Down | HealthState::Degraded)).collect();
    let table_h = (m.health.len() as u16 + 3).max(4);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(table_h), Constraint::Min(3), Constraint::Length(models_h)])
        .split(area);

    // O detalhe na tabela é cortado pela largura; aqui ele aparece inteiro, só do que precisa de atenção.
    let attention: Vec<Line> = if m.health.is_empty() {
        Vec::new()
    } else if problems.is_empty() {
        vec![Line::from(Span::styled(
            " Nenhum problema: tudo o que está configurado respondeu.",
            Style::default().fg(Color::Green),
        ))]
    } else {
        problems
            .iter()
            .map(|h| {
                let (mark, color) = health_mark(h.state);
                Line::from(vec![
                    Span::styled(format!(" {mark} {}: ", h.name), bold().fg(color)),
                    Span::raw(h.detail.clone()),
                ])
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(attention).wrap(Wrap { trim: false }).block(titled("Precisa de atenção")),
        rows[1],
    );

    let title = "Saúde dos serviços — conferida sozinha a cada 15 s · 'r' confere agora";
    if m.health.is_empty() {
        frame.render_widget(Paragraph::new(" conferindo os serviços…").style(dim()).block(titled(title)), rows[0]);
    } else {
        let header = Row::new(vec!["", "Serviço", "Tipo", "Latência", "Histórico", "No ar", "Detalhe"]).style(bold());
        let table_rows: Vec<Row> = m
            .health
            .iter()
            .map(|h| {
                let (mark, color) = health_mark(h.state);
                let history: Vec<Option<f64>> = m
                    .health_history
                    .get(&h.id)
                    .map(|q| q.iter().map(|p| p.map(|ms| ms as f64)).collect())
                    .unwrap_or_default();
                let uptime = m.uptime_pct(&h.id).map(|p| format!("{p:.0}%")).unwrap_or_else(|| "—".into());
                let detail_style = if h.state == HealthState::Off { dim() } else { Style::default().fg(Color::Gray) };
                Row::new(vec![
                    Cell::from(mark).style(Style::default().fg(color)),
                    Cell::from(h.name.clone()).style(if h.state == HealthState::Off { dim() } else { bold() }),
                    Cell::from(h.kind).style(dim()),
                    Cell::from(h.latency_ms.map(|ms| format!("{ms} ms")).unwrap_or_else(|| "—".into())),
                    Cell::from(sparkline(&history, 50.0, SPARK_W)).style(Style::default().fg(color)),
                    Cell::from(uptime),
                    Cell::from(h.detail.clone()).style(detail_style),
                ])
            })
            .collect();
        let widths = [
            Constraint::Length(1),
            Constraint::Length(15),
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(SPARK_W as u16),
            Constraint::Length(5),
            Constraint::Min(20),
        ];
        frame.render_widget(Table::new(table_rows, widths).header(header).block(titled(title)), rows[0]);
    }

    let models_title = "Modelos, como o orquestrador os vê";
    match &m.api {
        None => frame.render_widget(
            Paragraph::new(" Suba o orquestrador ('u') para ver quais modelos cada papel está usando.")
                .style(dim())
                .block(titled(models_title)),
            rows[2],
        ),
        Some(api) => {
            let header = Row::new(vec!["Chave", "Modelo", "Servidor", "Estado", "Papéis que usam"]).style(bold());
            let table_rows: Vec<Row> = api
                .models
                .iter()
                .map(|model| {
                    let (label, color) = model_state_label(&model.state);
                    let roles: Vec<&str> =
                        api.roles.iter().filter(|r| r.model == model.key).map(|r| r.role.as_str()).collect();
                    let host =
                        model.base_url.split("://").nth(1).unwrap_or(&model.base_url).split('/').next().unwrap_or("");
                    Row::new(vec![
                        Cell::from(model.key.clone()).style(bold()),
                        Cell::from(if model.model.is_empty() { "—".to_string() } else { model.model.clone() }),
                        Cell::from(if host.is_empty() { "—".to_string() } else { host.to_string() }),
                        Cell::from(label).style(Style::default().fg(color)),
                        Cell::from(if roles.is_empty() { "—".to_string() } else { roles.join(", ") }).style(dim()),
                    ])
                })
                .collect();
            let widths = [
                Constraint::Length(12),
                Constraint::Length(26),
                Constraint::Length(28),
                Constraint::Length(16),
                Constraint::Min(10),
            ];
            frame.render_widget(Table::new(table_rows, widths).header(header).block(titled(models_title)), rows[2]);
        }
    }
}

// ── Área Uso ─────────────────────────────────────────────────────────────────

fn tile(frame: &mut Frame, area: Rect, title: &str, value: String, color: Color, note: String) {
    let lines = vec![
        Line::from(Span::styled(format!(" {value}"), bold().fg(color))),
        Line::from(Span::styled(format!(" {note}"), dim())),
    ];
    frame.render_widget(Paragraph::new(lines).block(titled(title)), area);
}

fn success_color(pct: f64) -> Color {
    if pct >= 80.0 {
        Color::Green
    } else if pct >= 50.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

fn draw_usage(frame: &mut Frame, area: Rect, app: &App) {
    let u = &app.monitor.usage;
    if u.is_empty() {
        let source = match &u.source {
            Some(p) => format!("Lendo de {} (vazio por enquanto).", p.display()),
            None => "Nenhuma fase executada ainda: o registro (volumes/oreia-seca/data/escalation.jsonl) nasce quando \
                     um agente termina (ou falha) a primeira fase de uma tarefa."
                .to_string(),
        };
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(" Ainda não há execuções de agentes para mostrar.", bold())),
            Line::from(""),
            Line::from(" Cada vez que um agente tenta resolver uma fase de uma issue, fica registrado aqui:"),
            Line::from(" quantos tokens gastou, quanto tempo levou, quanto custou e se passou na verificação."),
            Line::from(""),
            Line::from(" Para ver isto se mexer: abra o Forgejo, crie uma issue no seu repositório e coloque a"),
            Line::from(" label agent:back nela."),
            Line::from(""),
            Line::from(Span::styled(format!(" {source}"), dim())),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(titled("Uso")), area);
        return;
    }

    let groups_h = (u.by_model.len().max(u.by_role.len()) as u16 + 3).clamp(4, 8);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(5), Constraint::Length(groups_h), Constraint::Min(5)])
        .split(area);

    // Totais
    let tiles =
        Layout::default().direction(Direction::Horizontal).constraints([Constraint::Ratio(1, 4); 4]).split(rows[0]);
    tile(frame, tiles[0], "Execuções", u.attempts.to_string(), Color::Cyan, format!("em {} tarefa(s)", u.tasks));
    let pct = u.success_pct();
    tile(
        frame,
        tiles[1],
        "Sucesso",
        format!("{pct:.0}%"),
        success_color(pct),
        format!("{} de {} passaram", u.passed, u.attempts),
    );
    tile(
        frame,
        tiles[2],
        "Tokens",
        human_tokens(u.total_tokens()),
        Color::Magenta,
        format!("{} entrada · {} saída", human_tokens(u.prompt_tokens), human_tokens(u.completion_tokens)),
    );
    tile(
        frame,
        tiles[3],
        "Custo",
        human_cost(u.cost_usd),
        Color::Yellow,
        format!("tempo médio {}", human_duration(u.avg_duration_ms)),
    );

    // Recortes de tempo + série diária
    let window_line = |label: &str, w: &super::usage::Window| {
        let ok = if w.attempts == 0 {
            "—".to_string()
        } else {
            format!("{:.0}% ok", w.passed as f64 * 100.0 / w.attempts as f64)
        };
        Line::from(vec![
            Span::styled(format!(" {label:<16}"), Style::default().fg(Color::Cyan)),
            Span::raw(format!(
                "{} execução(ões) · {ok} · {} tokens · {}",
                w.attempts,
                human_tokens(w.tokens),
                human_cost(w.cost_usd)
            )),
        ])
    };
    let daily: Vec<Option<f64>> = u.daily_tokens.iter().map(|(_, t)| Some(*t as f64)).collect();
    let peak = u.daily_tokens.iter().max_by_key(|(_, t)| *t);
    let peak_note = match peak {
        Some((day, tokens)) if *tokens > 0 => format!("   pico: {} em {}", human_tokens(*tokens), day.format("%d/%m")),
        _ => String::new(),
    };
    let period = vec![
        window_line("Últimas 24 h", &u.last_24h),
        window_line("Últimos 7 dias", &u.last_7d),
        Line::from(vec![
            Span::styled(format!(" {:<16}", "Tokens por dia"), Style::default().fg(Color::Cyan)),
            Span::styled(sparkline(&daily, 1.0, daily.len()), Style::default().fg(Color::Magenta)),
            Span::styled(format!("  ← {} dias atrás · hoje →{peak_note}", daily.len().saturating_sub(1)), dim()),
        ]),
    ];
    frame.render_widget(Paragraph::new(period).block(titled("Período")), rows[1]);

    // Por modelo | por papel
    let halves = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(rows[2]);
    let group_rows = |groups: &[super::usage::GroupUsage], with_tokens: bool| -> Vec<Row<'static>> {
        groups
            .iter()
            .map(|g| {
                let pct = g.success_pct();
                let mut cells = vec![
                    Cell::from(g.name.clone()).style(bold()),
                    Cell::from(g.attempts.to_string()),
                    Cell::from(format!("{pct:.0}%")).style(Style::default().fg(success_color(pct))),
                ];
                if with_tokens {
                    cells.push(Cell::from(format!(
                        "{} / {}",
                        human_tokens(g.prompt_tokens),
                        human_tokens(g.completion_tokens)
                    )));
                }
                cells.push(Cell::from(human_duration(g.avg_duration_ms)));
                cells.push(Cell::from(human_cost(g.cost_usd)));
                Row::new(cells)
            })
            .collect()
    };
    frame.render_widget(
        Table::new(
            group_rows(&u.by_model, true),
            [
                Constraint::Min(16),
                Constraint::Length(5),
                Constraint::Length(7),
                Constraint::Length(15),
                Constraint::Length(8),
                Constraint::Length(8),
            ],
        )
        .header(Row::new(vec!["Modelo", "Exec.", "Sucesso", "Tokens ent/saí", "T. médio", "Custo"]).style(bold()))
        .block(titled("Por modelo")),
        halves[0],
    );
    frame.render_widget(
        Table::new(
            group_rows(&u.by_role, false),
            [
                Constraint::Min(10),
                Constraint::Length(5),
                Constraint::Length(7),
                Constraint::Length(8),
                Constraint::Length(8),
            ],
        )
        .header(Row::new(vec!["Papel", "Exec.", "Sucesso", "T. médio", "Custo"]).style(bold()))
        .block(titled("Por papel")),
        halves[1],
    );

    // Últimas execuções
    let recent_rows: Vec<Row> = u
        .recent
        .iter()
        .map(|r| {
            let (mark, color) = if r.passed { ("✓ passou", Color::Green) } else { ("✗ falhou", Color::Red) };
            Row::new(vec![
                Cell::from(r.at.with_timezone(&Local).format("%d/%m %H:%M").to_string()).style(dim()),
                Cell::from(r.role.clone()),
                Cell::from(r.model.clone()),
                Cell::from(format!("{} · {}", r.step, r.executor)).style(dim()),
                Cell::from(mark).style(Style::default().fg(color)),
                Cell::from(human_duration(r.duration_ms)),
                Cell::from(human_tokens(r.tokens)),
                Cell::from(r.reason.clone().unwrap_or_default()).style(dim()),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Length(20),
        Constraint::Length(13),
        Constraint::Length(9),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Min(8),
    ];
    frame.render_widget(
        Table::new(recent_rows, widths)
            .header(
                Row::new(vec!["Quando", "Papel", "Modelo", "Degrau", "Resultado", "Duração", "Tokens", "Motivo"])
                    .style(bold()),
            )
            .block(titled("Últimas execuções")),
        rows[3],
    );
}

// ── Áreas de configuração ────────────────────────────────────────────────────

fn field_line(label: &str, value: &str, focused: bool) -> Line<'static> {
    let marker = if focused { "> " } else { "  " };
    let style = if focused { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
    Line::from(vec![
        Span::raw(marker.to_string()),
        Span::styled(format!("{label}: "), Style::default().fg(Color::Cyan)),
        Span::styled(value.to_string(), style),
    ])
}

fn secret_display(typed: &str, was_set: bool) -> String {
    if !typed.is_empty() {
        "•".repeat(typed.chars().count())
    } else if was_set {
        "(definido — digite para trocar)".to_string()
    } else {
        "(vazio)".to_string()
    }
}

fn px0_lines(app: &App, focus_idx: Option<usize>) -> Vec<Line<'static>> {
    let f = &app.px0;
    vec![
        Line::from("Quem pode acessar o px0 — não há botão de ligar/desligar (ver docs/px0.md)."),
        Line::from(""),
        field_line(
            "Contas autorizadas (PX0_ALLOWED_USERS)",
            &display_or_empty(&f.allowed_users.value),
            focus_idx == Some(0),
        ),
        Line::from(""),
        credential_line("FORGEJO_OWNER_TOKEN", f.forgejo_token_set),
        credential_line("PX0_OAUTH_CLIENT_ID", f.oauth_client_id_set),
        credential_line("PX0_OAUTH_CLIENT_SECRET", f.oauth_client_secret_set),
        Line::from(""),
        Line::from("Credenciais acima vêm de \"./oreia bootstrap\" (scripts/bootstrap.sh) — não editáveis aqui."),
    ]
}

fn forgejo_lines(app: &App, focus_idx: Option<usize>) -> Vec<Line<'static>> {
    let f = &app.forgejo;
    vec![
        field_line("URL interna (ex.: http://forgejo:3000)", &display_or_empty(&f.url.value), focus_idx == Some(0)),
        field_line("URL pública (abre no navegador)", &display_or_empty(&f.public_url.value), focus_idx == Some(1)),
        field_line(
            "Repositórios (dono/repo, separados por vírgula)",
            &display_or_empty(&f.repos.value),
            focus_idx == Some(2),
        ),
        field_line("Branch base", &display_or_empty(&f.base_branch.value), focus_idx == Some(3)),
        field_line(
            "Estilo de merge (merge/rebase/squash)",
            &display_or_empty(&f.merge_style.value),
            focus_idx == Some(4),
        ),
        Line::from(""),
        credential_line("FORGEJO_OWNER_TOKEN", f.owner_token_set),
        credential_line("FORGEJO_WEBHOOK_SECRET", f.webhook_secret_set),
    ]
}

fn credential_line(name: &str, set: bool) -> Line<'static> {
    let (mark, color, note) = if set {
        ("✓", Color::Green, "presente")
    } else {
        ("✗", Color::Red, "faltando — rode \"./oreia bootstrap\"")
    };
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{mark} "), Style::default().fg(color)),
        Span::raw(format!("{name}: {note}")),
    ])
}

fn mcp_lines(app: &App, focus_idx: Option<usize>) -> Vec<Line<'static>> {
    let f = &app.mcp;
    let mut lines = vec![
        Line::from("Servidor MCP (P.O. por IA + desenvolvedor sob demanda — docs/mcp.md)."),
        Line::from(""),
        toggle_line("Habilitado", f.enabled, focus_idx == Some(0)),
        toggle_line("Transporte HTTP (opcional)", f.enabled && f.http, focus_idx == Some(1)),
        field_line("Usuário no Forgejo (agent_user)", &display_or_empty(&f.agent_user.value), focus_idx == Some(2)),
        field_line("Diretório de trabalho (workdir)", &display_or_empty(&f.workdir.value), focus_idx == Some(3)),
        field_line(
            "Rodadas de revisão (max_review_rounds)",
            &display_or_empty(&f.max_review_rounds.value),
            focus_idx == Some(4),
        ),
        field_line("Bind do transporte HTTP", &display_or_empty(&f.bind.value), focus_idx == Some(5)),
    ];
    if f.http && f.enabled {
        let token_note =
            if f.token_was_set { "MCP_TOKEN já existe, mantido" } else { "MCP_TOKEN será gerado ao salvar" };
        lines.push(Line::from(format!("  {token_note}")));
    }
    if !f.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from("  Desligado: o bloco [mcp] é removido do oreia-seca.toml ao salvar."));
    }
    lines
}

fn telegram_lines(app: &App, focus_idx: Option<usize>) -> Vec<Line<'static>> {
    let f = &app.telegram;
    vec![
        toggle_line("Habilitado", f.enabled, focus_idx == Some(0)),
        Line::from(""),
        field_line("Token (@BotFather)", &secret_display(&f.token.value, f.token_was_set), focus_idx == Some(1)),
        field_line("IDs permitidos (TELEGRAM_ALLOWED)", &display_or_empty(&f.allowed.value), focus_idx == Some(2)),
    ]
}

fn llm_lines(app: &App, focus_idx: Option<usize>) -> Vec<Line<'static>> {
    let f = &app.llm;
    let mut lines = vec![
        field_line("URL base (ex.: http://<ip>:11434/v1)", &display_or_empty(&f.url.value), focus_idx == Some(0)),
        field_line(
            "Modelo código (planner/dev-*/executor)",
            &display_or_empty(&f.coder_model.value),
            focus_idx == Some(1),
        ),
        field_line(
            "Modelo conversa direta (vazio = igual ao de código)",
            &display_or_empty(&f.chat_model.value),
            focus_idx == Some(2),
        ),
        field_line(
            "Chave da API (se o servidor exigir)",
            &secret_display(&f.api_key.value, f.api_key_was_set),
            focus_idx == Some(3),
        ),
        Line::from(""),
        field_line("Contexto — modelo de código", &display_or_empty(&f.coder_context.value), focus_idx == Some(4)),
        field_line(
            "Temperatura — modelo de código",
            &display_or_empty(&f.coder_temperature.value),
            focus_idx == Some(5),
        ),
        field_line("Contexto — modelo de chat", &display_or_empty(&f.chat_context.value), focus_idx == Some(6)),
        field_line("Temperatura — modelo de chat", &display_or_empty(&f.chat_temperature.value), focus_idx == Some(7)),
    ];
    if let Some(status) = &app.llm_fetch_status {
        lines.push(Line::from(""));
        lines.push(Line::from(status.clone()));
    }
    if !app.llm_models.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(format!("Modelos disponíveis: {}", app.llm_models.join(", "))));
    }
    lines
}

fn executor_lines(app: &App, focus_idx: Option<usize>) -> Vec<Line<'static>> {
    let f = &app.executor;
    vec![
        field_line("Executor padrão (default)", &display_or_empty(&f.default.value), focus_idx == Some(0)),
        field_line("Rodadas por fase (max_steps)", &display_or_empty(&f.max_steps.value), focus_idx == Some(1)),
        field_line(
            "Retentativas no mesmo degrau (phase_retries)",
            &display_or_empty(&f.phase_retries.value),
            focus_idx == Some(2),
        ),
        field_line(
            "Timeout de shell por comando (shell_timeout_secs)",
            &display_or_empty(&f.shell_timeout_secs.value),
            focus_idx == Some(3),
        ),
        field_line(
            "Máximo de fases por tarefa (max_phases)",
            &display_or_empty(&f.max_phases.value),
            focus_idx == Some(4),
        ),
        field_line(
            "Timeout do degrau inteiro em segundos (step_timeout_secs)",
            &display_or_empty(&f.step_timeout_secs.value),
            focus_idx == Some(5),
        ),
        Line::from(""),
        toggle_line("Modo go horse — nunca rodar build/lint/test (go_horse)", f.go_horse, focus_idx == Some(6)),
        Line::from(if f.go_horse {
            "  Ligado: fases nunca rodam build, lint ou suíte de teste, mesmo com test_cmd configurado."
        } else {
            "  Desligado: o planejador só adiciona build/lint/test quando o projeto já tem um e a mudança é arriscada."
        }),
        Line::from(""),
        Line::from(vec![
            Span::styled("Dica: ", Style::default().fg(Color::Yellow)),
            Span::raw("no campo \"Executor padrão\", pressione "),
            Span::styled("Espaço", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" para alternar entre os presets embutidos,"),
        ]),
        Line::from("ou digite o nome de um executor customizado configurado em [executors.*]."),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Presets embutidos disponíveis:",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )]),
        Line::from("  · builtin  - loop agêntico nativo com ações XML (file_write, shell, etc.)"),
        Line::from("  · opencode - CLI OpenCode (ad-hoc provider @ai-sdk/openai-compatible)"),
        Line::from("  · codex    - CLI Codex via npm"),
        Line::from("  · aider    - CLI Aider via pip"),
        Line::from("  · goose    - CLI Goose"),
    ]
}

fn server_lines(app: &App, focus_idx: Option<usize>) -> Vec<Line<'static>> {
    let f = &app.server;
    vec![
        field_line("Host da API", &display_or_empty(&f.host.value), focus_idx == Some(0)),
        field_line("Porta da API", &display_or_empty(&f.port.value), focus_idx == Some(1)),
        field_line("Diretório de dados", &display_or_empty(&f.paths_data.value), focus_idx == Some(2)),
        field_line("Diretório de workspace", &display_or_empty(&f.paths_workspace.value), focus_idx == Some(3)),
        field_line(
            "Fila: tarefas em paralelo (concurrency)",
            &display_or_empty(&f.queue_concurrency.value),
            focus_idx == Some(4),
        ),
        field_line(
            "Fila: timeout por tarefa, em segundos",
            &display_or_empty(&f.queue_timeout_secs.value),
            focus_idx == Some(5),
        ),
        Line::from(""),
        credential_line("REST_API_KEY", f.api_key_set),
    ]
}

fn toggle_line(label: &str, on: bool, focused: bool) -> Line<'static> {
    let marker = if focused { "> " } else { "  " };
    let (mark, color) = if on { ("[x]", Color::Green) } else { ("[ ]", Color::DarkGray) };
    let style = if focused { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
    Line::from(vec![
        Span::raw(marker.to_string()),
        Span::styled(format!("{mark} "), Style::default().fg(color)),
        Span::styled(label.to_string(), style),
    ])
}

fn display_or_empty(value: &str) -> String {
    if value.is_empty() {
        "(vazio)".to_string()
    } else {
        value.to_string()
    }
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(flash) = &app.flash {
        frame.render_widget(
            Paragraph::new(format!(" {flash}"))
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    }
    let text = match app.focus {
        Focus::AreaList if app.selected_area_kind().is_monitor() => {
            " ↑/↓: área · r: atualizar · u: subir · d: parar · i: reiniciar · b: bootstrap · l: logs · a: assistente · q: sair"
        }
        Focus::AreaList => " ↑/↓: área · Enter/Tab: editar · Ctrl+S/F2: resumo e salvar · Esc/q: sair sem salvar",
        Focus::Field(_) => {
            " digite para editar · Espaço: liga/desliga ou troca o preset · Tab/Enter: próximo campo · Esc: voltar · Ctrl+S/F2: salvar"
        }
    };
    frame.render_widget(Paragraph::new(text).block(Block::default().borders(Borders::ALL)), area);
}

fn draw_summary(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let payload = app.build_save_payload();
    let Ok((answers, _)) = &payload else {
        let err = payload.unwrap_err();
        let lines = vec![
            Line::from("Não dá para salvar:"),
            Line::from(""),
            Line::from(err),
            Line::from(""),
            Line::from("Esc para corrigir."),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::Red))
                .block(Block::default().borders(Borders::ALL).title("Erro de validação")),
            area,
        );
        return;
    };

    let mut lines = vec![Line::from("Resumo — o que será gravado ao salvar:"), Line::from("")];

    lines.push(Line::from(format!(
        "Telegram: {} · token: {} · permitidos: {}",
        if app.telegram.enabled { "habilitado" } else { "desligado" },
        if !answers.telegram.token.is_empty() {
            "(alterado)"
        } else if app.telegram.token_was_set {
            "(mantido)"
        } else {
            "(vazio)"
        },
        display_or_empty(&answers.telegram.allowed),
    )));
    lines.push(Line::from(format!(
        "MCP: {} · HTTP: {} · agent_user: {} · workdir: {}",
        if app.mcp.enabled { "habilitado" } else { "desligado" },
        if app.mcp.enabled && app.mcp.http { "ligado" } else { "desligado" },
        display_or_empty(&app.mcp.agent_user.value),
        display_or_empty(&app.mcp.workdir.value),
    )));
    lines.push(Line::from(format!(
        "LLM: url {} · modelo código {} · modelo chat {} · chave: {}",
        display_or_empty(&answers.llm.url),
        display_or_empty(&answers.llm.model),
        display_or_empty(&answers.llm.chat_model),
        if !answers.llm.api_key.is_empty() {
            "(alterado)"
        } else if app.llm.api_key_was_set {
            "(mantido)"
        } else {
            "(vazio)"
        },
    )));
    lines.push(Line::from(format!("px0: contas permitidas {}", display_or_empty(&answers.px0.allowed_users))));
    lines.push(Line::from(format!(
        "Forgejo: url {} · repos {}",
        display_or_empty(&app.forgejo.url.value),
        display_or_empty(&app.forgejo.repos.value),
    )));
    lines.push(Line::from(format!(
        "Executor: padrão {} · fases max {} · rodadas {}x · timeout degrau {}s",
        display_or_empty(&app.executor.default.value),
        display_or_empty(&app.executor.max_phases.value),
        display_or_empty(&app.executor.max_steps.value),
        display_or_empty(&app.executor.step_timeout_secs.value),
    )));
    lines.push(Line::from(format!(
        "Servidor: {}:{} · fila {}x, timeout {}s",
        display_or_empty(&app.server.host.value),
        display_or_empty(&app.server.port.value),
        display_or_empty(&app.server.queue_concurrency.value),
        display_or_empty(&app.server.queue_timeout_secs.value),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from("Depois de salvar, o launcher pergunta se aplica agora (reinicia o orquestrador). Sem o"));
    lines.push(Line::from("launcher, rode \"./oreia restart\" no terminal para aplicar."));
    lines.push(Line::from(""));
    lines.push(Line::from("Enter/s: salvar · Esc: voltar a editar"));

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::default().borders(Borders::ALL).title("Resumo")),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::log::EscalationLogEntry;
    use crate::setup::current_state::CurrentState;
    use crate::setup::tui::monitor::{ApiStatus, EnvMap, ModelRow, RoleRow, Sample};
    use crate::setup::tui::status::{PrerequisiteCheck, StatusSnapshot};
    use crate::setup::tui::usage::summarize;
    use crate::setup::tui::wizard::{plan_ports, Step, Wizard, PORT_SPECS};
    use chrono::{Duration, Utc};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Desenha num terminal de mentira e devolve a tela como texto. Com
    /// `OREIA_DUMP_UI=1` (e `cargo test -- --nocapture`) imprime cada tela —
    /// o jeito de "ver" a TUI sem um terminal interativo.
    fn render(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..h {
            for x in 0..w {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        if std::env::var("OREIA_DUMP_UI").is_ok() {
            println!("{out}");
        }
        out
    }

    fn health(
        id: &str,
        name: &str,
        kind: &'static str,
        state: HealthState,
        ms: Option<u64>,
        detail: &str,
    ) -> ServiceHealth {
        ServiceHealth { id: id.into(), name: name.into(), kind, state, latency_ms: ms, detail: detail.into() }
    }

    fn container(service: &str, state: &str, status: &str, cpu: f64, mem: &str) -> ContainerRow {
        ContainerRow {
            service: service.into(),
            state: state.into(),
            status_text: status.into(),
            cpu_pct: (state == "running").then_some(cpu),
            mem_usage: (state == "running").then(|| mem.to_string()),
            mem_pct: (state == "running").then_some(cpu / 2.0),
        }
    }

    fn entry(task: &str, role: &str, model: &str, passed: bool, hours_ago: i64) -> EscalationLogEntry {
        EscalationLogEntry {
            task_id: task.into(),
            role: role.into(),
            phase: 1,
            step: 1,
            executor: "builtin".into(),
            model: model.into(),
            passed,
            reason: (!passed).then(|| "cargo test falhou: 2 testes".to_string()),
            duration_ms: 95_000,
            prompt_tokens: 18_400,
            completion_tokens: 3_100,
            cost_usd: if model.contains("sonnet") { 0.21 } else { 0.0 },
            at: Utc::now() - Duration::hours(hours_ago),
            handoff_from: None,
        }
    }

    fn live_sample(cpu: f64, forgejo_ms: u64) -> Sample {
        let ok = |label: &str, message: &str| PrerequisiteCheck {
            label: label.into(),
            level: CheckLevel::Ok,
            message: message.into(),
        };
        Sample {
            snapshot: StatusSnapshot {
                forgejo_url: "http://localhost:3000".into(),
                px0_url: "http://localhost:7777".into(),
                api_url: "http://localhost:3120".into(),
                containers: vec![
                    container("oreia-seca", "running", "Up 2 hours (healthy)", cpu, "48.2MiB / 15.6GiB"),
                    container("forgejo", "running", "Up 2 hours (healthy)", cpu * 2.0, "212MiB / 15.6GiB"),
                    container("postgres", "running", "Up 2 hours (healthy)", 0.3, "61MiB / 15.6GiB"),
                    container("px0", "exited", "Exited (1) 5 minutes ago", 0.0, ""),
                ],
                error: None,
                checks: vec![
                    ok("Arquivos de configuração", ".env e oreia-seca.toml presentes"),
                    ok("Bootstrap do Forgejo", "contas e tokens de acesso já gerados"),
                    PrerequisiteCheck {
                        label: "Acesso ao px0".into(),
                        level: CheckLevel::Warn,
                        message: "lista vazia bloqueia todo mundo na revisão de PRs (área px0)".into(),
                    },
                ],
            },
            api: Some(ApiStatus {
                running: 1,
                queued: 2,
                completed: 14,
                failed: 1,
                concurrency: 1,
                pending_approvals: 1,
                active_work: vec!["acme/loja#12".into()],
                agents: vec!["dev-back".into()],
                roles: vec![
                    RoleRow { role: "planner".into(), model: "coder".into(), ready: true },
                    RoleRow { role: "dev-back".into(), model: "coder".into(), ready: true },
                    RoleRow { role: "direct".into(), model: "chat".into(), ready: false },
                ],
                models: vec![
                    ModelRow {
                        key: "chat".into(),
                        model: "llama3.1:8b".into(),
                        base_url: "http://192.168.1.50:11434/v1".into(),
                        state: "unreachable".into(),
                    },
                    ModelRow {
                        key: "coder".into(),
                        model: "qwen2.5-coder:14b".into(),
                        base_url: "http://192.168.1.50:11434/v1".into(),
                        state: "ok".into(),
                    },
                ],
            }),
            health: Some(vec![
                health(
                    "docker",
                    "Docker",
                    "infra",
                    HealthState::Up,
                    Some(38),
                    "3 de 4 container(s) do projeto rodando",
                ),
                health("api", "Orquestrador", "API", HealthState::Up, Some(4), "localhost:3120"),
                health("forgejo", "Forgejo", "git", HealthState::Up, Some(forgejo_ms), "localhost:3000"),
                health(
                    "px0",
                    "px0",
                    "revisão",
                    HealthState::Down,
                    None,
                    "sem resposta (conexão recusada ou host inacessível)",
                ),
                health(
                    "llm-coder",
                    "LLM · código",
                    "LLM",
                    HealthState::Degraded,
                    Some(140),
                    "192.168.1.50:11434: modelo \"qwen3\" não está na lista do servidor",
                ),
                health("telegram", "Telegram", "canal", HealthState::Off, None, "sem token — opcional (área Telegram)"),
            ]),
            usage: summarize(
                &[
                    entry("t1", "dev-back", "qwen2.5-coder:14b", false, 2),
                    entry("t1", "dev-back", "qwen2.5-coder:14b", true, 1),
                    entry("t2", "dev-front", "claude-sonnet-5-5", true, 30),
                    entry("t3", "reviewer", "llama3.1:8b", true, 100),
                ],
                Utc::now(),
            ),
            at: Local::now(),
            took_ms: 2100,
        }
    }

    fn live_app() -> App {
        let mut app = App::new(&CurrentState::default());
        app.actions_enabled = true;
        for (cpu, ms) in [(1.0, 9), (4.0, 14), (2.5, 11), (7.5, 30), (3.0, 12)] {
            app.absorb(live_sample(cpu, ms));
        }
        app
    }

    fn select(app: &mut App, kind: AreaKind) {
        app.selected_area = AreaKind::ALL.iter().position(|a| *a == kind).unwrap();
    }

    #[test]
    fn sparkline_scales_right_aligns_and_marks_gaps() {
        assert_eq!(sparkline(&[Some(0.0), Some(50.0), Some(100.0)], 1.0, 3), "▁▅█");
        assert_eq!(sparkline(&[Some(100.0)], 1.0, 4), "   █", "o ponto mais novo fica na última coluna");
        assert_eq!(sparkline(&[Some(10.0), None, Some(10.0)], 1.0, 3), "█×█");
        assert_eq!(sparkline(&[Some(1.0), Some(2.0), Some(3.0)], 1.0, 2).chars().count(), 2, "só os mais recentes");
        assert_eq!(sparkline(&[Some(3.0), Some(3.0)], 50.0, 2), "▁▁", "abaixo do piso da escala fica rente ao chão");
        assert_eq!(sparkline(&[], 1.0, 3), "   ");
        assert_eq!(sparkline(&[Some(0.0)], 0.0, 1), "▁", "série toda zerada não divide por zero");
    }

    #[test]
    fn status_area_shows_access_queue_containers_and_checklist() {
        let app = live_app();
        let screen = render(&app, 130, 36);
        for expected in [
            "OREIA-SECA",
            "3/5 serviços no ar",
            "PAINEL",
            "CONFIGURAR",
            "http://localhost:3000",
            "Em andamento",
            "Parado: px0.",
            "no ar · 12 ms",
            "fora do ar",
            "1 rodando · 2 na espera · 14 concluída(s) · 1 falha(s)",
            "1 aguardando /approve",
            "acme/loja#12",
            "Up 2 hours (healthy)",
            "Exited (1) 5 minutes ago",
            "48.2MiB / 15.6GiB",
            "Acesso ao px0",
            "u: subir",
        ] {
            assert!(screen.contains(expected), "faltou \"{expected}\" em:\n{screen}");
        }
    }

    #[test]
    fn status_area_before_the_first_sample_and_without_containers() {
        let mut app = App::new(&CurrentState::default());
        let waiting = render(&app, 120, 30);
        assert!(waiting.contains("lendo o Docker"), "{waiting}");
        assert!(waiting.contains("verificando serviços"), "{waiting}");

        let mut sample = live_sample(1.0, 10);
        sample.snapshot.containers.clear();
        sample.api = None;
        app.absorb(sample);
        let empty = render(&app, 120, 30);
        assert!(empty.contains("Nenhum container do OREIA-SECA foi criado ainda"), "{empty}");
        assert!(empty.contains("o orquestrador não respondeu"), "{empty}");
    }

    #[test]
    fn status_area_shows_the_docker_error_instead_of_the_table() {
        let mut app = App::new(&CurrentState::default());
        let mut sample = live_sample(1.0, 10);
        sample.snapshot.containers.clear();
        sample.snapshot.error = Some("O Docker está instalado mas não está respondendo".into());
        app.absorb(sample);
        assert!(render(&app, 120, 30).contains("não está respondendo"));
    }

    #[test]
    fn health_area_lists_every_service_with_latency_history_and_models() {
        let mut app = live_app();
        select(&mut app, AreaKind::Health);
        let screen = render(&app, 130, 36);
        for expected in [
            "Saúde dos serviços",
            "Orquestrador",
            "Forgejo",
            "12 ms",
            "100%",
            "LLM · código",
            "não está na lista do servidor",
            "sem token — opcional",
            "Precisa de atenção",
            "▲ LLM · código: 192.168.1.50:11434: modelo \"qwen3\" não está na lista do servidor",
            "Modelos, como o orquestrador os vê",
            "qwen2.5-coder:14b",
            "192.168.1.50:11434",
            "inacessível",
            "planner, dev-back",
        ] {
            assert!(screen.contains(expected), "faltou \"{expected}\" em:\n{screen}");
        }
        assert!(screen.contains('×'), "px0 fora do ar deve marcar o histórico com ×:\n{screen}");
    }

    #[test]
    fn usage_area_shows_totals_windows_groups_and_recent_runs() {
        let mut app = live_app();
        select(&mut app, AreaKind::Usage);
        let screen = render(&app, 130, 36);
        for expected in [
            "Execuções",
            "em 3 tarefa(s)",
            "75%",
            "3 de 4 passaram",
            "86,0k",
            "$0,21",
            "tempo médio 1m35s",
            "Últimas 24 h",
            "Tokens por dia",
            "Por modelo",
            "qwen2.5-coder:14b",
            "Por papel",
            "dev-front",
            "✗ falhou",
            "cargo test falhou",
        ] {
            assert!(screen.contains(expected), "faltou \"{expected}\" em:\n{screen}");
        }
    }

    #[test]
    fn splash_screen_and_sidebar_mascot() {
        let mut app = live_app();
        app.screen = Screen::Splash;
        app.anim_ms = 2000;
        let splash = render(&app, 110, 32);
        assert!(splash.contains("agentes de IA"), "{splash}");
        assert!(!splash.contains("CONFIGURAR"), "a abertura cobre o painel inteiro");

        // No painel, com altura sobrando, o mascote aparece no pé da lista de áreas...
        app.screen = Screen::Editing;
        let tall = render(&app, 130, 40);
        assert!(tall.contains("em obra") || tall.contains("esperando serviço"), "{tall}");
        // ...e some (sem quebrar nada) quando o terminal é baixo.
        let short = render(&app, 130, 24);
        assert!(!short.contains("em obra") && !short.contains("esperando serviço"));
        assert!(short.contains("Servidor"));

        // Cada área do menu tem a sua atividade (e a sua legenda).
        let mut captions = std::collections::BTreeSet::new();
        for kind in AreaKind::ALL {
            select(&mut app, kind);
            let screen = render(&app, 130, 40);
            let caption = [
                "em obra",
                "medindo o pulso",
                "fazendo as contas",
                "revisando o PR",
                "carregando tijolo",
                "dando um alô",
                "mandando recado",
                "tendo uma ideia",
                "apertando parafuso",
                "olho no servidor",
            ]
            .into_iter()
            .find(|c| screen.contains(c))
            .unwrap_or_else(|| panic!("{kind:?} sem legenda do mascote:\n{screen}"));
            captions.insert(caption);
        }
        assert_eq!(captions.len(), AreaKind::ALL.len());
    }

    #[test]
    fn usage_area_explains_itself_when_there_is_nothing_yet() {
        let mut app = App::new(&CurrentState::default());
        select(&mut app, AreaKind::Usage);
        let screen = render(&app, 120, 30);
        assert!(screen.contains("Ainda não há execuções"), "{screen}");
        assert!(screen.contains("agent:back"), "{screen}");
    }

    #[test]
    fn flash_message_replaces_the_help_line() {
        let mut app = live_app();
        app.flash = Some("Há alterações não salvas".into());
        let screen = render(&app, 120, 30);
        assert!(screen.contains("Há alterações não salvas"));
        assert!(!screen.contains("u: subir"));
    }

    #[test]
    fn configuration_areas_still_render() {
        let mut app = live_app();
        for kind in [
            AreaKind::Px0,
            AreaKind::Forgejo,
            AreaKind::Mcp,
            AreaKind::Telegram,
            AreaKind::Llm,
            AreaKind::Executor,
            AreaKind::Server,
        ] {
            select(&mut app, kind);
            assert!(render(&app, 120, 34).contains(kind.title()));
        }
        app.screen = Screen::Summary;
        assert!(render(&app, 120, 34).contains("Resumo"));
    }

    #[test]
    fn every_screen_survives_a_tiny_terminal() {
        let mut app = live_app();
        for kind in AreaKind::ALL {
            select(&mut app, kind);
            render(&app, 40, 10);
            render(&app, 80, 24);
        }
        let mut w = Wizard::new(&CurrentState::default(), &EnvMap::default());
        w.on_models(Ok((0..60).map(|i| format!("modelo-{i}")).collect()));
        app.wizard = Some(w);
        app.screen = Screen::Wizard;
        render(&app, 40, 10);
        render(&app, 20, 5);
    }

    #[test]
    fn wizard_screens_render_each_step() {
        let mut app = App::new(&CurrentState::default());
        app.wizard = Some(Wizard::new(&CurrentState::default(), &EnvMap::default()));
        app.screen = Screen::Wizard;
        fn w(app: &mut App) -> &mut Wizard {
            app.wizard.as_mut().unwrap()
        }

        let welcome = render(&app, 110, 36);
        assert!(welcome.contains("primeira instalação"), "{welcome}");
        assert!(welcome.contains("Enter: começar"), "{welcome}");

        w(&mut app).step = Step::Provider;
        let provider = render(&app, 110, 36);
        assert!(provider.contains("passo 1 de 5"), "{provider}");
        assert!(provider.contains("LM Studio neste computador"), "{provider}");
        assert!(provider.contains("Configurar depois"), "{provider}");

        w(&mut app).step = Step::Connection;
        w(&mut app).url.value = "http://host.docker.internal:11434/v1".into();
        w(&mut app).error = Some("erro de exemplo".into());
        let connection = render(&app, 110, 36);
        assert!(connection.contains("http://host.docker.internal:11434/v1"), "{connection}");
        assert!(connection.contains("✗ erro de exemplo"), "{connection}");

        w(&mut app).on_models(Ok(vec!["llama3.1:8b".into(), "qwen2.5-coder:14b".into()]));
        let model = render(&app, 110, 36);
        assert!(model.contains("passo 2 de 5"), "{model}");
        assert!(model.contains("qwen2.5-coder:14b"), "{model}");
        assert!(model.contains("2 de 2 modelo(s)"), "{model}");

        w(&mut app).step = Step::Project;
        let project = render(&app, 110, 36);
        assert!(project.contains("minha-org/meu-sistema"), "{project}");

        w(&mut app).step = Step::Telegram;
        assert!(render(&app, 110, 36).contains("@BotFather"));

        w(&mut app).on_ports(plan_ports(PORT_SPECS, &|p| p == 3000));
        let review = render(&app, 110, 36);
        assert!(review.contains("passo 5 de 5"), "{review}");
        assert!(review.contains("http://localhost:3001"), "{review}");
        assert!(review.contains("a 3000 já estava em uso"), "{review}");
        assert!(review.contains("Enter: gravar e instalar"), "{review}");
    }
}
