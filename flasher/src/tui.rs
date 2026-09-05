//! A small, pretty live dashboard for KTMicro KT02H20 / FiiO JA11 dongles.
//! Works natively on macOS and Linux, no OrbStack required. `u` attempts an unlock — on
//! macOS this auto-detects the `ktmac` companion binary if it's built (`docs/MACOS-NATIVE.md`),
//! falling back to a `rusb` attempt otherwise — and `f` shows the native‑flash guidance. The
//! native CDC write is proven on hardware on both OSes; the TUI keeps loudly reminding you to
//! **back up first**.

use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    DefaultTerminal, Frame,
};

use crate::{class_name, scan, Dev, Iface, Mode};

// palette
const ACCENT: Color = Color::Rgb(34, 211, 238); // cyan
const MAG: Color = Color::Rgb(232, 121, 249); // magenta
const GREEN: Color = Color::Rgb(74, 222, 128);
const AMBER: Color = Color::Rgb(251, 191, 36);
const RED: Color = Color::Rgb(248, 113, 113);
const DIM: Color = Color::Rgb(100, 116, 139);
const FG: Color = Color::Rgb(226, 232, 240);

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

struct App {
    devs: Vec<Dev>,
    log: Vec<(String, Color)>,
    tick: u64,
    last_scan: Instant,
    running: bool,
    demo: bool,
    start: Instant,
    progress: Option<u16>,
    phase: u8,
}

impl App {
    fn new(demo: bool) -> Self {
        App {
            devs: vec![],
            log: vec![],
            tick: 0,
            last_scan: Instant::now(),
            running: true,
            demo,
            start: Instant::now(),
            progress: None,
            phase: 0,
        }
    }

    /// Scripted flash journey for the demo GIF (no hardware needed).
    fn demo_step(&mut self) {
        let ms = self.start.elapsed().as_millis() as u64;
        let phase = match ms {
            0..=1200 => 0,     // waiting
            1201..=2600 => 1,  // stock detected
            2601..=3200 => 2,  // unlock
            3201..=3600 => 3,  // bootloader
            3601..=5600 => 4,  // programming
            _ => 5,            // JA11 done
        };
        if phase != self.phase {
            self.phase = phase;
            match phase {
                1 => self.logline("detected 31b2:0111 — KT02H20 (stock)", AMBER),
                2 => {
                    self.logline("⚠ back up firmware first — no read-back yet", AMBER);
                    self.logline("unlock → sending T12345678…", AMBER);
                }
                3 => self.logline("rebooted → bootloader 8888:cdc0 (CDC)", MAG),
                4 => self.logline("native write: ktflash flash-cdc (no OrbStack)…", ACCENT),
                5 => {
                    self.logline("UPGRADE FIRMWARE SUCCESS ✓", GREEN);
                    self.logline("now running: JadeAudio JA11 (2972:0102)", GREEN);
                }
                _ => {}
            }
        }
        self.devs = match phase {
            0 => vec![],
            1 | 2 => vec![mock(0x31b2, 0x0111, "KT02H20 HIFI Audio", "KTMicro", Mode::KtStock, true)],
            3 | 4 => vec![mock(0x8888, 0xcdc0, "KTMicro 2021-07-15", "KTMicro", Mode::Bootloader, false)],
            _ => vec![mock(0x2972, 0x0102, "JadeAudio JA11", "FIIO", Mode::Ja11, true)],
        };
        self.progress = if phase == 4 {
            Some((((ms - 3601) as f32 / 2000.0) * 100.0).min(100.0) as u16)
        } else {
            None
        };
    }

    fn logline(&mut self, msg: impl Into<String>, c: Color) {
        self.log.push((msg.into(), c));
        if self.log.len() > 200 {
            self.log.drain(0..self.log.len() - 200);
        }
    }

