//! The Output panel: what the engine, Studio and the game's scripts logged ([`crate::logging`]), with a level
//! filter, a search and Clear.
//!
//! Each line (#502) is its time of day, where it came from and what it says, coloured by its level: errors red,
//! warnings amber, the engine's messages blue, a script's prints in the text's own colour, debug lines faint. The
//! three dots on its tab bar choose whether the time and the source show, and whether Play empties it first (#503).
//! The lines are text that can be selected, across lines, and copied (#510).
//!
//! The panel copies the lines logged since its last frame from the shared buffer into its own list, so it never holds
//! the buffer's lock while it draws. Clear empties its list; the lines do not come back.

use std::collections::VecDeque;
use std::sync::Arc;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Role, ScrollArea, Sense, Stroke, Ui,
    WidgetInfo, pos2, vec2,
};
use log::Level;

use super::{empty_note, ids, matches_words, search_box};
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::logging::{LogBuffer, LogLine, Origin};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};

/// An error's colour: the one status colour the website's tokens leave to the app.
pub(crate) const ERROR: Color32 = Color32::from_rgb(0xFF, 0x7A, 0x7A);
/// An error's colour on the light theme.
pub(crate) const ERROR_LIGHT: Color32 = Color32::from_rgb(0xB4, 0x23, 0x23);
/// A warning's colour, on the dark and the light theme (#502).
pub(crate) const WARNING: Color32 = Color32::from_rgb(0xFF, 0xC0, 0x57);
pub(crate) const WARNING_LIGHT: Color32 = Color32::from_rgb(0x9A, 0x5B, 0x00);
/// The engine's messages, on the dark and the light theme (#502): a script's prints keep the text's colour.
const ENGINE: Color32 = Color32::from_rgb(0x86, 0xB8, 0xFF);
const ENGINE_LIGHT: Color32 = Color32::from_rgb(0x1F, 0x5F, 0xB8);

/// The levels the filter offers, most serious first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    /// Errors.
    Errors,
    /// Warnings.
    Warnings,
    /// Information, and what scripts print.
    Info,
    /// Debug and trace lines.
    Debug,
}

impl Filter {
    /// Every filter, in the order the toolbar shows them.
    pub const ALL: [Filter; 4] = [Filter::Errors, Filter::Warnings, Filter::Info, Filter::Debug];

    /// The filter a line at `level` falls under.
    pub fn of(level: Level) -> Self {
        match level {
            Level::Error => Filter::Errors,
            Level::Warn => Filter::Warnings,
            Level::Info => Filter::Info,
            Level::Debug | Level::Trace => Filter::Debug,
        }
    }

    /// The name on its toggle.
    pub fn label(self) -> &'static str {
        match self {
            Filter::Errors => "Errors",
            Filter::Warnings => "Warnings",
            Filter::Info => "Messages",
            Filter::Debug => "Debug",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// What the Output panel remembers: its copy of the lines, the filter and the search.
#[derive(Debug)]
pub struct OutputState {
    buffer: Arc<LogBuffer>,
    lines: VecDeque<LogLine>,
    /// The last line number copied.
    seen: u64,
    /// Which levels show, by [`Filter`].
    shown: [bool; 4],
    query: String,
}

impl OutputState {
    /// The Output of `buffer`'s lines, every level but debug shown.
    pub fn new(buffer: Arc<LogBuffer>) -> Self {
        Self {
            buffer,
            lines: VecDeque::new(),
            seen: 0,
            shown: [true, true, true, false],
            query: String::new(),
        }
    }

    /// Copies the lines logged since the last call.
    pub fn update(&mut self) {
        self.seen = self.buffer.read_since(self.seen, &mut self.lines);
        let extra = self.lines.len().saturating_sub(LogBuffer::CAPACITY);
        self.lines.drain(..extra);
    }

    /// Empties the panel. The lines logged so far do not come back.
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Whether lines at `filter` show.
    pub fn is_shown(&self, filter: Filter) -> bool {
        self.shown[filter.index()]
    }

    /// Shows or hides lines at `filter`.
    pub fn set_shown(&mut self, filter: Filter, shown: bool) {
        self.shown[filter.index()] = shown;
    }

    /// The search.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Sets the search.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// Every line the panel holds.
    pub fn lines(&self) -> &VecDeque<LogLine> {
        &self.lines
    }

    /// The lines that show: their level is shown and they match the search (in their source or message).
    pub fn visible(&self) -> Vec<&LogLine> {
        let query = self.query.trim();
        self.lines
            .iter()
            .filter(|line| self.is_shown(Filter::of(line.level)))
            .filter(|line| query.is_empty() || matches_words(&format!("{} {}", line.source, line.message), query))
            .collect()
    }

    /// How many lines the panel holds at `filter`'s levels.
    pub fn count(&self, filter: Filter) -> usize {
        self.lines
            .iter()
            .filter(|line| Filter::of(line.level) == filter)
            .count()
    }
}

/// The Output panel.
#[derive(Clone, Copy, Debug)]
pub struct Output;

impl Panel for Output {
    fn id(&self) -> &'static str {
        ids::OUTPUT
    }

    fn title(&self) -> &'static str {
        "Output"
    }

