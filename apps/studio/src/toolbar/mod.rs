//! The editor's toolbar (#232; ADR 0009, Toolbar): a row of tabs, and under it the front tab's toolbar of tools,
//! spacers and separators.
//!
//! - **Tabs** (Home, Model, UI, Scripts and Plugins by default) can be added (the + after them), renamed in place
//!   (double-click), and moved by dragging one along the row. Right-click a tab for the Tabs window ([`tabs`], #329):
//!   every tab with its name, whether it is shown and whether it loads a layout of its own, moving and deleting, and
//!   where the tabs sit in the row (left, center or right). Switching tab changes the toolbar only, unless the tab
//!   loads a layout of its own: then the dock shows that tab's layout while it is in front.
//! - **Elements** come from the [`ToolRegistry`] ([`tools`]): right-click the toolbar and choose Add... for the Add
//!   window ([`add`], #327), which lists every tool, spacer and separator in groups with what each does; right-click
//!   one to add another before it, move it or remove it, or drag it along the row.
//! - **Saved** as `toolbar.json` beside the settings ([`model`]), each tab's own layout in it.
//!
//! The toolbar runs nothing itself: a click reports the tool's [`ToolAction`] ([`ToolbarEvent::Run`]) for Studio
//! to carry out, which is also how a plugin's tool will reach its plugin.

pub mod add;
pub mod increments;
pub mod model;
pub mod tabs;
pub mod tools;

use std::path::{Path, PathBuf};

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Rect, Response, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2,
};

use crate::dock::Panel;
use crate::settings::Editing;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{size, space, text};
use crate::theme::{PlayControl, current_colors, fonts};
pub use model::{Element, TOOLBAR_FILE, TOOLBAR_VERSION, TabAlign, TabConfig, ToolbarConfig};
pub use tools::{StudioCommand, Tool, ToolAction, ToolError, ToolRegistry};

/// The tab row's height.
pub const TAB_ROW: f32 = 28.0;
/// The tool row's height to start with; the line under it drags it taller or shorter (#414).
pub const TOOL_ROW: f32 = 54.0;
/// How short and how tall the tool row may be dragged, in points.
pub const TOOL_ROW_RANGE: (u16, u16) = (30, 96);
/// The toolbar's whole height to start with.
pub const TOOLBAR_HEIGHT: f32 = TAB_ROW + TOOL_ROW;
/// The shortest tool row whose tools show their labels under their icons; a shorter one shows icons alone.
const LABELS_FROM: f32 = 44.0;
/// How far above the toolbar's bottom line its resize grip reaches, in points.
const GRIP: f32 = 4.0;
/// A tool button's narrowest width.
const TOOL_WIDTH: f32 = 52.0;
/// A spacer's width.
const SPACER_WIDTH: f32 = 24.0;
/// A separator's width (the line in its middle).
const SEPARATOR_WIDTH: f32 = 13.0;
/// A tool's icon size in the tool row as it starts, and as an empty tab's Add tool button has it.
const TOOL_ICON: f32 = 20.0;

/// A tool's icon size in a tool row `row` points tall: [`TOOL_ICON`] at the start, growing a little with the row.
fn icon_side(row: f32) -> f32 {
    (row * TOOL_ICON / TOOL_ROW).clamp(16.0, 28.0)
}
/// The room the + after the tabs takes.
const PLUS_WIDTH: f32 = 28.0;
/// What a tab being renamed has past its name: room for the text cursor (#326).
const RENAME_SLACK: f32 = 2.0;

/// What happened in the toolbar this frame, for Studio to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolbarEvent {
    /// A tool was clicked: do what it does.
    Run(ToolAction),
}

/// A change asked for this frame, applied once the toolbar is drawn.
enum Change {
    SelectTab(usize),
    AddTab,
    StartRename(usize),
    FinishRename,
    CancelRename,
    MoveTab(usize, usize),
    OpenTabs,
    CloseTabs,
    Insert(usize, Element),
    OpenAdd(usize),
    CloseAdd,
    Remove(usize),
    MoveElement(usize, usize),
}

