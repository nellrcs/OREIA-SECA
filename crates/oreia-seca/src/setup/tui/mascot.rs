//! O mascote do projeto — o "oreia seca", servente de obra — em pixel art, e
//! a tela de abertura onde ele levanta uma parede debaixo do nome do projeto.
//!
//! Cada célula do terminal mostra DOIS pixels (um em cima do outro) usando os
//! meios-blocos `▀`/`▄`: como a célula é ~2x mais alta que larga, o pixel sai
//! quadrado. Pixel transparente não pinta nada (nem cor de frente, nem de
//! fundo), então o sprite funciona sobre qualquer tema de terminal.
//!
//! As cores são da paleta de 256 (`Color::Indexed`), não RGB: o Terminal.app
//! do Mac e alguns consoles não entendem "truecolor", e a paleta de 256
//! funciona em todos.
//!
//! Tudo aqui é função do tempo (`ms` desde que a tela abriu) — nada guarda
//! estado, o que deixa as animações testáveis quadro a quadro.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Largura do sprite, em células (sobra à direita para o martelo).
pub const WIDTH: u16 = 12;
/// Altura do sprite, em linhas do terminal (14 pixels).
pub const HEIGHT: u16 = 7;

const HELMET: Color = Color::Indexed(208);
const BRIM: Color = Color::Indexed(202);
const SKIN: Color = Color::Indexed(130);
const EYE: Color = Color::Indexed(233);
const VEST: Color = Color::Indexed(214);
const STRIPE: Color = Color::Indexed(252);
const CLOTHES: Color = Color::Indexed(25);
const BOOT: Color = Color::Indexed(238);
const TOOL: Color = Color::Indexed(247);
const WOOD: Color = Color::Indexed(137);
const BRICK_A: Color = Color::Indexed(167);
const BRICK_B: Color = Color::Indexed(131);
const HEART: Color = Color::Indexed(196);
const GOLD: Color = Color::Indexed(220);
const GLASS: Color = Color::Indexed(81);
const PAPER: Color = Color::Indexed(255);
const RACK: Color = Color::Indexed(240);
const LED: Color = Color::Indexed(46);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pose {
    Idle,
    Blink,
    WalkA,
    WalkB,
    HammerUp,
    HammerDown,
    // Uma dupla de quadros por atividade ([`Act`]).
    HeartBig,
    HeartSmall,
    CoinLow,
    CoinHigh,
    LensLow,
    LensHigh,
    CarryA,
    CarryB,
    WaveLow,
    WaveHigh,
    PlaneNear,
    PlaneFar,
    IdeaOn,
    IdeaOff,
    WrenchA,
    WrenchB,
    RackA,
    RackB,
}

/// O que o mascote está fazendo — uma atividade por área do menu do painel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Saúde: um coração batendo.
    Pulse,
    /// Uso: jogando uma moeda para cima.
    Coin,
    /// px0 (revisão de PR): examinando com a lupa.
    Review,
    /// Forgejo: carregando tijolo.
    Carry,
    /// MCP: acenando para quem chega de fora.
    Wave,
    /// Telegram: soltando um aviãozinho de papel.
    Message,
    /// LLM: a lâmpada da ideia acendendo.
    Idea,
    /// Executor: girando a chave.
    Wrench,
    /// Servidor: ao lado do rack, luzes piscando.
    Server,
}

/// O quadro da atividade naquele instante.
pub fn act_pose(act: Act, ms: u64) -> Pose {
    let (a, b, period) = match act {
        Act::Pulse => (Pose::HeartBig, Pose::HeartSmall, 450),
        Act::Coin => (Pose::CoinLow, Pose::CoinHigh, 350),
        Act::Review => (Pose::LensLow, Pose::LensHigh, 700),
        Act::Carry => (Pose::CarryA, Pose::CarryB, 300),
        Act::Wave => (Pose::WaveLow, Pose::WaveHigh, 350),
        Act::Message => (Pose::PlaneNear, Pose::PlaneFar, 500),
        Act::Idea => (Pose::IdeaOff, Pose::IdeaOn, 800),
        Act::Wrench => (Pose::WrenchA, Pose::WrenchB, 400),
        Act::Server => (Pose::RackA, Pose::RackB, 500),
    };
    if (ms / period).is_multiple_of(2) {
        a
    } else {
        b
    }
}