    fn icon(&self) -> Icon {
        Icon::PanelOutput
    }

    fn zone(&self) -> Zone {
        Zone::Bottom
    }

    fn in_front_by_default(&self) -> bool {
        // Assets and Output share the bottom bar; Output is the one open (#256).
        true
    }

    fn context(&self) -> Context {
        Context::Output
    }

    fn has_menu(&self) -> bool {
        true
    }

    /// What the lines show (#502), and whether Play empties the Output (#503).
    fn menu(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        ui.set_min_width(crate::theme::MENU_WIDTH);
        ui.spacing_mut().item_spacing.y = space::S2;
        let output = &mut cx.settings.output;
        crate::theme::section(ui, "metallic.output.menu.lines", "Lines", |ui| {
            crate::theme::checkbox(ui, &mut output.show_time, "Time");
            crate::theme::checkbox(ui, &mut output.show_source, "Source");
        });
        crate::theme::section(ui, "metallic.output.menu.play", "Play", |ui| {
            crate::theme::checkbox(ui, &mut output.clear_on_play, "Clear on Play");
        });
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        let shown = cx.settings.output;
        let state = &mut cx.editor.output;
        state.update();
        // One row of a fixed height, everything centred in it (#275): the toggles, the search box and Clear on a line.
        let toolbar = vec2(ui.available_width(), size::CONTROL);
        ui.allocate_ui_with_layout(toolbar, Layout::left_to_right(Align::Center), |ui| {
            ui.set_height(size::CONTROL);
            for filter in Filter::ALL {
                let count = state.count(filter);
                let mut on = state.is_shown(filter);
                if chip(ui, filter, count, &mut on) {
                    state.set_shown(filter, on);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, ButtonKind::Quiet, "Clear")
                    .on_hover_text("Empty the Output")
                    .clicked()
                {
                    state.clear();
                }
                ui.add_space(space::S3);
                let search = vec2(ui.available_width().min(320.0), size::CONTROL);
                ui.allocate_ui_with_layout(search, Layout::left_to_right(Align::Center), |ui| {
                    search_box(ui, &mut state.query, "Search the output");
                });
            });
        });
        ui.add_space(space::S3);
        let visible = state.visible();
        if visible.is_empty() {
            // An empty Output is just empty (#286); a filter that hides every line says so.
            if !state.lines.is_empty() {
                empty_note(ui, "No lines match the filter.");
            }
            return;
        }
        let row = text::MONO + space::S3;
        ScrollArea::vertical()
            .id_salt("output.lines")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(ui, row, visible.len(), |ui, range| {
                for line in &visible[range] {
                    line_ui(ui, line, row, shown);
                }
            });
    }
}