/// The toolbar: what it holds (saved), Studio's and plugins' tools, and what is being edited.
#[derive(Debug)]
pub struct Toolbar {
    config: ToolbarConfig,
    /// What was last read or written.
    saved: ToolbarConfig,
    /// The file, unless the toolbar lives in memory.
    path: Option<PathBuf>,
    registry: ToolRegistry,
    /// The tab being renamed, by index, and its new name so far.
    renaming: Option<(usize, String)>,
    /// The Add window, while it is open.
    adding: Option<add::Adding>,
    /// Whether the Tabs window is open.
    tabs_open: bool,
    /// Whether it draws its lines (under it, in its separators): Settings' Panel borders, as of this pass (#441).
    lines: bool,
}

impl Default for Toolbar {
    /// The default toolbar with Studio's panels' tools, in memory.
    fn default() -> Self {
        Self::in_memory(crate::panels::PANELS)
    }
}

impl Toolbar {
    /// The default toolbar with `panels`' tools, kept in memory only.
    pub fn in_memory(panels: &[&dyn Panel]) -> Self {
        let config = ToolbarConfig::default();
        Self {
            saved: config.clone(),
            config,
            path: None,
            registry: ToolRegistry::studio(panels),
            renaming: None,
            adding: None,
            tabs_open: false,
            lines: true,
        }
    }

    /// The toolbar saved at `path` (the default when there is none), with `panels`' tools.
    pub fn load(path: PathBuf, panels: &[&dyn Panel]) -> Self {
        let config = ToolbarConfig::load(&path);
        Self {
            saved: config.clone(),
            config,
            path: Some(path),
            registry: ToolRegistry::studio(panels),
            renaming: None,
            adding: None,
            tabs_open: false,
            lines: true,
        }
    }

    /// Whether the Add window is open.
    pub fn is_adding(&self) -> bool {
        self.adding.is_some()
    }

    /// Whether the Tabs window is open.
    pub fn is_editing_tabs(&self) -> bool {
        self.tabs_open
    }

    /// Opens or closes the Tabs window.
    pub fn set_editing_tabs(&mut self, open: bool) {
        self.apply(if open { Change::OpenTabs } else { Change::CloseTabs });
    }

    /// Closes the Add window.
    pub fn close_add(&mut self) {
        self.apply(Change::CloseAdd);
    }

    /// Opens the Add window for the front tab, adding at `at` (its end when past it).
    pub fn open_add(&mut self, at: usize) {
        self.apply(Change::OpenAdd(at));
    }

    /// The file it is saved in, unless it lives in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// What it holds.
    pub fn config(&self) -> &ToolbarConfig {
        &self.config
    }

    /// What it holds, to change ([`Toolbar::save_if_changed`] writes it).
    pub fn config_mut(&mut self) -> &mut ToolbarConfig {
        &mut self.config
    }

    /// The tools it can hold.
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// The tools it can hold, to register more (the plugin API, later).
    pub fn registry_mut(&mut self) -> &mut ToolRegistry {
        &mut self.registry
    }

    /// Brings tab `index` to the front.
    pub fn select_tab(&mut self, index: usize) {
        if index < self.config.tabs.len() {
            self.config.active = index;
        }
    }

    /// The tab whose own layout the dock should show now: the front tab's id, when it has one.
    pub fn layout_tab(&self) -> Option<u32> {
        let tab = self.config.active_tab();
        tab.own_layout.then_some(tab.id)
    }

    /// Writes the toolbar if it changed, unless `busy` (a drag is going on). Returns whether it wrote.
    pub fn save_if_changed(&mut self, busy: bool) -> std::io::Result<bool> {
        if busy || self.config == self.saved {
            return Ok(false);
        }
        self.saved = self.config.clone();
        let Some(path) = &self.path else {
            return Ok(false);
        };
        crate::settings::store::write_atomically(path, &self.config.to_json())?;
        Ok(true)
    }