/// 14 linhas de pixels x 12 colunas. Legenda em [`color_of`].
const BASE: [&str; 14] = [
    "...HHHH.....",
    "..HHHHHH....",
    ".hhhhhhhh...",
    "..SSSSSS....",
    ".SSESSESS...",
    "..SSSSSS....",
    "...SSSS.....",
    ".BVFVVFVB...",
    "SBVFVVFVBS..",
    "S.VVVVVV.S..",
    "..BBBBBB....",
    "..BB..BB....",
    "..BB..BB....",
    ".KKK..KKK...",
];

fn color_of(c: u8) -> Option<Color> {
    Some(match c {
        b'H' => HELMET,
        b'h' => BRIM,
        b'S' => SKIN,
        b'E' => EYE,
        b'V' => VEST,
        b'F' => STRIPE,
        b'B' => CLOTHES,
        b'K' => BOOT,
        b'T' => TOOL,
        b'W' => WOOD,
        b'r' => BRICK_A,
        b'R' => HEART,
        b'Y' => GOLD,
        b'P' => GLASS,
        b'p' => PAPER,
        b'D' => RACK,
        b'G' => LED,
        _ => return None,
    })
}

/// A pose é a base com algumas linhas trocadas.
fn pixels(pose: Pose) -> [&'static str; 14] {
    let mut g = BASE;
    match pose {
        Pose::Idle => {}
        Pose::Blink => g[4] = ".SSSSSSSS...",
        Pose::WalkA => {
            g[12] = ".KKK..BB....";
            g[13] = "......KKK...";
        }
        Pose::WalkB => {
            g[12] = "..BB..KKK...";
            g[13] = ".KKK........";
        }
        Pose::HammerUp => {
            g[5] = "..SSSSSS..TT";
            g[6] = "...SSSS...TT";
            g[7] = ".BVFVVFVBSW.";
            g[8] = "SBVFVVFVB...";
            g[9] = "S.VVVVVV....";
        }
        Pose::HammerDown => {
            g[8] = "SBVFVVFVBSWT";
            g[9] = "S.VVVVVV...T";
        }
        Pose::HeartBig => {
            g[1] = "..HHHHHH.R.R";
            g[2] = ".hhhhhhhhRRR";
            g[3] = "..SSSSSS..R.";
        }
        Pose::HeartSmall => g[2] = ".hhhhhhhh.R.",
        Pose::CoinLow | Pose::CoinHigh | Pose::PlaneNear | Pose::PlaneFar | Pose::LensLow => {
            // Braço direito erguido, mão na altura do ombro.
            g[7] = ".BVFVVFVBS..";
            g[8] = "SBVFVVFVB...";
            g[9] = "S.VVVVVV....";
            match pose {
                Pose::CoinLow => g[5] = "..SSSSSS..Y.",
                Pose::CoinHigh => g[2] = ".hhhhhhhh.Y.",
                Pose::PlaneNear => g[6] = "...SSSS..pp.",
                Pose::PlaneFar => g[4] = ".SSESSESS.pp",
                _ => {
                    g[4] = ".SSESSESS.PP";
                    g[5] = "..SSSSSS..PP";
                    g[6] = "...SSSS...W.";
                }
            }
        }
        Pose::LensHigh => {
            g[3] = "..SSSSSS..PP";
            g[4] = ".SSESSESS.PP";
            g[5] = "..SSSSSS..W.";
            g[6] = "...SSSS..S..";
            g[8] = "SBVFVVFVB...";
            g[9] = "S.VVVVVV....";
        }
        Pose::CarryA | Pose::CarryB => {
            g[8] = "SBVFVVFVBSrr";
            g[9] = "S.VVVVVV..rr";
            if pose == Pose::CarryA {
                g[12] = ".KKK..BB....";
                g[13] = "......KKK...";
            } else {
                g[12] = "..BB..KKK...";
                g[13] = ".KKK........";
            }
        }
        Pose::WaveLow | Pose::WaveHigh => {
            g[6] = "...SSSS..S..";
            g[8] = "SBVFVVFVB...";
            g[9] = "S.VVVVVV....";
            if pose == Pose::WaveHigh {
                g[5] = "..SSSSSS..S.";
            }
        }
        Pose::IdeaOn | Pose::IdeaOff => {
            if pose == Pose::IdeaOn {
                g[0] = "...HHHH...YY";
                g[1] = "..HHHHHH..YY";
            } else {
                g[0] = "...HHHH...DD";
                g[1] = "..HHHHHH..DD";
            }
            g[2] = ".hhhhhhhh.T.";
        }
        Pose::WrenchA => g[8] = "SBVFVVFVBSTT",
        Pose::WrenchB => {
            g[8] = "SBVFVVFVBST.";
            g[9] = "S.VVVVVV.ST.";
        }
        Pose::RackA | Pose::RackB => {
            let on = pose == Pose::RackA;
            g[8] = "SBVFVVFVBSDD";
            g[9] = if on { "S.VVVVVV.SGD" } else { "S.VVVVVV.SDG" };
            g[10] = "..BBBBBB..DD";
            g[11] = if on { "..BB..BB..DG" } else { "..BB..BB..GD" };
            g[12] = "..BB..BB..DD";
            g[13] = ".KKK..KKK.DD";
        }
    }
    g
}