/// A level's toggle: its name and how many lines it has, raised when on.
fn chip(ui: &mut Ui, filter: Filter, count: usize, on: &mut bool) -> bool {
    let p = current_colors(ui.ctx()).palette;
    let label = format!("{} {count}", filter.label());
    let text = RichText::new(&label)
        .font(fonts::named(fonts::STRONG, text::SMALL))
        .color(if *on { p.text } else { p.subtle });
    let response = ui.add(
        egui::Button::new(text)
            .fill(if *on { p.surface_hover } else { Color32::TRANSPARENT })
            .stroke(Stroke::new(
                size::BORDER,
                if *on { p.border } else { Color32::TRANSPARENT },
            ))
            .corner_radius(CornerRadius::same(radius::ITEM))
            .min_size(vec2(0.0, 24.0)),
    );
    response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, *on, filter.label()));
    if response.clicked() {
        *on = !*on;
        return true;
    }
    false
}

/// A line's colour by its level and origin (#502): errors red, warnings amber, the engine's messages blue, a script's
/// prints the text's colour, debug faint.
fn line_colour(line: &LogLine, colors: &crate::theme::Colors) -> Color32 {
    let p = colors.palette;
    let pick = |dark: Color32, light: Color32| if colors.dark { dark } else { light };
    match (line.level, line.origin) {
        (Level::Error, _) => pick(ERROR, ERROR_LIGHT),
        (Level::Warn, _) => pick(WARNING, WARNING_LIGHT),
        (Level::Info, Origin::Script) => p.text,
        (Level::Info, Origin::Engine) => pick(ENGINE, ENGINE_LIGHT),
        (Level::Debug | Level::Trace, _) => p.subtle,
    }
}

/// When `line` was logged, as the local time of day: `14:03:27.512`.
pub fn time_of_day(line: &LogLine) -> String {
    chrono::DateTime::<chrono::Local>::from(line.time)
        .format("%H:%M:%S%.3f")
        .to_string()
}