    /// Draws the toolbar filling `ui`'s rect. `enabled` says whether a tool can be used now, `checked` whether it is
    /// on (a panel that is open); the Snap tool edits `editing` (#375). With `lines` off (Settings' Panel borders,
    /// #441) it draws no line under it nor in its separators. Returns what was clicked.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        enabled: impl Fn(&ToolAction) -> bool,
        checked: impl Fn(&ToolAction) -> bool,
        editing: &mut Editing,
        lines: bool,
    ) -> Vec<ToolbarEvent> {
        self.lines = lines;
        let p = current_colors(ui.ctx()).palette;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, p.header);
        if lines {
            // One physical pixel under it, as the dock's lines (#287).
            let pixel = 1.0 / ui.ctx().pixels_per_point();
            let line = Rect::from_min_max(pos2(rect.left(), rect.bottom() - pixel), rect.right_bottom());
            ui.painter().rect_filled(line, 0.0, p.border);
        }
        let tabs = Rect::from_min_size(rect.min, vec2(rect.width(), TAB_ROW));
        let tools = Rect::from_min_size(
            pos2(rect.left(), tabs.bottom()),
            vec2(rect.width(), self.config.tool_row.into()),
        );
        let mut changes = Vec::new();
        let mut events = Vec::new();
        self.tab_row(ui, tabs, &mut changes);
        self.tool_row(ui, tools, &enabled, &checked, editing, &mut changes, &mut events);
        self.resize_grip(ui, tools);
        self.add_window(ui.ctx(), &mut changes);
        if self.tabs_open && !tabs::show(ui.ctx(), &mut self.config) {
            changes.push(Change::CloseTabs);
        }
        for change in changes {
            self.apply(change);
        }
        events
    }

    /// The toolbar's whole height: the tab row and the tool row as dragged (#414).
    pub fn height(&self) -> f32 {
        TAB_ROW + f32::from(self.config.tool_row)
    }

    /// The strip along the toolbar's bottom line (#414): dragging it makes the tool row taller or shorter, as the
    /// lines between panels do; under the pointer and while dragged it shows in the accent, with the resize pointer.
    fn resize_grip(&mut self, ui: &mut Ui, tools: Rect) {
        let grip = Rect::from_min_max(pos2(tools.left(), tools.bottom() - GRIP), tools.max);
        let response = ui.interact(grip, ui.scope_id().with("metallic.toolbar.resize"), Sense::drag());
        response.widget_info(|| WidgetInfo::labeled(Role::Splitter, true, "Toolbar height"));
        // From where the drag began: the row it had, and how far the pointer has gone since.
        let start = response.id.with("start");
        if response.drag_started() {
            ui.data_mut(|data| data.insert_temp(start, tools.height()));
        }
        if response.dragged()
            && let (Some(pointer), Some(origin)) = (
                response.interact_pointer_pos(),
                ui.input(|input| input.pointer.press_origin()),
            )
        {
            let (low, high) = TOOL_ROW_RANGE;
            let began = ui.data(|data| data.get_temp::<f32>(start)).unwrap_or(tools.height());
            let row = (began + pointer.y - origin.y)
                .round()
                .clamp(f32::from(low), f32::from(high));
            self.config.tool_row = row as u16;
            ui.ctx().request_repaint();
        }
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            let accent = current_colors(ui.ctx()).accent;
            let line = Rect::from_min_max(pos2(tools.left(), tools.bottom() - size::BORDER), tools.max);
            ui.painter().rect_filled(line, 0.0, accent);
        }
    }

    fn apply(&mut self, change: Change) {
        let config = &mut self.config;
        match change {
            Change::SelectTab(index) => {
                if index < config.tabs.len() {
                    config.active = index;
                }
            }
            Change::AddTab => {
                let index = config.add_tab("New tab");
                config.active = index;
                self.renaming = Some((index, "New tab".to_owned()));
            }
            Change::StartRename(index) => {
                if let Some(tab) = config.tabs.get(index) {
                    self.renaming = Some((index, tab.name.clone()));
                }
            }
            Change::FinishRename => {
                if let Some((index, name)) = self.renaming.take() {
                    let name = name.trim();
                    if let (Some(tab), false) = (config.tabs.get_mut(index), name.is_empty()) {
                        name.clone_into(&mut tab.name);
                    }
                }
            }
            Change::CancelRename => self.renaming = None,
            Change::MoveTab(from, to) => {
                config.move_tab(from, to);
                self.renaming = None;
            }
            Change::OpenTabs => {
                self.tabs_open = true;
                self.renaming = None;
            }
            Change::CloseTabs => self.tabs_open = false,
            Change::Insert(at, element) => config.insert_element(config.active, at, element),
            Change::OpenAdd(at) => {
                let tab = config.active_tab();
                self.adding = Some(add::Adding {
                    tab: tab.id,
                    at: at.min(tab.elements.len()),
                    query: String::new(),
                });
            }
            Change::CloseAdd => self.adding = None,
            Change::Remove(index) => config.remove_element(config.active, index),
            Change::MoveElement(from, to) => config.move_element(config.active, from, to),
        }
    }

    /// The Add window, while open: what is chosen goes into its tab where it was opened for, the next after it. The
    /// front tab is the one added to; a window left open while another tab is brought forward adds at its end.
    fn add_window(&mut self, ctx: &egui::Context, changes: &mut Vec<Change>) {
        let Some(adding) = self.adding.as_mut() else {
            return;
        };
        let front = self.config.active_tab();
        if adding.tab != front.id {
            adding.tab = front.id;
            adding.at = front.elements.len();
        }
        let (chosen, open) = add::show(ctx, &self.registry, adding);
        if let Some(element) = chosen {
            changes.push(Change::Insert(adding.at, element));
            adding.at += 1;
        }
        if !open {
            changes.push(Change::CloseAdd);
        }
    }

    /// The shown tabs' places in the row `rect` (#329): each with its index, left to right, placed as
    /// [`ToolbarConfig::align`] says, followed by the + that adds a tab. A tab being renamed is as wide as its new name.
    fn tab_layout(&self, ui: &Ui, rect: Rect, font: &FontId) -> (Vec<(usize, Rect)>, Rect) {
        let widths: Vec<(usize, f32)> = self
            .config
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.shown)
            .map(|(index, tab)| {
                let name = match &self.renaming {
                    Some((at, name)) if *at == index => name.clone(),
                    _ => tab.name.clone(),
                };
                let width = ui.painter().layout_no_wrap(name, font.clone(), Color32::WHITE).size().x;
                (index, width + 2.0 * space::S5)
            })
            .collect();
        let tabs: f32 = widths.iter().map(|(_, width)| width + space::S1).sum();
        let total = tabs + PLUS_WIDTH;
        let start = match self.config.align {
            TabAlign::Left => rect.left() + space::S3,
            TabAlign::Center => rect.center().x - total / 2.0,
            TabAlign::Right => rect.right() - space::S3 - total,
        }
        .max(rect.left() + space::S3);
        let mut x = start;
        let rects = widths
            .into_iter()
            .map(|(index, width)| {
                let at = Rect::from_min_size(pos2(x, rect.top()), vec2(width, rect.height()));
                x = at.right() + space::S1;
                (index, at)
            })
            .collect();
        let plus = Rect::from_center_size(pos2(x + PLUS_WIDTH / 2.0, rect.center().y), vec2(22.0, 22.0));
        (rects, plus)
    }

    /// The shown tabs, the + after them, and a tab being dragged. Right-click a tab for the Tabs window.
    fn tab_row(&mut self, ui: &mut Ui, rect: Rect, changes: &mut Vec<Change>) {
        let colors = current_colors(ui.ctx());
        let p = colors.palette;
        let font = fonts::named(fonts::STRONG, text::SMALL + 1.0);
        let (layout, plus) = self.tab_layout(ui, rect, &font);
        let rects: Vec<Rect> = layout.iter().map(|(_, at)| *at).collect();
        let mut dragged = None;
        for &(index, tab_rect) in &layout {
            let tab = &self.config.tabs[index];
            let id = ui.scope_id().with(("metallic.toolbar.tab", tab.id));
            if let Some((_, name)) = self.renaming.as_mut().filter(|(at, _)| *at == index) {
                // In place (#326): the tab as it is drawn, its name where it was, with only a text cursor added. No
                // frame, no margin, and as wide as the name so far, so nothing moves while typing.
                let galley = ui.painter().layout_no_wrap(name.clone(), font.clone(), p.text);
                if index == self.config.active {
                    ui.painter()
                        .rect_filled(tab_rect, CornerRadius::ZERO, crate::dock::active_tab_fill(&p));
                }
                let field = Rect::from_min_size(
                    pos2(tab_rect.left() + space::S5, tab_rect.center().y - galley.size().y / 2.0),
                    // Room for the cursor after the last letter.
                    vec2(galley.size().x + RENAME_SLACK, galley.size().y),
                );
                let field_id = id.with("rename");
                let response = ui.put(
                    field,
                    egui::TextEdit::singleline(name)
                        .id(field_id)
                        .font(font.clone())
                        .text_color(p.text)
                        .frame(egui::Frame::NONE)
                        .desired_width(field.width()),
                );
                if !response.has_focus() && !response.lost_focus() {
                    response.request_focus();
                    // The cursor after the name, nothing selected: typing goes on from where the name ends.
                    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), field_id).unwrap_or_default();
                    let end = egui::text::CCursor::new(name.chars().count());
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                    state.store(ui.ctx(), field_id);
                }
                if response.lost_focus() {
                    let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
                    changes.push(if escape {
                        Change::CancelRename
                    } else {
                        Change::FinishRename
                    });
                }
                continue;
            }
            let galley = ui.painter().layout_no_wrap(tab.name.clone(), font.clone(), p.text);
            let response = ui.interact(tab_rect, id, Sense::click_and_drag());
            let active = index == self.config.active;
            response.widget_info(|| WidgetInfo::selected(Role::Tab, true, active, &tab.name));
            if ui.is_rect_visible(tab_rect) {
                let fill = if active {
                    crate::dock::active_tab_fill(&p)
                } else if response.hovered() {
                    p.header.lerp_to_gamma(p.surface_hover, 0.2)
                } else {
                    Color32::TRANSPARENT
                };
                ui.painter().rect_filled(tab_rect, CornerRadius::ZERO, fill);
                let ink = if active || response.hovered() { p.text } else { p.muted };
                ui.painter().galley(
                    pos2(tab_rect.left() + space::S5, tab_rect.center().y - galley.size().y / 2.0),
                    galley,
                    ink,
                );
            }
            if response.clicked() {
                changes.push(Change::SelectTab(index));
            }
            if response.double_clicked() {
                changes.push(Change::StartRename(index));
            }
            if response.secondary_clicked() {
                changes.push(Change::OpenTabs);
            }
            if response.dragged() {
                dragged = Some(index);
            }
            if response.drag_stopped()
                && let Some(to) = drop_index(&rects, ui.ctx().pointer_latest_pos())
                && let Some(&(to, _)) = layout.get(to)
            {
                changes.push(Change::MoveTab(index, to));
            }
        }
        // The + that adds a tab.
        let response = ui.interact(plus, ui.scope_id().with("metallic.toolbar.add_tab"), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, "New tab"));
        if response.hovered() {
            ui.painter().rect_filled(plus, CornerRadius::same(4), p.surface_hover);
        }
        icons::paint_at(
            ui.painter(),
            plus.center(),
            Align2::CENTER_CENTER,
            Icon::UiAdd,
            text::BODY,
            if response.hovered() { p.text } else { p.subtle },
        );
        if response.on_hover_text("New tab").clicked() {
            changes.push(Change::AddTab);
        }
        if dragged.is_some() {
            paint_drop_marker(ui, &rects, colors.accent);
        }
    }

    /// The front tab's elements; right-click on the row to add one.
    #[allow(clippy::too_many_arguments)]
    fn tool_row(
        &self,
        ui: &mut Ui,
        rect: Rect,
        enabled: &impl Fn(&ToolAction) -> bool,
        checked: &impl Fn(&ToolAction) -> bool,
        editing: &mut Editing,
        changes: &mut Vec<Change>,
        events: &mut Vec<ToolbarEvent>,
    ) {
        let colors = current_colors(ui.ctx());
        let p = colors.palette;
        // The row itself, under the elements: right-click it to add one at the end.
        let row = ui.interact(rect, ui.scope_id().with("metallic.toolbar.row"), Sense::click());
        row.widget_info(|| WidgetInfo::labeled(Role::Toolbar, true, "Toolbar"));
        let elements = &self.config.active_tab().elements;
        row.context_menu(|ui| {
            if ui.button("Add...").clicked() {
                changes.push(Change::OpenAdd(elements.len()));
            }
        });
        if elements.is_empty() {
            // An empty tab offers its first tool (#328): a button shaped as a tool's, opening the Add window.
            let label = "Add tool";
            let galley = ui
                .painter()
                .layout_no_wrap(label.to_owned(), FontId::proportional(text::SMALL), p.text);
            let at = Rect::from_min_size(
                pos2(rect.left() + space::S3, rect.top() + space::S1),
                vec2(
                    (galley.size().x + 2.0 * space::S3).max(TOOL_WIDTH),
                    rect.height() - 2.0 * space::S1,
                ),
            );
            let response = ui.interact(at, ui.scope_id().with("metallic.toolbar.add_tool"), Sense::click());
            response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
            if ui.is_rect_visible(at) {
                if response.hovered() {
                    ui.painter().rect_filled(at, CornerRadius::same(4), p.surface);
                }
                let ink = if response.hovered() { p.text } else { p.muted };
                icons::paint_at(
                    ui.painter(),
                    pos2(at.center().x, at.top() + space::S2),
                    Align2::CENTER_TOP,
                    Icon::UiAdd,
                    TOOL_ICON,
                    ink,
                );
                ui.painter().text(
                    pos2(at.center().x, at.bottom() - space::S2),
                    Align2::CENTER_BOTTOM,
                    label,
                    FontId::proportional(text::SMALL),
                    ink,
                );
            }
            if response
                .on_hover_text("Choose tools for this tab. Right-click the toolbar to add more later.")
                .clicked()
            {
                changes.push(Change::OpenAdd(0));
            }
        }
        let mut x = rect.left() + space::S3;
        let mut rects = Vec::new();
        let mut dragged = false;
        for (index, element) in elements.iter().enumerate() {
            let (width, label) = match element {
                Element::Tool { id } if self.registry.get(id).is_some_and(is_increments) => {
                    (increments::width(rect.height()), "Snap".to_owned())
                }
                Element::Tool { id } => {
                    let label = self.registry.get(id).map_or(id.as_str(), |tool| tool.label.as_str());
                    let width = if rect.height() < LABELS_FROM {
                        // Icons alone in a short row.
                        icon_side(rect.height()) + 2.0 * space::S3
                    } else {
                        let galley =
                            ui.painter()
                                .layout_no_wrap(label.to_owned(), FontId::proportional(text::SMALL), p.text);
                        (galley.size().x + 2.0 * space::S3).max(TOOL_WIDTH)
                    };
                    (width, label.to_owned())
                }
                Element::Spacer => (SPACER_WIDTH, "Spacer".to_owned()),
                Element::Separator => (SEPARATOR_WIDTH, "Separator".to_owned()),
            };
            let at = Rect::from_min_size(
                pos2(x, rect.top() + space::S1),
                vec2(width, rect.height() - 2.0 * space::S1),
            );
            let response = self.element(ui, index, element, at, &label, enabled, checked, editing, events);
            if response.dragged() {
                dragged = true;
            }
            if response.drag_stopped() {
                let pointer = ui.ctx().pointer_latest_pos();
                if let Some(to) = drop_index_in_row(&rects, at, pointer, elements.len())
                    && to != index
                {
                    changes.push(Change::MoveElement(index, to));
                }
            }
            response.context_menu(|ui| {
                if ui.button("Add before...").clicked() {
                    changes.push(Change::OpenAdd(index));
                }
                ui.separator();
                if ui.add_enabled(index > 0, egui::Button::new("Move left")).clicked() {
                    changes.push(Change::MoveElement(index, index - 1));
                }
                if ui
                    .add_enabled(index + 1 < elements.len(), egui::Button::new("Move right"))
                    .clicked()
                {
                    changes.push(Change::MoveElement(index, index + 1));
                }
                ui.separator();
                if ui.button("Remove").clicked() {
                    changes.push(Change::Remove(index));
                }
            });
            rects.push(at);
            x = at.right() + space::S1;
        }
        if dragged {
            paint_drop_marker(ui, &rects, colors.accent);
        }
    }

    /// One element: a tool's button (icon over label), a spacer or a separator. Returns its response, for its menu
    /// and dragging.
    #[allow(clippy::too_many_arguments)]
    fn element(
        &self,
        ui: &mut Ui,
        index: usize,
        element: &Element,
        rect: Rect,
        label: &str,
        enabled: &impl Fn(&ToolAction) -> bool,
        checked: &impl Fn(&ToolAction) -> bool,
        editing: &mut Editing,
        events: &mut Vec<ToolbarEvent>,
    ) -> Response {
        let p = current_colors(ui.ctx()).palette;
        let id = ui
            .scope_id()
            .with(("metallic.toolbar.element", self.config.active_tab().id, index));
        let response = ui.interact(rect, id, Sense::click_and_drag());
        match element {
            Element::Tool { id } if self.registry.get(id).is_some_and(is_increments) => {
                response.widget_info(|| WidgetInfo::labeled(Role::Group, true, "Snap increments"));
                increments::show(ui, rect, editing);
                response
            }
            Element::Tool { id } => {
                let tool = self.registry.get(id);
                let usable = tool.is_some_and(|tool| enabled(&tool.action));
                let on = tool.is_some_and(|tool| checked(&tool.action));
                // "Play tool": apart from the top bar's Play, for screen readers too.
                response.widget_info(|| WidgetInfo::labeled(Role::Button, usable, format!("{label} tool")));
                if ui.is_rect_visible(rect) {
                    if on || (usable && response.hovered()) {
                        let fill = if on { p.surface_hover } else { p.surface };
                        ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
                    }
                    let ink = if usable { p.text } else { p.text.gamma_multiply(0.48) };
                    let colors = current_colors(ui.ctx());
                    let icon = tool.map_or(Icon::ToolbarPlugin, |tool| tool.icon);
                    // The game's controls as the top bar draws them (#325); other tools in their group's colour.
                    let look = tool.and_then(|tool| play_control(&tool.action));
                    let tint = match look {
                        Some(look) if usable => look.ink(&colors),
                        None if usable => icon.color(&colors),
                        _ => ink,
                    };
                    // The row's height decides (#414): the icon grows a little with it, and a short row shows it
                    // alone, in the middle.
                    let row = rect.height() + 2.0 * space::S1;
                    let side = icon_side(row);
                    let labelled = row >= LABELS_FROM;
                    let at = if labelled {
                        Align2::CENTER_TOP.anchor_size(pos2(rect.center().x, rect.top() + space::S2), vec2(side, side))
                    } else {
                        Rect::from_center_size(rect.center(), vec2(side, side))
                    };
                    if look.is_some_and(PlayControl::filled) {
                        icons::paint_filled(ui.painter(), icon, at, tint);
                    } else {
                        icons::paint(ui.painter(), icon, at, tint);
                    }
                    if labelled {
                        ui.painter().text(
                            pos2(rect.center().x, rect.bottom() - space::S2),
                            Align2::CENTER_BOTTOM,
                            label,
                            FontId::proportional(text::SMALL),
                            ink,
                        );
                    }
                }
                let response = match tool {
                    Some(tool) if !tool.description.is_empty() => response.on_hover_text(&tool.description),
                    Some(_) => response,
                    None => response.on_hover_text(format!("`{id}` is not available: its plugin is not loaded.")),
                };
                if usable
                    && response.clicked()
                    && let Some(tool) = tool
                {
                    events.push(ToolbarEvent::Run(tool.action.clone()));
                }
                response
            }
            Element::Spacer => {
                response.widget_info(|| WidgetInfo::labeled(Role::Unknown, true, "Spacer"));
                if response.hovered() {
                    // Shown only under the pointer, so it can be found to move or remove.
                    ui.painter().rect_stroke(
                        rect.shrink(2.0),
                        CornerRadius::same(3),
                        Stroke::new(size::BORDER, p.border),
                        egui::StrokeKind::Inside,
                    );
                }
                response
            }
            Element::Separator => {
                response.widget_info(|| WidgetInfo::labeled(Role::Unknown, true, "Separator"));
                if self.lines {
                    let line =
                        Rect::from_center_size(rect.center(), vec2(size::BORDER, rect.height() - 2.0 * space::S3));
                    ui.painter().rect_filled(line, 0.0, p.border);
                }
                response
            }
        }
    }
}