    fn rescan(&mut self) {
        let before: Vec<(u16, u16)> = self.devs.iter().map(|d| (d.vid, d.pid)).collect();
        self.devs = scan();
        let after: Vec<(u16, u16)> = self.devs.iter().map(|d| (d.vid, d.pid)).collect();
        if before != after {
            if self.devs.is_empty() {
                self.logline("device removed", DIM);
            } else {
                let msgs: Vec<(String, Color)> = self
                    .devs
                    .iter()
                    .filter(|d| !before.contains(&(d.vid, d.pid)))
                    .map(|d| {
                        (
                            format!("detected {:04x}:{:04x} — {}", d.vid, d.pid, d.mode.label()),
                            mode_color(d.mode),
                        )
                    })
                    .collect();
                for (m, c) in msgs {
                    self.logline(m, c);
                }
            }
        }
        self.last_scan = Instant::now();
    }

    fn on_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Char('r') => {
                self.logline("rescan", ACCENT);
                self.rescan();
            }
            KeyCode::Char('u') => {
                if self.devs.iter().any(|d| matches!(d.mode, Mode::Ja11 | Mode::KtStock | Mode::KtFamily)) {
                    self.logline("unlock → reboot to bootloader…", AMBER);
                    match crate::try_unlock() {
                        Ok(m) => self.logline(m, GREEN),
                        Err(e) => {
                            for l in e.lines() {
                                self.logline(l.to_string(), RED);
                            }
                        }
                    }
                } else {
                    self.logline("no normal-mode dongle to unlock", DIM);
                }
            }
            KeyCode::Char('f') => {
                self.logline("native flash (macOS or Linux — no OrbStack required):", ACCENT);
                self.logline("  ⚠ NO backup possible — this can BRICK it. Own risk.", RED);
                self.logline("  1. SAVE your original firmware image first!", RED);
                self.logline("  2. ktflash unlock   → fresh CDC bootloader", DIM);
                self.logline("     (auto-detects ktmac on macOS)", DIM);
                self.logline("  3. ktflash flash-cdc --image fw.bin --execute --yes", GREEN);
                self.logline("  see docs/FLASHING.md — flag auto-derived from image", DIM);
            }
            _ => {}
        }
    }

    fn event_loop(&mut self, term: &mut DefaultTerminal) -> Result<(), String> {
        loop {
            term.draw(|f| self.render(f)).map_err(|e| e.to_string())?;
            if event::poll(Duration::from_millis(120)).map_err(|e| e.to_string())? {
                if let Event::Key(k) = event::read().map_err(|e| e.to_string())? {
                    if k.kind == KeyEventKind::Press {
                        self.on_key(k.code);
                    }
                }
            }
            self.tick += 1;
            if self.demo {
                self.demo_step();
            } else if self.last_scan.elapsed() >= Duration::from_millis(800) {
                self.rescan();
            }
            if !self.running {
                return Ok(());
            }
        }
    }

    fn render(&self, f: &mut Frame) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(6), Constraint::Length(3)])
            .split(f.area());
        self.render_header(f, rows[0]);
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
            .split(rows[1]);
        self.render_device(f, body[0]);
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(body[1]);
        self.render_ifaces(f, right[0]);
        self.render_log(f, right[1]);
        self.render_footer(f, rows[2]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let sp = SPINNER[(self.tick as usize) % SPINNER.len()];
        let line = Line::from(vec![
            Span::styled("  ⚡ ktflash ", Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled("  KTMicro KT02H20 · FiiO JA11 toolkit", Style::default().fg(FG).add_modifier(Modifier::BOLD)),
            Span::styled("   native write ✓", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
            Span::styled(format!("   {sp} scanning USB"), Style::default().fg(DIM)),
        ]);
        f.render_widget(
            Paragraph::new(line).block(bordered().border_style(Style::default().fg(ACCENT))),
            area,
        );
    }

    fn render_device(&self, f: &mut Frame, area: Rect) {
        let block = titled("  device  ", MAG);
        let mut lines: Vec<Line> = vec![];
        if let Some(d) = self.devs.first() {
            let mc = mode_color(d.mode);
            lines.push(Line::from(vec![
                Span::styled("  ◆ ", Style::default().fg(mc)),
                Span::styled(
                    if d.product.is_empty() { d.mode.label().to_string() } else { d.product.clone() },
                    Style::default().fg(FG).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::raw("    "),
                badge(d.mode),
            ]));
            lines.push(Line::from(""));
            lines.push(kv("vendor:product", &format!("{:04x}:{:04x}", d.vid, d.pid)));
            if !d.mfr.is_empty() {
                lines.push(kv("manufacturer", &d.mfr));
            }
            if !d.serial.is_empty() {
                lines.push(kv("serial", &d.serial));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("    firmware journey", Style::default().fg(DIM))));
            lines.push(journey(d.mode));
            if let Some(p) = self.progress {
                lines.push(Line::from(""));
                lines.push(progress_bar(p));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "    ⚠ NO firmware backup is possible —",
                Style::default().fg(RED).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                "      flashing can BRICK it. Keep your",
                Style::default().fg(RED).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                "      original image. Proceed at own risk.",
                Style::default().fg(RED).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                "    press f for native-flash guidance",
                Style::default().fg(DIM),
            )));
        } else {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("    ○ no dongle detected", Style::default().fg(DIM))));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("    plug a KTMicro KT02H20 / FiiO JA11", Style::default().fg(DIM))));
            lines.push(Line::from(Span::styled("    USB-C DAC dongle into this Mac.", Style::default().fg(DIM))));
        }
        f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
    }

    fn render_ifaces(&self, f: &mut Frame, area: Rect) {
        let block = titled("  interfaces  ", ACCENT);
        let mut lines: Vec<Line> = vec![];
        if let Some(d) = self.devs.first() {
            for i in &d.ifaces {
                let cn = class_name(i.class);
                let col = match i.class {
                    3 => AMBER,     // HID
                    1 => ACCENT,    // Audio
                    2 | 10 => MAG,  // CDC
                    _ => DIM,
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  IF{} ", i.num), Style::default().fg(DIM)),
                    Span::styled(format!("{cn:<9}"), Style::default().fg(col).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        if i.eps.is_empty() { "—".to_string() } else { i.eps.join("  ") },
                        Style::default().fg(FG),
                    ),
                ]));
            }
        } else {
            lines.push(Line::from(Span::styled("  —", Style::default().fg(DIM))));
        }
        f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
    }

    fn render_log(&self, f: &mut Frame, area: Rect) {
        let block = titled("  activity  ", GREEN);
        let h = area.height.saturating_sub(2) as usize;
        let start = self.log.len().saturating_sub(h);
        let lines: Vec<Line> = self.log[start..]
            .iter()
            .map(|(m, c)| Line::from(vec![Span::styled("  › ", Style::default().fg(DIM)), Span::styled(m.clone(), Style::default().fg(*c))]))
            .collect();
        f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let key = |k: &str, d: &str| -> Vec<Span> {
            vec![
                Span::styled(format!(" {k} "), Style::default().fg(Color::Black).bg(DIM).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {d}   "), Style::default().fg(DIM)),
            ]
        };
        let mut spans = vec![Span::raw("  ")];
        spans.extend(key("r", "rescan"));
        spans.extend(key("u", "unlock → bootloader"));
        spans.extend(key("f", "flash guide"));
        spans.extend(key("q", "quit"));
        f.render_widget(
            Paragraph::new(Line::from(spans)).block(bordered().border_style(Style::default().fg(DIM))),
            area,
        );
    }
}