/// The text of `line` as it shows (#502): its time and its source when `shown` says, then what it says (its first
/// line; the rest on hover).
fn line_text(line: &LogLine, shown: crate::settings::Output, colors: &crate::theme::Colors) -> egui::text::LayoutJob {
    let p = colors.palette;
    let mono = FontId::monospace(text::MONO);
    let format = |color: Color32| egui::TextFormat {
        font_id: mono.clone(),
        color,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    if shown.show_time {
        job.append(&time_of_day(line), 0.0, format(p.subtle));
        job.append("  ", 0.0, format(p.subtle));
    }
    if shown.show_source {
        let source = match line.origin {
            Origin::Script => line.source.as_str(),
            Origin::Engine => line.source.split("::").next().unwrap_or(&line.source),
        };
        job.append(source, 0.0, format(p.muted));
        job.append("  ", 0.0, format(p.muted));
    }
    let message = line.message.lines().next().unwrap_or_default();
    job.append(message, 0.0, format(line_colour(line, colors)));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

/// One line: the level's icon, then its text in the code font, coloured by level (#502) and selectable (#510).
fn line_ui(ui: &mut Ui, line: &LogLine, height: f32, shown: crate::settings::Output) {
    let colors = current_colors(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let glyph = match line.level {
        Level::Error => Icon::UiError,
        Level::Warn => Icon::UiWarning,
        Level::Info | Level::Debug | Level::Trace => Icon::UiInfo,
    };
    let x = rect.left() + space::S2;
    icons::paint_at(
        ui.painter(),
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        glyph,
        text::SMALL + 1.0,
        line_colour(line, &colors),
    );
    let words = Rect::from_min_max(pos2(x + text::BODY + space::S2, rect.top()), rect.right_bottom());
    let mut job = line_text(line, shown, &colors);
    job.wrap.max_width = words.width();
    // A label that can be selected, across lines, and copied with Ctrl+C or its menu (#510).
    let response = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(words)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.add(
                    egui::Label::new(job)
                        .selectable(true)
                        .wrap_mode(egui::TextWrapMode::Truncate),
                )
            },
        )
        .inner;
    response.widget_info(|| WidgetInfo::labeled(Role::ListItem, true, &line.message));
    if line.message.contains('\n') {
        response.on_hover_text(format!("{}\n{}", line.source, line.message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> OutputState {
        let buffer = Arc::new(LogBuffer::new());
        buffer.push(Level::Info, "metallic_project", "opened Space Race", Origin::Engine);
        buffer.push(
            Level::Warn,
            "metallic_engine::mesh_assets",
            "Meshes/Rock.glb is missing",
            Origin::Engine,
        );
        buffer.push(
            Level::Error,
            "Workspace.Door.Script:3",
            "attempt to index nil",
            Origin::Script,
        );
        buffer.push(Level::Debug, "metallic_render", "pipelines ready", Origin::Engine);
        let mut state = OutputState::new(buffer);
        state.update();
        state
    }

    fn messages(state: &OutputState) -> Vec<&str> {
        state.visible().iter().map(|line| line.message.as_str()).collect()
    }

    #[test]
    fn the_filter_hides_levels_and_debug_starts_hidden() {
        let mut state = state();
        assert_eq!(messages(&state).len(), 3);
        state.set_shown(Filter::Info, false);
        assert_eq!(messages(&state), ["Meshes/Rock.glb is missing", "attempt to index nil"]);
        state.set_shown(Filter::Debug, true);
        assert!(messages(&state).contains(&"pipelines ready"));
        assert_eq!(state.count(Filter::Errors), 1);
    }

    #[test]
    fn the_search_reads_the_source_and_the_message() {
        let mut state = state();
        state.set_query("door");
        assert_eq!(messages(&state), ["attempt to index nil"]);
        state.set_query("ROCK missing");
        assert_eq!(messages(&state), ["Meshes/Rock.glb is missing"]);
    }

    /// #502: a line is its time of day, its source and its message, as the menu says, coloured by level.
    #[test]
    fn a_line_shows_its_time_and_source_as_chosen_and_is_coloured_by_level() {
        let state = state();
        let colors = crate::theme::Colors::default();
        let error = state.lines().iter().find(|line| line.level == Level::Error).unwrap();
        let time = time_of_day(error);
        // hh:mm:ss.mmm
        assert_eq!(time.len(), 12, "{time}");
        assert!(
            time.chars().enumerate().all(|(i, c)| if [2, 5].contains(&i) {
                c == ':'
            } else if i == 8 {
                c == '.'
            } else {
                c.is_ascii_digit()
            }),
            "{time}"
        );
        let all = crate::settings::Output::default();
        let text = line_text(error, all, &colors).text;
        assert!(text.starts_with(&time), "{text}");
        assert!(
            text.contains("Workspace.Door.Script:3") && text.ends_with("attempt to index nil"),
            "{text}"
        );
        let bare = crate::settings::Output {
            show_time: false,
            show_source: false,
            clear_on_play: true,
        };
        assert_eq!(line_text(error, bare, &colors).text, "attempt to index nil");
        // The engine's module only, not its path.
        let warning = state.lines().iter().find(|line| line.level == Level::Warn).unwrap();
        let text = line_text(warning, all, &colors).text;
        assert!(
            text.contains("metallic_engine  ") && !text.contains("mesh_assets"),
            "{text}"
        );
        // Each level its own colour; a script's print the text's.
        let colour = |level: Level, origin: Origin| {
            let line = LogLine {
                level,
                origin,
                ..error.clone()
            };
            line_colour(&line, &colors)
        };
        let colours = [
            colour(Level::Error, Origin::Engine),
            colour(Level::Warn, Origin::Engine),
            colour(Level::Info, Origin::Engine),
            colour(Level::Info, Origin::Script),
            colour(Level::Debug, Origin::Engine),
        ];
        for (i, a) in colours.iter().enumerate() {
            for b in &colours[i + 1..] {
                assert_ne!(a, b, "{colours:?}");
            }
        }
        assert_eq!(colour(Level::Info, Origin::Script), colors.palette.text);
    }

    #[test]
    fn clear_empties_and_only_new_lines_come_in() {
        let mut state = state();
        state.clear();
        state.update();
        assert!(state.lines().is_empty());
        state.buffer.push(Level::Info, "metallic", "after", Origin::Engine);
        state.update();
        assert_eq!(messages(&state), ["after"]);
    }
}
