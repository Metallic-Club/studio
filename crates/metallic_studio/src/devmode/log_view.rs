//! The log viewer: Studio's `log` records (what `RUST_LOG` prints to the console), kept in a
//! ring of the last [`CAPACITY`] lines and shown in Developer > Log, filtered by level and text.
//!
//! [`install`] replaces `env_logger::init()`: records still go to the console as before, and
//! every record at `info` or above (plus whatever `RUST_LOG` enables) is also kept for the viewer
//! and for bug reports.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use eframe::egui::{self, RichText};
use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::theme::{self, fonts};
use crate::widgets::{Button, segmented};

/// Lines kept.
pub const CAPACITY: usize = 2000;
/// Longest kept message, in bytes; longer ones are cut.
pub const MAX_LINE: usize = 2000;

/// One captured record.
#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    /// Seconds since the capture started.
    pub time: f64,
    /// Severity.
    pub level: Level,
    /// Module path (`metallic_studio::viewport`).
    pub target: String,
    /// Message.
    pub text: String,
}

impl LogLine {
    /// "12.345 WARN metallic_studio::app: text".
    pub fn format(&self) -> String {
        format!(
            "{:9.3} {:5} {}: {}",
            self.time, self.level, self.target, self.text
        )
    }
}

struct Ring {
    start: Instant,
    lines: VecDeque<LogLine>,
}

fn ring() -> &'static Mutex<Ring> {
    static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
    RING.get_or_init(|| {
        Mutex::new(Ring {
            start: Instant::now(),
            lines: VecDeque::new(),
        })
    })
}

/// Keeps one line (the viewer and tests call this; [`install`]'s logger calls it for records).
pub fn push(level: Level, target: &str, text: &str) {
    let mut text = text.to_owned();
    if text.len() > MAX_LINE {
        let mut cut = MAX_LINE;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push('…');
    }
    if let Ok(mut r) = ring().lock() {
        let time = r.start.elapsed().as_secs_f64();
        if r.lines.len() >= CAPACITY {
            r.lines.pop_front();
        }
        r.lines.push_back(LogLine {
            time,
            level,
            target: target.to_owned(),
            text,
        });
    }
}

/// A copy of the kept lines, oldest first.
pub fn lines() -> Vec<LogLine> {
    ring()
        .lock()
        .map(|r| r.lines.iter().cloned().collect())
        .unwrap_or_default()
}

/// Forgets the kept lines.
pub fn clear() {
    if let Ok(mut r) = ring().lock() {
        r.lines.clear();
    }
}

/// Every kept line as text, one per line (bug reports).
pub fn text() -> String {
    let mut out = String::new();
    for l in lines() {
        let _ = writeln!(out, "{}", l.format());
    }
    out
}

struct Capture {
    console: env_logger::Logger,
}

impl Log for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Info || self.console.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if record.level() <= Level::Info || self.console.enabled(record.metadata()) {
            push(record.level(), record.target(), &record.args().to_string());
        }
        if self.console.matches(record) {
            self.console.log(record);
        }
    }

    fn flush(&self) {
        self.console.flush();
    }
}

/// Installs the logger: the console as `RUST_LOG` says (like `env_logger::init()`), plus the
/// capture. Does nothing if a logger is already installed.
pub fn install() {
    let console = env_logger::Builder::from_default_env().build();
    let max = console.filter().max(LevelFilter::Info);
    if log::set_boxed_logger(Box::new(Capture { console })).is_ok() {
        log::set_max_level(max);
    }
}

/// The viewer's filters.
#[derive(Clone, Debug)]
pub struct LogView {
    /// Lowest level shown.
    pub level: LevelFilter,
    /// Text the line must contain (case ignored); empty shows all.
    pub filter: String,
}

impl Default for LogView {
    fn default() -> Self {
        LogView {
            level: LevelFilter::Info,
            filter: String::new(),
        }
    }
}

impl LogView {
    /// The kept lines this filter shows, oldest first.
    pub fn visible(&self, lines: &[LogLine]) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.level <= self.level)
            .filter(|(_, l)| {
                needle.is_empty()
                    || l.text.to_lowercase().contains(&needle)
                    || l.target.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Draws the Log window. Returns false when it was closed.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        egui::Window::new(super::overlay::window_title("Log"))
            .id(egui::Id::unique("dev-log"))
            .open(&mut open)
            .default_size([640.0, 360.0])
            .default_pos([300.0, 260.0])
            .order(egui::Order::Foreground)
            .frame(super::overlay::window_frame())
            .show(ctx, |ui| self.contents(ui));
        open
    }

    fn contents(&mut self, ui: &mut egui::Ui) {
        let all = lines();
        ui.horizontal(|ui| {
            segmented(
                ui,
                &mut self.level,
                &[
                    (LevelFilter::Error, "Errors"),
                    (LevelFilter::Warn, "Warnings"),
                    (LevelFilter::Info, "Info"),
                    (LevelFilter::Debug, "Debug"),
                    (LevelFilter::Trace, "All"),
                ],
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter")
                    .desired_width(180.0),
            );
            if ui.add(Button::ghost("Copy").compact()).clicked() {
                let text: String = self
                    .visible(&all)
                    .into_iter()
                    .map(|i| all[i].format() + "\n")
                    .collect();
                ui.ctx().copy_text(text);
            }
            if ui.add(Button::ghost("Clear").compact()).clicked() {
                clear();
            }
        });
        let shown = self.visible(&all);
        ui.label(
            RichText::new(format!("{} of {} lines", shown.len(), all.len()))
                .font(fonts::small())
                .color(theme::text_3()),
        );
        ui.add_space(6.0);
        if shown.is_empty() {
            ui.label(RichText::new("No log lines match.").color(theme::text_3()));
            return;
        }
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(ui, row, shown.len(), |ui, range| {
                // One line per record (scroll sideways for long ones), so rows stay one height.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for &i in &shown[range] {
                    let l = &all[i];
                    let color = match l.level {
                        Level::Error => theme::error(),
                        Level::Warn => theme::warning(),
                        Level::Info => theme::text_1(),
                        _ => theme::text_3(),
                    };
                    let text = l.format().replace('\n', " ");
                    ui.label(RichText::new(text).font(mono.clone()).color(color));
                }
            });
    }
}