fn bordered() -> Block<'static> {
    Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
}
fn titled(title: &str, color: Color) -> Block<'_> {
    bordered()
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(title, Style::default().fg(color).add_modifier(Modifier::BOLD)))
}

fn mode_color(m: Mode) -> Color {
    match m {
        Mode::Ja11 => GREEN,
        Mode::KtStock | Mode::KtFamily => AMBER,
        Mode::Bootloader => MAG,
    }
}

fn badge(m: Mode) -> Span<'static> {
    let (txt, col) = match m {
        Mode::Ja11 => ("  JadeAudio JA11 ✓  ", GREEN),
        Mode::KtStock => ("  KT02H20 stock  ", AMBER),
        Mode::KtFamily => ("  KTMicro dongle  ", AMBER),
        Mode::Bootloader => ("  BOOTLOADER (ISP)  ", MAG),
    };
    Span::styled(txt, Style::default().fg(Color::Black).bg(col).add_modifier(Modifier::BOLD))
}

fn journey(m: Mode) -> Line<'static> {
    let stage = match m {
        Mode::KtStock | Mode::KtFamily => 0,
        Mode::Bootloader => 1,
        Mode::Ja11 => 2,
    };
    let pill = |label: &'static str, idx: usize| -> Span<'static> {
        if idx == stage {
            Span::styled(format!(" {label} "), Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD))
        } else if idx < stage {
            Span::styled(format!(" {label} "), Style::default().fg(GREEN))
        } else {
            Span::styled(format!(" {label} "), Style::default().fg(DIM))
        }
    };
    Line::from(vec![
        Span::raw("    "),
        pill("stock", 0),
        Span::styled(" → ", Style::default().fg(DIM)),
        pill("bootloader", 1),
        Span::styled(" → ", Style::default().fg(DIM)),
        pill("JA11", 2),
    ])
}

