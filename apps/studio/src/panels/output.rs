//! The Output panel: what the engine, Studio and the game's scripts logged ([`crate::logging`]), with a level
//! filter, a search and Clear.
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
const ERROR: Color32 = Color32::from_rgb(0xFF, 0x7A, 0x7A);
/// An error's colour on the light theme.
const ERROR_LIGHT: Color32 = Color32::from_rgb(0xB4, 0x23, 0x23);

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

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        let state = &mut cx.editor.output;
        state.update();
        ui.horizontal(|ui| {
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
                ui.allocate_ui(vec2(ui.available_width().min(320.0), size::CONTROL), |ui| {
                    search_box(ui, &mut state.query, "Search the output");
                });
            });
        });
        ui.add_space(space::S3);
        let visible = state.visible();
        if visible.is_empty() {
            let line = if state.lines.is_empty() {
                "The output is empty."
            } else {
                "No lines match the filter."
            };
            empty_note(ui, line);
            return;
        }
        let row = text::MONO + space::S3;
        ScrollArea::vertical()
            .id_salt("output.lines")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(ui, row, visible.len(), |ui, range| {
                for line in &visible[range] {
                    line_ui(ui, line, row);
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

/// One line: the level's icon, where it came from, and what it says, in the code font.
fn line_ui(ui: &mut Ui, line: &LogLine, height: f32) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::ListItem, true, &line.message));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let (glyph, tint) = match line.level {
        Level::Error => (Icon::UiError, if colors.dark { ERROR } else { ERROR_LIGHT }),
        Level::Warn => (Icon::UiWarning, colors.accent),
        Level::Info => (Icon::UiInfo, p.subtle),
        Level::Debug | Level::Trace => (Icon::UiInfo, p.border),
    };
    let mut x = rect.left() + space::S2;
    icons::paint_at(
        ui.painter(),
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        glyph,
        text::SMALL + 1.0,
        tint,
    );
    x += text::BODY + space::S2;
    let mono = FontId::monospace(text::MONO);
    let source = match line.origin {
        Origin::Script => line.source.clone(),
        Origin::Engine => line.source.split("::").next().unwrap_or(&line.source).to_owned(),
    };
    let at = ui.painter().text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        &source,
        mono.clone(),
        p.subtle,
    );
    x = at.right() + space::S4;
    let message = line.message.lines().next().unwrap_or_default();
    let clip = Rect::from_min_max(pos2(x, rect.top()), rect.right_bottom());
    let ink = match line.level {
        Level::Error => tint,
        _ => p.text,
    };
    ui.painter().with_clip_rect(clip.intersect(ui.clip_rect())).text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        message,
        mono,
        ink,
    );
    if line.message.contains('\n') || response.hovered() {
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