/// O sprite pronto para um `Paragraph`: [`HEIGHT`] linhas de [`WIDTH`] células.
pub fn lines(pose: Pose) -> Vec<Line<'static>> {
    let g = pixels(pose);
    g.chunks(2)
        .map(|pair| {
            let (top, bottom) = (pair[0].as_bytes(), pair[1].as_bytes());
            let spans: Vec<Span<'static>> = top
                .iter()
                .zip(bottom)
                .map(|(t, b)| match (color_of(*t), color_of(*b)) {
                    (None, None) => Span::raw(" "),
                    (Some(t), None) => Span::styled("▀", Style::default().fg(t)),
                    (None, Some(b)) => Span::styled("▄", Style::default().fg(b)),
                    (Some(t), Some(b)) => Span::styled("▀", Style::default().fg(t).bg(b)),
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

/// Parado, piscando de vez em quando.
pub fn idle_pose(ms: u64) -> Pose {
    if ms % 3200 >= 3000 {
        Pose::Blink
    } else {
        Pose::Idle
    }
}

/// Martelando.
pub fn working_pose(ms: u64) -> Pose {
    if (ms / 280).is_multiple_of(2) {
        Pose::HammerUp
    } else {
        Pose::HammerDown
    }
}

fn walking_pose(ms: u64) -> Pose {
    if (ms / 160).is_multiple_of(2) {
        Pose::WalkA
    } else {
        Pose::WalkB
    }
}

/// Desenha o mascote com o canto superior esquerdo em (`x`, `y`), se couber em `within`.
pub fn render(frame: &mut Frame, within: Rect, x: u16, y: u16, pose: Pose) {
    let rect = Rect { x, y, width: WIDTH, height: HEIGHT }.intersection(within);
    if rect.width == WIDTH && rect.height == HEIGHT {
        frame.render_widget(Paragraph::new(lines(pose)), rect);
    }
}

// ── Tela de abertura ─────────────────────────────────────────────────────────

const TITLE: &str = "O.R.E.I.A-S.E.C.A";
const TAGLINE: &str = "agentes de IA que pegam a issue e entregam o pull request";

/// Ele entra andando...
const WALK_MS: u64 = 1100;
/// ...assenta um tijolo a cada tanto...
const BRICK_MS: u64 = 70;
/// ...e a tela ainda fica um instante com a obra pronta antes de abrir o painel.
const HOLD_MS: u64 = 1400;

/// Onde o mascote para (coluna, dentro da cena) e onde a parede começa.
const STOP_X: u16 = 14;
const WALL_X: u16 = STOP_X + WIDTH;
const WALL_W: u16 = 24;
const WALL_ROWS: u16 = 4;
const SCENE_W: u16 = WALL_X + WALL_W;

/// Letras de 5 linhas; só as que o nome usa.
fn glyph(c: char) -> [&'static str; 5] {
    match c {
        'O' => [" ███ ", "█   █", "█   █", "█   █", " ███ "],
        'R' => ["████ ", "█   █", "████ ", "█  █ ", "█   █"],
        'E' => ["█████", "█    ", "████ ", "█    ", "█████"],
        'I' => ["███", " █ ", " █ ", " █ ", "███"],
        'A' => [" ███ ", "█   █", "█████", "█   █", "█   █"],
        'S' => [" ████", "█    ", " ███ ", "    █", "████ "],
        'C' => [" ████", "█    ", "█    ", "█    ", " ████"],
        '.' => [" ", " ", " ", " ", "▄"],
        '-' => ["   ", "   ", "▀▀▀", "   ", "   "],
        _ => [" ", " ", " ", " ", " "],
    }
}

/// O nome do projeto em letras grandes (5 linhas).
fn big_title() -> Vec<String> {
    (0..5).map(|row| TITLE.chars().map(|c| glyph(c)[row]).collect::<Vec<_>>().join(" ")).collect()
}

/// Os tijolos da parede na ordem em que são assentados: de baixo para cima,
/// da esquerda para a direita, fiadas alternadas desencontradas.
/// Cada item é (fiada a partir do chão, coluna, largura).
fn bricks() -> Vec<(u16, u16, u16)> {
    let mut v = Vec::new();
    for row in 0..WALL_ROWS {
        let mut x: i32 = if row % 2 == 0 { 0 } else { -2 };
        while x < WALL_W as i32 {
            let start = x.max(0);
            let end = (x + 3).min(WALL_W as i32);
            if end > start {
                v.push((row, start as u16, (end - start) as u16));
            }
            x += 4;
        }
    }
    v
}

/// Quanto dura a abertura inteira; depois disso o painel abre sozinho.
pub fn splash_duration_ms() -> u64 {
    WALK_MS + bricks().len() as u64 * BRICK_MS + HOLD_MS
}

fn centered(area: Rect, width: u16, y: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    Rect { x: area.x + (area.width - width) / 2, y, width, height }.intersection(area)
}

pub fn draw_splash(frame: &mut Frame, ms: u64) {
    let area = frame.area();
    let title = big_title();
    let title_w = title[0].chars().count() as u16;
    let big = area.width >= title_w + 2 && area.height >= 19;
    let scene = area.width >= SCENE_W + 2 && area.height >= if big { 19 } else { 13 };

    let title_h: u16 = if big { 5 } else { 1 };
    let total = title_h + 2 + if scene { 2 + HEIGHT + 1 } else { 0 } + 2;
    let mut y = area.y + area.height.saturating_sub(total) / 2;

    if big {
        let shades = [214, 214, 208, 208, 202];
        let lines: Vec<Line> = title
            .into_iter()
            .zip(shades)
            .map(|(text, shade)| Line::from(Span::styled(text, Style::default().fg(Color::Indexed(shade)))))
            .collect();
        frame.render_widget(Paragraph::new(lines), centered(area, title_w, y, 5));
    } else {
        let line = Line::from(Span::styled(TITLE, Style::default().fg(HELMET).add_modifier(Modifier::BOLD)));
        frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), centered(area, area.width, y, 1));
    }
    y += title_h + 1;
    let tagline = Line::from(Span::styled(TAGLINE, Style::default().fg(Color::Gray)));
    frame.render_widget(Paragraph::new(tagline).alignment(Alignment::Center), centered(area, area.width, y, 1));
    y += 1;

    let all = bricks();
    let laid = (ms.saturating_sub(WALK_MS) / BRICK_MS).min(all.len() as u64) as usize;
    let done = laid == all.len();

    if scene {
        y += 2;
        let origin = centered(area, SCENE_W, y, HEIGHT + 1);
        // Mascote: anda até o ponto, martela enquanto a parede sobe, descansa no fim.
        let (x, pose) = if ms < WALK_MS {
            ((ms * STOP_X as u64 / WALK_MS) as u16, walking_pose(ms))
        } else if !done {
            (STOP_X, working_pose(ms))
        } else {
            (STOP_X, idle_pose(ms))
        };
        render(frame, area, origin.x + x, origin.y, pose);

        for (i, (row, bx, w)) in all.iter().take(laid).enumerate() {
            let color = if i % 3 == 1 { BRICK_B } else { BRICK_A };
            let rect = Rect { x: origin.x + WALL_X + bx, y: origin.y + HEIGHT - 1 - row, width: *w, height: 1 };
            let brick = Span::styled("▆".repeat(*w as usize), Style::default().fg(color));
            frame.render_widget(Paragraph::new(Line::from(brick)), rect.intersection(area));
        }
        let ground = Span::styled("▔".repeat(SCENE_W as usize), Style::default().fg(Color::DarkGray));
        let ground_rect = Rect { x: origin.x, y: origin.y + HEIGHT, width: SCENE_W, height: 1 };
        frame.render_widget(Paragraph::new(Line::from(ground)), ground_rect.intersection(area));
        y += HEIGHT + 1;
    }

    y += 1;
    let hint = if done { "obra pronta — abrindo o painel…" } else { "qualquer tecla pula a abertura" };
    let version = format!("v{}  ·  {hint}", env!("CARGO_PKG_VERSION"));
    let footer = Line::from(Span::styled(version, Style::default().fg(Color::DarkGray)));
    frame.render_widget(Paragraph::new(footer).alignment(Alignment::Center), centered(area, area.width, y, 1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const ACTS: [Act; 9] =
        [Act::Pulse, Act::Coin, Act::Review, Act::Carry, Act::Wave, Act::Message, Act::Idea, Act::Wrench, Act::Server];

    fn all_poses() -> Vec<Pose> {
        let mut v = vec![Pose::Idle, Pose::Blink, Pose::WalkA, Pose::WalkB, Pose::HammerUp, Pose::HammerDown];
        for act in ACTS {
            // Um ciclo inteiro de qualquer atividade cabe em 2 s.
            for ms in (0..2000).step_by(50) {
                let p = act_pose(act, ms);
                if !v.contains(&p) {
                    v.push(p);
                }
            }
        }
        v
    }

    #[test]
    fn every_activity_has_two_distinct_frames_unlike_any_other() {
        let mut seen: Vec<[&str; 14]> = vec![pixels(Pose::Idle)];
        for act in ACTS {
            let frames: Vec<Pose> = (0..2000).step_by(50).map(|ms| act_pose(act, ms)).collect();
            let (a, b) = (frames[0], *frames.iter().find(|p| **p != frames[0]).expect("a animação tem de se mexer"));
            for p in [a, b] {
                let g = pixels(p);
                assert!(!seen.contains(&g), "{act:?}/{p:?} repete um desenho que já existe");
                seen.push(g);
            }
        }
        assert_eq!(all_poses().len(), 6 + 2 * ACTS.len());
    }

    /// `OREIA_DUMP_UI=1 cargo test mascot -- --nocapture` imprime cada pose em letras.
    #[test]
    fn dump_poses() {
        if std::env::var("OREIA_DUMP_UI").is_ok() {
            for p in all_poses() {
                println!("{p:?}\n{}\n", pixels(p).join("\n"));
            }
        }
    }

    fn splash(ms: u64, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw_splash(f, ms)).unwrap();
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

    #[test]
    fn every_pose_is_a_full_grid_with_known_pixels() {
        for pose in all_poses() {
            let g = pixels(pose);
            for row in g {
                assert_eq!(row.len(), WIDTH as usize, "{pose:?}: {row:?}");
                assert!(row.bytes().all(|c| c == b'.' || color_of(c).is_some()), "{pose:?}: {row:?}");
            }
            let l = lines(pose);
            assert_eq!(l.len(), HEIGHT as usize);
            assert!(l.iter().all(|line| line.width() == WIDTH as usize), "{pose:?}");
        }
    }

    #[test]
    fn transparent_pixels_paint_nothing() {
        // Canto superior esquerdo é vazio em toda pose: tem de ser espaço sem cor.
        for pose in all_poses() {
            let first = &lines(pose)[0].spans[0];
            assert_eq!(first.content, " ");
            assert_eq!(first.style, Style::default());
        }
    }

    #[test]
    fn poses_follow_the_clock() {
        assert_eq!(idle_pose(0), Pose::Idle);
        assert_eq!(idle_pose(3100), Pose::Blink);
        assert_eq!(idle_pose(3300), Pose::Idle);
        assert_ne!(working_pose(0), working_pose(280));
        assert_ne!(walking_pose(0), walking_pose(160));
    }

    #[test]
    fn wall_is_staggered_and_stays_inside_its_width() {
        let all = bricks();
        assert_eq!(all.iter().filter(|b| b.0 == 0).count(), 6);
        assert_eq!(all.iter().filter(|b| b.0 == 1).count(), 7, "fiada desencontrada começa e termina em meio tijolo");
        assert!(all.iter().all(|(_, x, w)| x + w <= WALL_W && *w >= 1));
        assert!(all.windows(2).all(|p| p[0].0 <= p[1].0), "de baixo para cima");
    }

    #[test]
    fn splash_shows_the_name_and_builds_the_wall_over_time() {
        let start = splash(0, 100, 30);
        assert!(start.contains("█████"), "{start}");
        assert!(start.contains(TAGLINE));
        assert!(start.contains("qualquer tecla"));
        assert!(!start.contains('▆'), "ainda sem tijolo");

        let middle = splash(WALK_MS + 10 * BRICK_MS, 100, 30);
        assert_eq!(middle.matches('▆').count(), bricks().iter().take(10).map(|b| b.2 as usize).sum::<usize>());

        let end = splash(splash_duration_ms(), 100, 30);
        assert_eq!(end.matches('▆').count(), bricks().iter().map(|b| b.2 as usize).sum::<usize>());
        assert!(end.contains("obra pronta"));
    }

    #[test]
    fn splash_degrades_on_small_terminals_without_panicking() {
        // Estreito: nome em texto simples no lugar das letras grandes.
        let narrow = splash(500, 60, 24);
        assert!(narrow.contains(TITLE), "{narrow}");
        // Minúsculo: só não pode quebrar.
        for (w, h) in [(20, 5), (1, 1), (80, 10), (200, 60), (71, 19)] {
            splash(900, w, h);
            splash(splash_duration_ms() + 5000, w, h);
        }
    }
}