fn kv(k: &str, v: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("    {k:<14}", ), Style::default().fg(DIM)),
        Span::styled(v.to_string(), Style::default().fg(FG)),
    ])
}

fn progress_bar(p: u16) -> Line<'static> {
    let width = 26usize;
    let filled = (p as usize * width / 100).min(width);
    let bar: String = "█".repeat(filled) + &"░".repeat(width - filled);
    Line::from(vec![
        Span::raw("    "),
        Span::styled(bar, Style::default().fg(ACCENT)),
        Span::styled(format!(" {p:>3}%"), Style::default().fg(FG).add_modifier(Modifier::BOLD)),
    ])
}

fn mock(vid: u16, pid: u16, product: &str, mfr: &str, mode: Mode, audio_hid: bool) -> Dev {
    let ifaces = if audio_hid {
        vec![
            Iface { num: 0, class: 1, sub: 1, proto: 32, eps: vec![] },
            Iface { num: 1, class: 1, sub: 2, proto: 32, eps: vec!["0x84 IN iso".into()] },
            Iface { num: 2, class: 1, sub: 2, proto: 32, eps: vec!["0x04 OUT iso".into(), "0x85 IN iso".into()] },
            Iface { num: 3, class: 3, sub: 0, proto: 0, eps: vec!["0x83 IN intr".into(), "0x03 OUT intr".into()] },
        ]
    } else {
        vec![
            Iface { num: 0, class: 2, sub: 2, proto: 1, eps: vec!["0x82 IN intr".into()] },
            Iface { num: 1, class: 10, sub: 0, proto: 0, eps: vec!["0x03 OUT bulk".into(), "0x83 IN bulk".into()] },
        ]
    };
    Dev {
        vid,
        pid,
        mfr: mfr.into(),
        product: product.into(),
        serial: "2020-02-20-0000-0000-0000".into(),
        mode,
        ifaces,
    }
}

pub fn run() -> Result<(), String> {
    run_inner(false)
}
pub fn run_demo() -> Result<(), String> {
    run_inner(true)
}
fn run_inner(demo: bool) -> Result<(), String> {
    let mut term = ratatui::init();
    let mut app = App::new(demo);
    app.logline(
        if demo { "demo — simulated flash journey" } else { "ktflash started — watching USB" },
        ACCENT,
    );
    if !demo {
        app.logline("flashing can BRICK the dongle — no firmware backup exists", RED);
        app.logline("keep your original image; flash at your own risk (press f)", AMBER);
        app.rescan();
    }
    let res = app.event_loop(&mut term);
    ratatui::restore();
    res
}