/// Whether `tool` is drawn as the move and rotate increments (#375) rather than a button.
fn is_increments(tool: &Tool) -> bool {
    tool.action == ToolAction::Studio(StudioCommand::Snap)
}

/// How `action` is drawn when it is one of the game's controls, as on the top bar (#325).
fn play_control(action: &ToolAction) -> Option<PlayControl> {
    match action {
        ToolAction::Studio(StudioCommand::Play) => Some(PlayControl::Play),
        ToolAction::Studio(StudioCommand::Stop) => Some(PlayControl::Stop),
        ToolAction::Studio(StudioCommand::Pause | StudioCommand::Step) => Some(PlayControl::Plain),
        _ => None,
    }
}

/// The index a thing dropped at `pointer` goes to among `rects` (left to right): before the first whose middle is
/// right of it, else the last.
fn drop_index(rects: &[Rect], pointer: Option<egui::Pos2>) -> Option<usize> {
    let pointer = pointer?;
    if rects.is_empty() {
        return None;
    }
    Some(
        rects
            .iter()
            .position(|rect| pointer.x < rect.center().x)
            .unwrap_or(rects.len() - 1),
    )
}

/// Where an element dropped at `pointer` goes, from the elements drawn before it (`before`), itself (`at`) and how
/// many the row has: before an earlier element whose middle is right of the pointer, else at the pointer's place
/// counted in element widths after it.
fn drop_index_in_row(before: &[Rect], at: Rect, pointer: Option<egui::Pos2>, count: usize) -> Option<usize> {
    let pointer = pointer?;
    if let Some(earlier) = before.iter().position(|rect| pointer.x < rect.center().x) {
        return Some(earlier);
    }
    if pointer.x < at.right() {
        return Some(before.len());
    }
    // Past itself: one place per element width further on.
    let steps = ((pointer.x - at.right()) / at.width().max(1.0)).ceil() as usize;
    Some((before.len() + steps).min(count.saturating_sub(1)))
}

/// The accent line where a dragged tab or element would go: at the pointer's nearest gap among `rects`.
fn paint_drop_marker(ui: &Ui, rects: &[Rect], accent: Color32) {
    let Some(pointer) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    let Some(first) = rects.first() else {
        return;
    };
    let x = rects
        .iter()
        .find(|rect| pointer.x < rect.center().x)
        .map_or_else(|| rects.last().map_or(first.right(), Rect::right), Rect::left);
    let line = Rect::from_center_size(pos2(x, first.center().y), vec2(2.0, first.height()));
    ui.ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            Id::unique("metallic.toolbar.drop"),
        ))
        .rect_filled(line, 0.0, accent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_land_between_their_neighbours() {
        let rect = |x: f32| Rect::from_min_size(pos2(x, 0.0), vec2(50.0, 20.0));
        let rects = [rect(0.0), rect(52.0), rect(104.0)];
        assert_eq!(drop_index(&rects, Some(pos2(10.0, 5.0))), Some(0));
        assert_eq!(drop_index(&rects, Some(pos2(90.0, 5.0))), Some(2));
        assert_eq!(
            drop_index(&rects, Some(pos2(500.0, 5.0))),
            Some(2),
            "past the end: the last place"
        );
        assert_eq!(drop_index(&rects, None), None);
        // An element dragged from the middle: back over the first, or on past the last.
        assert_eq!(
            drop_index_in_row(&rects[..1], rects[1], Some(pos2(10.0, 5.0)), 3),
            Some(0)
        );
        assert_eq!(
            drop_index_in_row(&rects[..1], rects[1], Some(pos2(160.0, 5.0)), 3),
            Some(2)
        );
        assert_eq!(
            drop_index_in_row(&rects[..1], rects[1], Some(pos2(60.0, 5.0)), 3),
            Some(1),
            "on itself"
        );
    }
}
