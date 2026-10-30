//! The Controls page (docs/design/input-actions.md sections 8 and 11) as plain state: tabs, rows,
//! focus, listening, the conflict and reset dialogs, the Undo toast. No window and no egui
//! context are needed, so it is unit tested; [`super::paint`] draws it.
//!
//! Keyboard and gamepad drive everything through [`Nav`] commands; the mouse uses
//! [`ControlsPage::hover`], [`ControlsPage::click`] and [`ControlsPage::scroll_by`].
use egui::{Rect, pos2, vec2};
use metallic_runtime::input::glyph::{
    ControlGlyph, GamepadStyle, Glyph, control_glyph, slot_glyph,
};
use metallic_runtime::input::overrides::{
    DEADZONE_RANGE, GamepadGlyphPref, HOLD_SCALE_RANGE, PlayerOverrides, SENSITIVITY_RANGE, Slot,
    Stick, StickSettings,
};
use metallic_runtime::input::rebind::{
    self, Conflict, LISTEN_TIMEOUT, RebindError, Resolution, SlotKind, SlotRef, UNDO_SECONDS,
};
use metallic_runtime::input::{Control, ControlKind, DeviceKind, InputConfig, Role};

/// Width of the page content in points.
pub const CONTENT_W: f32 = 720.0;
/// Left edge of the first slot column.
pub const SLOT_X: [f32; 2] = [212.0, 360.0];
/// Width of a slot.
pub const SLOT_W: f32 = 140.0;
/// Hold/Toggle switch column.
pub const HOLD_X: f32 = 512.0;
/// Width of the Hold/Toggle switch.
pub const HOLD_W: f32 = 140.0;
/// Reset button column.
pub const RESET_X: f32 = 664.0;
/// Size of the reset button.
pub const RESET_W: f32 = 40.0;
/// Left edge of a setting's control.
pub const SETTING_X: f32 = 360.0;
/// Width of a setting's control.
pub const SETTING_W: f32 = 292.0;
const ROW_H: f32 = 48.0;
const GROUP_H: f32 = 36.0;
const SECTION_H: f32 = 44.0;
const HEADER_H: f32 = 28.0;
const BANNER_H: f32 = 44.0;
const CELL_PAD: f32 = 6.0;
/// Seconds a short notice ("That control is used by ...") stays.
const NOTICE_SECONDS: f64 = 4.0;

/// A tab of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Keyboard and mouse bindings, mouse sensitivity and inversion.
    Keyboard,
    /// Gamepad bindings, stick settings, glyph style.
    Gamepad,
    /// Hold and toggle assists.
    Assist,
}

impl Tab {
    /// The tabs in order. (The Touch tab comes with the touch overlay, #252.)
    pub const ALL: [Tab; 3] = [Tab::Keyboard, Tab::Gamepad, Tab::Assist];

    /// The title.
    pub fn title(self) -> &'static str {
        match self {
            Tab::Keyboard => "Keyboard and Mouse",
            Tab::Gamepad => "Gamepad",
            Tab::Assist => "Hold and Toggle",
        }
    }

    /// The device whose bindings the tab shows.
    pub fn device(self) -> Option<DeviceKind> {
        match self {
            Tab::Keyboard => Some(DeviceKind::KeyboardMouse),
            Tab::Gamepad => Some(DeviceKind::Gamepad),
            Tab::Assist => None,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Something the focus can rest on.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A binding slot (part).
    Slot(SlotRef),
    /// A row's Hold / Toggle switch.
    Hold(String),
    /// A row's reset button.
    ResetRow(String),
    /// "Swap WASD and arrows" for a direction action.
    SwapArrows(String),
    /// A slider.
    Slider(SliderId),
    /// An on/off switch.
    Switch(SwitchId),
    /// The gamepad glyph style (Auto, Letters, Positions).
    GlyphStyle,
    /// "Reset this tab".
    ResetTab,
    /// "Reset all".
    ResetAll,
}

/// The sliders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderId {
    /// Sensitivity of a stick or the mouse.
    Sensitivity(Stick),
    /// Dead zone of a stick.
    Deadzone(Stick),
    /// The `HoldTime` scale.
    HoldScale,
}

/// The switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchId {
    /// Invert the x axis of a stick or the mouse.
    InvertX(Stick),
    /// Invert the y axis.
    InvertY(Stick),
    /// "Toggle every hold action".
    ToggleAll,
    /// "Replace holds with presses".
    ReplaceHolds,
}

/// What a cell shows.
#[derive(Debug, Clone, PartialEq)]
pub enum CellArt {
    /// Nothing precomputed.
    None,
    /// A whole slot's glyph.
    Glyph(Glyph),
    /// One part of a composite: its glyph (`None`: empty part) and the direction it stands for.
    Part(Option<ControlGlyph>, usize),
}

/// One focusable cell, in content coordinates (x from the content's left, y from its top).
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// What it is.
    pub item: Item,
    /// Where it is.
    pub rect: Rect,
    /// Precomputed glyph, so painting allocates nothing for it.
    pub art: CellArt,
}

/// What a row is.
#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    /// Column titles.
    Header,
    /// "N actions have no binding".
    Banner(usize),
    /// A `RebindGroup` title.
    Group(String),
    /// A section title.
    Section(&'static str),
    /// An action.
    Action {
        /// `Context/Action`.
        path: String,
        /// Display name.
        name: String,
        /// Enabled now.
        enabled: bool,
        /// Has no binding on this tab's device although the creator gave it one.
        unbound: bool,
        /// The row toggles.
        toggle: bool,
    },
    /// A setting with a label and help text.
    Setting(&'static str, &'static str),
    /// The two reset buttons.
    Footer,
}

/// One row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// What it is.
    pub kind: RowKind,
    /// Top in content coordinates.
    pub top: f32,
    /// Height.
    pub height: f32,
    /// Focusable cells, left to right.
    pub cells: Vec<Cell>,
}

/// A navigation command from the keyboard or a gamepad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    /// Move focus up.
    Up,
    /// Move focus down.
    Down,
    /// Move left, or lower a slider.
    Left,
    /// Move right, or raise a slider.
    Right,
    /// Choose.
    Accept,
    /// Back or cancel.
    Back,
    /// Next tab.
    NextTab,
    /// Previous tab.
    PrevTab,
    /// Clear the focused slot.
    Clear,
    /// Reset the focused row; while the Undo toast shows, undo instead.
    Reset,
}

/// The conflict dialog's buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Swap.
    Swap,
    /// Replace.
    Replace,
    /// Cancel.
    Cancel,
}

/// Which reset a confirmation is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetKind {
    /// This tab.
    Tab,
    /// Everything.
    All,
}

/// What the page is doing.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// Moving the focus.
    Browse,
    /// Waiting for a control for `target` (spec 8.2).
    Listening {
        /// The slot part.
        target: SlotRef,
        /// When listening started.
        since: f64,
        /// A modifier key that went down: it binds on release unless another key follows.
        modifier: Option<Control>,
    },
    /// The control is in use: Swap, Replace or Cancel (spec 8.3).
    Conflict {
        /// The slot part.
        target: SlotRef,
        /// The control the player pressed.
        control: Control,
        /// The other use.
        conflict: Conflict,
        /// The focused button.
        choice: Choice,
    },
    /// "Reset ...?" with Reset and Cancel.
    Confirm {
        /// What to reset.
        reset: ResetKind,
        /// Reset is focused (else Cancel).
        yes: bool,
    },
}

/// The Undo toast after a reset (10 s).
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    /// Its text.
    pub text: String,
    /// When it goes away.
    pub until: f64,
    /// The overrides before the reset.
    pub undo: PlayerOverrides,
}

/// What a call changed, for the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outcome {
    /// The overrides changed: apply them to the runtime.
    pub changed: bool,
    /// Leave the page.
    pub back: bool,
}

/// The Controls page.
#[derive(Debug, Clone)]
pub struct ControlsPage {
    base: InputConfig,
    overrides: PlayerOverrides,
    style: GamepadStyle,
    tab: Tab,
    rows: Vec<Row>,
    focus: Option<(usize, usize)>,
    /// Remembered focus per tab.
    tab_focus: [Option<Item>; 3],
    /// Content scrolled up by this many points.
    pub scroll: f32,
    /// Height of the visible part of the content, set by the painter.
    pub viewport_h: f32,
    /// Screen position of the content's top-left corner (before scrolling), set by the painter.
    pub origin: egui::Pos2,
    /// Screen rectangle of the listening dialog's Cancel button, set by the painter.
    pub cancel_rect: Option<Rect>,
    /// Screen rectangles of the dialog buttons (conflict or confirm), set by the painter.
    pub dialog_rects: Vec<(Rect, Choice)>,
    /// Screen rectangles of the tabs, set by the painter.
    pub tab_rects: Vec<(Rect, Tab)>,
    /// Screen rectangle of the toast's Undo button, set by the painter.
    pub undo_rect: Option<Rect>,
    mode: Mode,
    toast: Option<Toast>,
    notice: Option<(String, f64)>,
    clock: f64,
}

impl ControlsPage {
    /// A page over the creator's `base` config and the player's `overrides`, glyphs in `style`.
    pub fn new(base: InputConfig, overrides: PlayerOverrides, style: GamepadStyle) -> Self {
        let mut page = Self {
            base,
            overrides,
            style,
            tab: Tab::Keyboard,
            rows: Vec::new(),
            focus: None,
            tab_focus: Default::default(),
            scroll: 0.0,
            viewport_h: 480.0,
            origin: pos2(0.0, 0.0),
            cancel_rect: None,
            dialog_rects: Vec::new(),
            tab_rects: Vec::new(),
            undo_rect: None,
            mode: Mode::Browse,
            toast: None,
            notice: None,
            clock: 0.0,
        };
        page.rebuild();
        page.focus_first();
        page
    }

    /// The player's overrides as edited so far.
    pub fn overrides(&self) -> &PlayerOverrides {
        &self.overrides
    }

    /// The creator's config.
    pub fn base(&self) -> &InputConfig {
        &self.base
    }

    /// The tab shown.
    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// The rows of the tab shown.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The focused row and cell.
    pub fn focus(&self) -> Option<(usize, usize)> {
        self.focus
    }

    /// The focused item.
    pub fn focused_item(&self) -> Option<&Item> {
        self.focus
            .and_then(|(r, c)| self.rows.get(r)?.cells.get(c))
            .map(|c| &c.item)
    }

    /// The mode.
    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// The Undo toast, while it shows.
    pub fn toast(&self) -> Option<&Toast> {
        self.toast.as_ref()
    }

    /// A short notice, while it shows.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|(s, _)| s.as_str())
    }

    /// Glyph style for the gamepad cells.
    pub fn style(&self) -> GamepadStyle {
        self.style
    }

    /// Height of all rows.
    pub fn content_height(&self) -> f32 {
        self.rows.last().map_or(0.0, |r| r.top + r.height) + 8.0
    }

    /// The creator's and player's base config changed under the page (the scene reloaded).
    pub fn set_base(&mut self, base: InputConfig) {
        if base != self.base {
            self.base = base;
            self.rebuild_keep_focus();
        }
    }

    /// The glyph style changed (a different pad, or the setting).
    pub fn set_style(&mut self, style: GamepadStyle) {
        if style != self.style {
            self.style = style;
            self.rebuild_keep_focus();
        }
    }

    /// Seconds left before listening gives up.
    pub fn listen_left(&self) -> f64 {
        match &self.mode {
            Mode::Listening { since, .. } => (LISTEN_TIMEOUT - (self.clock - since)).max(0.0),
            _ => 0.0,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Rows

    fn rebuild_keep_focus(&mut self) {
        let item = self.focused_item().cloned();
        self.rebuild();
        self.focus = item.and_then(|i| self.find(&i)).or(self.focus);
        self.clamp_focus();
    }

    fn find(&self, item: &Item) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().find_map(|(r, row)| {
            row.cells
                .iter()
                .position(|c| c.item == *item)
                .map(|c| (r, c))
        })
    }

    fn clamp_focus(&mut self) {
        let valid = self
            .focus
            .is_some_and(|(r, c)| self.rows.get(r).is_some_and(|row| c < row.cells.len()));
        if !valid {
            self.focus_first();
        }
    }

    fn focus_first(&mut self) {
        self.focus = self
            .rows
            .iter()
            .position(|r| !r.cells.is_empty())
            .map(|r| (r, 0));
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let mut y = 0.0;
        match self.tab.device() {
            Some(device) => self.binding_rows(device, &mut rows, &mut y),
            None => self.assist_rows(&mut rows, &mut y),
        }
        let footer = vec![
            Cell {
                item: Item::ResetTab,
                rect: Rect::from_min_size(pos2(SETTING_X, y + 8.0), vec2(140.0, ROW_H - 16.0)),
                art: CellArt::None,
            },
            Cell {
                item: Item::ResetAll,
                rect: Rect::from_min_size(
                    pos2(SETTING_X + 152.0, y + 8.0),
                    vec2(140.0, ROW_H - 16.0),
                ),
                art: CellArt::None,
            },
        ];
        rows.push(Row {
            kind: RowKind::Footer,
            top: y,
            height: ROW_H,
            cells: footer,
        });
        self.rows = rows;
    }

    fn push(rows: &mut Vec<Row>, y: &mut f32, kind: RowKind, height: f32, cells: Vec<Cell>) {
        rows.push(Row {
            kind,
            top: *y,
            height,
            cells,
        });
        *y += height;
    }

    fn binding_rows(&self, device: DeviceKind, rows: &mut Vec<Row>, y: &mut f32) {
        let unbound = rebind::unbound_actions(&self.base, &self.overrides, device);
        if !unbound.is_empty() {
            Self::push(
                rows,
                y,
                RowKind::Banner(unbound.len()),
                BANNER_H,
                Vec::new(),
            );
        }
        Self::push(rows, y, RowKind::Header, HEADER_H, Vec::new());
        let mut swappable = Vec::new();
        for group in rebind::action_groups(&self.base) {
            Self::push(
                rows,
                y,
                RowKind::Group(group.name.clone()),
                GROUP_H,
                Vec::new(),
            );
            for row in group.rows {
                let top = *y;
                let slots = rebind::display_slots(&self.base, &self.overrides, &row.path, device);
                let mut cells = Vec::new();
                for (si, slot) in slots.iter().take(2).enumerate() {
                    let slot_rect = Rect::from_min_size(
                        pos2(SLOT_X[si], top + CELL_PAD),
                        vec2(SLOT_W, ROW_H - 2.0 * CELL_PAD),
                    );
                    match slot {
                        Slot::Composite(parts) => {
                            let w = (SLOT_W - 3.0 * 4.0) / 4.0;
                            // Shown like a keyboard row: Up, Left, Down, Right.
                            for (n, part) in [0usize, 2, 1, 3].into_iter().enumerate() {
                                let rect = Rect::from_min_size(
                                    pos2(slot_rect.left() + n as f32 * (w + 4.0), slot_rect.top()),
                                    vec2(w, slot_rect.height()),
                                );
                                cells.push(Cell {
                                    item: Item::Slot(SlotRef {
                                        path: row.path.clone(),
                                        device,
                                        slot: si,
                                        part: Some(part),
                                    }),
                                    rect,
                                    art: CellArt::Part(
                                        parts[part].map(|c| control_glyph(c, self.style)),
                                        part,
                                    ),
                                });
                            }
                        }
                        Slot::Single(_) => cells.push(Cell {
                            item: Item::Slot(SlotRef {
                                path: row.path.clone(),
                                device,
                                slot: si,
                                part: None,
                            }),
                            rect: slot_rect,
                            art: CellArt::Glyph(slot_glyph(slot, device, self.style)),
                        }),
                    }
                }
                if slots
                    .iter()
                    .filter(|s| matches!(s, Slot::Composite(_)))
                    .count()
                    >= 2
                {
                    swappable.push((row.path.clone(), row.name.clone()));
                }
                let (_, action) = self.base.action(&row.path).expect("row of a known action");
                let toggle = self.overrides.toggles(&row.path, action);
                if row.can_toggle {
                    cells.push(Cell {
                        item: Item::Hold(row.path.clone()),
                        rect: Rect::from_min_size(
                            pos2(HOLD_X, top + CELL_PAD + 4.0),
                            vec2(HOLD_W, ROW_H - 2.0 * CELL_PAD - 8.0),
                        ),
                        art: CellArt::None,
                    });
                }
                cells.push(Cell {
                    item: Item::ResetRow(row.path.clone()),
                    rect: Rect::from_min_size(
                        pos2(RESET_X, top + (ROW_H - RESET_W * 0.8) / 2.0),
                        vec2(RESET_W, RESET_W * 0.8),
                    ),
                    art: CellArt::None,
                });
                Self::push(
                    rows,
                    y,
                    RowKind::Action {
                        unbound: unbound.contains(&row.path),
                        path: row.path,
                        name: row.name,
                        enabled: row.enabled,
                        toggle,
                    },
                    ROW_H,
                    cells,
                );
            }
        }
        match device {
            DeviceKind::KeyboardMouse => {
                Self::push(rows, y, RowKind::Section("Mouse"), SECTION_H, Vec::new());
                self.stick_rows(Stick::Mouse, rows, y);
                for (path, name) in swappable {
                    let cell = setting_cell(Item::SwapArrows(path), *y, 140.0);
                    let label: &'static str = if name == "Move" {
                        "Movement keys"
                    } else {
                        "Direction keys"
                    };
                    Self::push(
                        rows,
                        y,
                        RowKind::Setting(label, "Swap WASD and the arrow keys"),
                        ROW_H,
                        vec![cell],
                    );
                }
            }
            DeviceKind::Gamepad => {
                Self::push(
                    rows,
                    y,
                    RowKind::Section("Left stick"),
                    SECTION_H,
                    Vec::new(),
                );
                self.stick_rows(Stick::Left, rows, y);
                Self::push(
                    rows,
                    y,
                    RowKind::Section("Right stick"),
                    SECTION_H,
                    Vec::new(),
                );
                self.stick_rows(Stick::Right, rows, y);
                Self::push(
                    rows,
                    y,
                    RowKind::Section("Button glyphs"),
                    SECTION_H,
                    Vec::new(),
                );
                let cell = setting_cell(Item::GlyphStyle, *y, SETTING_W);
                Self::push(
                    rows,
                    y,
                    RowKind::Setting("Face buttons", "Letters, or positions for any pad"),
                    ROW_H,
                    vec![cell],
                );
            }
            DeviceKind::Touch => {}
        }
    }

    fn stick_rows(&self, stick: Stick, rows: &mut Vec<Row>, y: &mut f32) {
        let cell = setting_cell(Item::Slider(SliderId::Sensitivity(stick)), *y, SETTING_W);
        Self::push(
            rows,
            y,
            RowKind::Setting("Sensitivity", "25% to 200%"),
            ROW_H,
            vec![cell],
        );
        if stick != Stick::Mouse {
            let cell = setting_cell(Item::Slider(SliderId::Deadzone(stick)), *y, SETTING_W);
            Self::push(
                rows,
                y,
                RowKind::Setting("Dead zone", "Ignores small movements near the centre"),
                ROW_H,
                vec![cell],
            );
        }
        let cell = setting_cell(Item::Switch(SwitchId::InvertX(stick)), *y, 64.0);
        Self::push(
            rows,
            y,
            RowKind::Setting("Invert X", "Left and right swap"),
            ROW_H,
            vec![cell],
        );
        let cell = setting_cell(Item::Switch(SwitchId::InvertY(stick)), *y, 64.0);
        Self::push(
            rows,
            y,
            RowKind::Setting("Invert Y", "Up and down swap"),
            ROW_H,
            vec![cell],
        );
    }

    fn assist_rows(&self, rows: &mut Vec<Row>, y: &mut f32) {
        Self::push(
            rows,
            y,
            RowKind::Section("Every game"),
            SECTION_H,
            Vec::new(),
        );
        let cell = setting_cell(Item::Switch(SwitchId::ToggleAll), *y, 64.0);
        Self::push(
            rows,
            y,
            RowKind::Setting(
                "Toggle every hold action",
                "Press once to hold, again to let go",
            ),
            ROW_H,
            vec![cell],
        );
        let cell = setting_cell(Item::Switch(SwitchId::ReplaceHolds), *y, 64.0);
        Self::push(
            rows,
            y,
            RowKind::Setting(
                "Replace holds with presses",
                "Actions that need a long hold fire at once",
            ),
            ROW_H,
            vec![cell],
        );
        let cell = setting_cell(Item::Slider(SliderId::HoldScale), *y, SETTING_W);
        Self::push(
            rows,
            y,
            RowKind::Setting("Hold time", "How long a hold takes: 0.5x to 3x"),
            ROW_H,
            vec![cell],
        );
        let mut first = true;
        for group in rebind::action_groups(&self.base) {
            for row in group.rows.into_iter().filter(|r| r.can_toggle) {
                if first {
                    Self::push(
                        rows,
                        y,
                        RowKind::Section("Per action"),
                        SECTION_H,
                        Vec::new(),
                    );
                    first = false;
                }
                let (_, action) = self.base.action(&row.path).expect("row of a known action");
                let toggle = self.overrides.toggles(&row.path, action);
                let top = *y;
                let cells = vec![Cell {
                    item: Item::Hold(row.path.clone()),
                    rect: Rect::from_min_size(
                        pos2(HOLD_X, top + CELL_PAD + 4.0),
                        vec2(HOLD_W, ROW_H - 2.0 * CELL_PAD - 8.0),
                    ),
                    art: CellArt::None,
                }];
                Self::push(
                    rows,
                    y,
                    RowKind::Action {
                        path: row.path,
                        name: row.name,
                        enabled: row.enabled,
                        unbound: false,
                        toggle,
                    },
                    ROW_H,
                    cells,
                );
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Values for the painter

    /// A slider's value, its range and its text ("100%").
    pub fn slider(&self, id: SliderId) -> (f32, (f32, f32), String) {
        match id {
            SliderId::Sensitivity(s) => {
                let v = self.overrides.stick(s).sensitivity;
                (v, SENSITIVITY_RANGE, format!("{:.0}%", v * 100.0))
            }
            SliderId::Deadzone(s) => {
                let v = self.deadzone(s);
                (v, DEADZONE_RANGE, format!("{:.2}", v))
            }
            SliderId::HoldScale => {
                let v = self.overrides.assist.hold_time_scale;
                (v, HOLD_SCALE_RANGE, format!("{:.2}x", v))
            }
        }
    }

    fn deadzone(&self, s: Stick) -> f32 {
        self.overrides
            .stick(s)
            .deadzone
            .unwrap_or_else(|| s.control().default_deadzone())
    }

    /// A switch's state.
    pub fn switch(&self, id: SwitchId) -> bool {
        match id {
            SwitchId::InvertX(s) => self.overrides.stick(s).invert_x,
            SwitchId::InvertY(s) => self.overrides.stick(s).invert_y,
            SwitchId::ToggleAll => self.overrides.assist.toggle_all,
            SwitchId::ReplaceHolds => self.overrides.assist.replace_holds,
        }
    }

    /// The glyph style setting.
    pub fn glyph_pref(&self) -> GamepadGlyphPref {
        self.overrides.gamepad_glyphs
    }

    /// True when the row toggles.
    pub fn toggles(&self, path: &str) -> bool {
        self.base
            .action(path)
            .is_some_and(|(_, a)| self.overrides.toggles(path, a))
    }

    // -----------------------------------------------------------------------------------------
    // Input

    /// Advances the clock: listening times out, toasts and notices expire.
    pub fn tick(&mut self, now: f64) {
        self.clock = now;
        if let Mode::Listening { since, .. } = self.mode
            && now - since >= LISTEN_TIMEOUT
        {
            self.mode = Mode::Browse;
            self.note("No control was pressed: nothing changed");
        }
        if self.toast.as_ref().is_some_and(|t| now >= t.until) {
            self.toast = None;
        }
        if self.notice.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.notice = None;
        }
    }

    fn note(&mut self, text: &str) {
        self.notice = Some((text.to_string(), self.clock + NOTICE_SECONDS));
    }

    /// True while a slot waits for a control.
    pub fn is_listening(&self) -> bool {
        matches!(self.mode, Mode::Listening { .. })
    }

    /// A navigation command.
    pub fn nav(&mut self, nav: Nav) -> Outcome {
        match self.mode.clone() {
            Mode::Browse => self.browse(nav),
            Mode::Listening { .. } => {
                // Listening reads raw controls (`listen`); only Back arrives here.
                if nav == Nav::Back {
                    self.mode = Mode::Browse;
                }
                Outcome::default()
            }
            Mode::Conflict { choice, .. } => self.conflict_nav(nav, choice),
            Mode::Confirm { reset, yes } => self.confirm_nav(nav, reset, yes),
        }
    }

    fn browse(&mut self, nav: Nav) -> Outcome {
        let mut out = Outcome::default();
        match nav {
            Nav::Up => self.move_vertical(-1),
            Nav::Down => self.move_vertical(1),
            Nav::Left | Nav::Right => {
                let dir = if nav == Nav::Left { -1.0 } else { 1.0 };
                match self.focused_item().cloned() {
                    Some(Item::Slider(id)) => out.changed = self.step_slider(id, dir),
                    Some(Item::GlyphStyle) => out.changed = self.cycle_style(dir as i32),
                    _ => self.move_horizontal(if dir < 0.0 { -1 } else { 1 }),
                }
            }
            Nav::Accept => {
                if let Some(item) = self.focused_item().cloned() {
                    out = self.activate(item);
                }
            }
            Nav::Back => out.back = true,
            Nav::NextTab => self.switch_tab(1),
            Nav::PrevTab => self.switch_tab(-1),
            Nav::Clear => {
                if let Some(Item::Slot(target)) = self.focused_item().cloned() {
                    out.changed = self.clear_slot(&target);
                }
            }
            Nav::Reset => {
                if self.toast.is_some() {
                    out.changed = self.undo();
                } else if let Some(path) = self.focused_row_path() {
                    if let Some(device) = self.tab.device() {
                        let before = self.overrides.clone();
                        rebind::reset_action(&mut self.overrides, &path, device);
                        out.changed = before != self.overrides;
                    } else if let Some(a) = self.overrides.actions.get_mut(&path) {
                        out.changed = a.toggle.take().is_some();
                        self.overrides.prune();
                    }
                    if out.changed {
                        self.rebuild_keep_focus();
                    }
                }
            }
        }
        self.ensure_visible();
        out
    }

    fn focused_row_path(&self) -> Option<String> {
        let (r, _) = self.focus?;
        match &self.rows.get(r)?.kind {
            RowKind::Action { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    fn activate(&mut self, item: Item) -> Outcome {
        let mut out = Outcome::default();
        match item {
            Item::Slot(target) => match rebind::slot_kind(&self.base, &self.overrides, &target) {
                Some(SlotKind::Fixed) => {
                    self.note("Mouse movement is set with the Mouse settings below");
                }
                Some(_) => {
                    self.mode = Mode::Listening {
                        target,
                        since: self.clock,
                        modifier: None,
                    };
                }
                None => {}
            },
            Item::Hold(path) => {
                if let Some((_, action)) = self.base.action(&path) {
                    let now = self.overrides.toggles(&path, action);
                    self.overrides.action_mut(&path).toggle = Some(!now);
                    // Back to "follow the global switch" when that gives the same answer.
                    let follows = self.overrides.assist.toggle_all
                        && !matches!(action.role, Role::Jump | Role::Menu);
                    if follows == !now {
                        self.overrides.action_mut(&path).toggle = None;
                    }
                    self.overrides.prune();
                    out.changed = true;
                }
            }
            Item::ResetRow(path) => {
                if let Some(device) = self.tab.device() {
                    let before = self.overrides.clone();
                    rebind::reset_action(&mut self.overrides, &path, device);
                    out.changed = before != self.overrides;
                }
            }
            Item::SwapArrows(path) => {
                out.changed = rebind::swap_composites(&self.base, &mut self.overrides, &path);
            }
            Item::Slider(id) => out.changed = self.step_slider(id, 1.0),
            Item::Switch(id) => {
                let on = !self.switch(id);
                match id {
                    SwitchId::InvertX(s) => self.edit_stick(s, |st| st.invert_x = on),
                    SwitchId::InvertY(s) => self.edit_stick(s, |st| st.invert_y = on),
                    SwitchId::ToggleAll => self.overrides.assist.toggle_all = on,
                    SwitchId::ReplaceHolds => self.overrides.assist.replace_holds = on,
                }
                out.changed = true;
            }
            Item::GlyphStyle => out.changed = self.cycle_style(1),
            Item::ResetTab => {
                self.mode = Mode::Confirm {
                    reset: ResetKind::Tab,
                    yes: false,
                }
            }
            Item::ResetAll => {
                self.mode = Mode::Confirm {
                    reset: ResetKind::All,
                    yes: false,
                }
            }
        }
        if out.changed {
            self.rebuild_keep_focus();
        }
        out
    }

    fn edit_stick(&mut self, s: Stick, f: impl FnOnce(&mut StickSettings)) {
        let mut st = self.overrides.stick(s);
        f(&mut st);
        self.overrides.set_stick(s, st);
    }

    fn step_slider(&mut self, id: SliderId, dir: f32) -> bool {
        let (v, (lo, hi), _) = self.slider(id);
        let step = match id {
            SliderId::Sensitivity(_) => 0.05,
            SliderId::Deadzone(_) => 0.02,
            SliderId::HoldScale => 0.25,
        };
        let next = ((v + dir * step) / step).round() * step;
        let next = next.clamp(lo, hi);
        if (next - v).abs() < 1e-6 {
            return false;
        }
        self.set_slider(id, next);
        true
    }

    /// Sets a slider (the mouse drags it).
    pub fn set_slider(&mut self, id: SliderId, value: f32) -> Outcome {
        match id {
            SliderId::Sensitivity(s) => self.edit_stick(s, |st| st.sensitivity = value),
            SliderId::Deadzone(s) => {
                let default = s.control().default_deadzone();
                self.edit_stick(s, |st| {
                    st.deadzone = ((value - default).abs() > 1e-4).then_some(value);
                });
            }
            SliderId::HoldScale => {
                self.overrides.assist.hold_time_scale =
                    value.clamp(HOLD_SCALE_RANGE.0, HOLD_SCALE_RANGE.1);
            }
        }
        Outcome {
            changed: true,
            back: false,
        }
    }

    fn cycle_style(&mut self, dir: i32) -> bool {
        const ORDER: [GamepadGlyphPref; 3] = [
            GamepadGlyphPref::Auto,
            GamepadGlyphPref::Letters,
            GamepadGlyphPref::Positions,
        ];
        let i = ORDER
            .iter()
            .position(|p| *p == self.overrides.gamepad_glyphs)
            .unwrap_or(0) as i32;
        self.overrides.gamepad_glyphs = ORDER[(i + dir).rem_euclid(3) as usize];
        true
    }

    fn clear_slot(&mut self, target: &SlotRef) -> bool {
        match rebind::clear(&self.base, &mut self.overrides, target) {
            Ok(()) => {
                self.rebuild_keep_focus();
                true
            }
            Err(RebindError::Required(name)) => {
                self.note(&format!("{name} needs at least one binding on this device"));
                false
            }
            Err(RebindError::Fixed) => {
                self.note("Mouse movement is set with the Mouse settings below");
                false
            }
            Err(_) => false,
        }
    }

    fn switch_tab(&mut self, dir: i32) {
        let i = (self.tab.index() as i32 + dir).rem_euclid(Tab::ALL.len() as i32) as usize;
        self.set_tab(Tab::ALL[i]);
    }

    /// Shows a tab (the mouse clicks one).
    pub fn set_tab(&mut self, tab: Tab) {
        if tab == self.tab {
            return;
        }
        self.tab_focus[self.tab.index()] = self.focused_item().cloned();
        self.tab = tab;
        self.rebuild();
        self.focus = self.tab_focus[tab.index()]
            .clone()
            .and_then(|i| self.find(&i));
        self.clamp_focus();
        self.scroll = 0.0;
        self.ensure_visible();
    }

    fn move_vertical(&mut self, dir: i32) {
        let Some((r, c)) = self.focus else {
            self.focus_first();
            return;
        };
        let x = self.rows[r].cells[c].rect.center().x;
        let mut i = r as i32 + dir;
        while i >= 0 && (i as usize) < self.rows.len() {
            let row = &self.rows[i as usize];
            if !row.cells.is_empty() {
                let best = row
                    .cells
                    .iter()
                    .enumerate()
                    .min_by(|a, b| {
                        let da = (a.1.rect.center().x - x).abs();
                        let db = (b.1.rect.center().x - x).abs();
                        da.total_cmp(&db)
                    })
                    .map_or(0, |(j, _)| j);
                self.focus = Some((i as usize, best));
                return;
            }
            i += dir;
        }
        // At the top: show the banner and header above the first row.
        if dir < 0 {
            self.scroll = 0.0;
        }
    }

    fn move_horizontal(&mut self, dir: i32) {
        if let Some((r, c)) = self.focus {
            let n = self.rows[r].cells.len() as i32;
            let next = (c as i32 + dir).clamp(0, n - 1);
            self.focus = Some((r, next as usize));
        }
    }

    fn ensure_visible(&mut self) {
        let Some((r, _)) = self.focus else { return };
        let row = &self.rows[r];
        let margin = 12.0;
        let top = row.top - margin;
        let bottom = row.top + row.height + margin;
        if top < self.scroll {
            self.scroll = top.max(0.0);
        } else if bottom > self.scroll + self.viewport_h {
            self.scroll = bottom - self.viewport_h;
        }
        if r <= 2 {
            // Keep the banner and the column titles in view near the top.
            self.scroll = self.scroll.min(row.top.max(0.0) - 0.0).max(0.0);
        }
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let max = (self.content_height() - self.viewport_h).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
    }

    /// Scrolls by `dy` points (the mouse wheel).
    pub fn scroll_by(&mut self, dy: f32) {
        self.scroll += dy;
        self.clamp_scroll();
    }

    fn content_pos(&self, screen: egui::Pos2) -> egui::Pos2 {
        pos2(
            screen.x - self.origin.x,
            screen.y - self.origin.y + self.scroll,
        )
    }

    fn hit(&self, screen: egui::Pos2) -> Option<(usize, usize)> {
        let p = self.content_pos(screen);
        if screen.y < self.origin.y || screen.y > self.origin.y + self.viewport_h {
            return None;
        }
        self.rows.iter().enumerate().find_map(|(r, row)| {
            row.cells
                .iter()
                .position(|c| c.rect.expand(2.0).contains(p))
                .map(|c| (r, c))
        })
    }

    /// The mouse moved: the cell under it takes the focus (only while browsing).
    pub fn hover(&mut self, screen: egui::Pos2) {
        if self.mode == Mode::Browse
            && let Some(hit) = self.hit(screen)
        {
            self.focus = Some(hit);
        }
    }

    /// A left click at `screen`. While listening, a click outside Cancel binds the left mouse
    /// button (keyboard tab) through [`listen`](Self::listen) instead: the caller asks
    /// [`click_cancels`](Self::click_cancels) first.
    pub fn click(&mut self, screen: egui::Pos2) -> Outcome {
        if let Some(t) = self
            .tab_rects
            .iter()
            .find(|(r, _)| r.contains(screen))
            .map(|t| t.1)
        {
            if self.mode == Mode::Browse {
                self.set_tab(t);
            }
            return Outcome::default();
        }
        if let Some(r) = self.undo_rect
            && r.contains(screen)
            && self.toast.is_some()
        {
            return Outcome {
                changed: self.undo(),
                back: false,
            };
        }
        match self.mode.clone() {
            Mode::Browse => {
                let Some(hit) = self.hit(screen) else {
                    return Outcome::default();
                };
                self.focus = Some(hit);
                let cell = &self.rows[hit.0].cells[hit.1];
                if let Item::Slider(id) = cell.item {
                    let (_, (lo, hi), _) = self.slider(id);
                    let p = self.content_pos(screen);
                    let t = ((p.x - cell.rect.left()) / cell.rect.width()).clamp(0.0, 1.0);
                    let out = self.set_slider(id, lo + t * (hi - lo));
                    self.rebuild_keep_focus();
                    return out;
                }
                self.nav(Nav::Accept)
            }
            Mode::Conflict { .. } | Mode::Confirm { .. } => {
                let choice = self
                    .dialog_rects
                    .iter()
                    .find(|(r, _)| r.contains(screen))
                    .map(|c| c.1);
                match (choice, self.mode.clone()) {
                    (Some(c), Mode::Conflict { .. }) => self.resolve(c),
                    (Some(c), Mode::Confirm { reset, .. }) => {
                        if c == Choice::Swap {
                            self.do_reset(reset)
                        } else {
                            self.mode = Mode::Browse;
                            Outcome::default()
                        }
                    }
                    _ => Outcome::default(),
                }
            }
            Mode::Listening { .. } => {
                if self.click_cancels(screen) {
                    self.mode = Mode::Browse;
                }
                Outcome::default()
            }
        }
    }

    /// True when a click at `screen` hits the listening dialog's Cancel button.
    pub fn click_cancels(&self, screen: egui::Pos2) -> bool {
        self.cancel_rect.is_some_and(|r| r.contains(screen))
    }

    /// A raw control went down (`pressed`) or up while listening (spec 8.2). Returns what
    /// changed; controls the slot does not take are ignored with a notice.
    pub fn listen(&mut self, control: Control, pressed: bool) -> Outcome {
        let Mode::Listening {
            target, modifier, ..
        } = self.mode.clone()
        else {
            return Outcome::default();
        };
        let is_menu = target.path == metallic_runtime::input::MENU_ACTION;
        if !pressed {
            // A modifier that went up without another key binds by itself.
            if modifier == Some(control) {
                return self.bind(target, control);
            }
            return Outcome::default();
        }
        let keyboard_tab = target.device == DeviceKind::KeyboardMouse;
        match control {
            Control::Escape if !(is_menu && keyboard_tab) => {
                self.mode = Mode::Browse;
                return Outcome::default();
            }
            Control::ButtonB if keyboard_tab => {
                self.mode = Mode::Browse;
                return Outcome::default();
            }
            Control::Backspace => {
                self.mode = Mode::Browse;
                let changed = self.clear_slot(&target);
                return Outcome {
                    changed,
                    back: false,
                };
            }
            _ => {}
        }
        if is_modifier(control) && keyboard_tab {
            if let Mode::Listening { modifier, .. } = &mut self.mode {
                *modifier = Some(control);
            }
            return Outcome::default();
        }
        if !rebind::accepts(&self.base, &self.overrides, &target, control) {
            let msg = match (target.device, control.kind()) {
                (DeviceKind::KeyboardMouse, _) if control.is_gamepad() => {
                    "This tab takes keys and mouse buttons"
                }
                (DeviceKind::Gamepad, _) if !control.is_gamepad() => {
                    "This tab takes gamepad buttons and sticks"
                }
                (_, ControlKind::Stick) => "This slot takes a button",
                _ => "This slot takes a stick",
            };
            self.note(msg);
            return Outcome::default();
        }
        self.bind(target, control)
    }

    fn bind(&mut self, target: SlotRef, control: Control) -> Outcome {
        match rebind::assign(&self.base, &mut self.overrides, &target, control, None) {
            Ok(()) => {
                self.mode = Mode::Browse;
                self.rebuild_keep_focus();
                Outcome {
                    changed: true,
                    back: false,
                }
            }
            Err(RebindError::Conflict(conflict)) => {
                if conflict.fixed {
                    self.mode = Mode::Browse;
                    self.note(&format!(
                        "{} is part of {}: choose another control",
                        control_glyph(control, self.style).text,
                        conflict.other_name
                    ));
                    return Outcome::default();
                }
                let choice = if conflict.can_swap {
                    Choice::Swap
                } else if conflict.can_replace {
                    Choice::Replace
                } else {
                    Choice::Cancel
                };
                self.mode = Mode::Conflict {
                    target,
                    control,
                    conflict,
                    choice,
                };
                Outcome::default()
            }
            Err(_) => {
                self.mode = Mode::Browse;
                Outcome::default()
            }
        }
    }

    /// The buttons the conflict dialog offers, in order.
    pub fn conflict_choices(&self) -> Vec<Choice> {
        let Mode::Conflict { conflict, .. } = &self.mode else {
            return Vec::new();
        };
        let mut v = Vec::with_capacity(3);
        if conflict.can_swap {
            v.push(Choice::Swap);
        }
        if conflict.can_replace {
            v.push(Choice::Replace);
        }
        v.push(Choice::Cancel);
        v
    }

    fn conflict_nav(&mut self, nav: Nav, choice: Choice) -> Outcome {
        let choices = self.conflict_choices();
        let i = choices.iter().position(|c| *c == choice).unwrap_or(0);
        let set = |page: &mut Self, c: Choice| {
            if let Mode::Conflict { choice, .. } = &mut page.mode {
                *choice = c;
            }
        };
        match nav {
            Nav::Left | Nav::Up => set(self, choices[i.saturating_sub(1)]),
            Nav::Right | Nav::Down => set(self, choices[(i + 1).min(choices.len() - 1)]),
            Nav::Accept => return self.resolve(choice),
            Nav::Back => self.mode = Mode::Browse,
            _ => {}
        }
        Outcome::default()
    }

    /// Settles the conflict dialog.
    pub fn resolve(&mut self, choice: Choice) -> Outcome {
        let Mode::Conflict {
            target, control, ..
        } = self.mode.clone()
        else {
            return Outcome::default();
        };
        self.mode = Mode::Browse;
        let resolution = match choice {
            Choice::Swap => Resolution::Swap,
            Choice::Replace => Resolution::Replace,
            Choice::Cancel => return Outcome::default(),
        };
        match rebind::assign(
            &self.base,
            &mut self.overrides,
            &target,
            control,
            Some(resolution),
        ) {
            Ok(()) => {
                self.rebuild_keep_focus();
                Outcome {
                    changed: true,
                    back: false,
                }
            }
            Err(_) => {
                self.note("That change is not possible");
                Outcome::default()
            }
        }
    }

    fn confirm_nav(&mut self, nav: Nav, reset: ResetKind, yes: bool) -> Outcome {
        match nav {
            Nav::Left | Nav::Right | Nav::Up | Nav::Down => {
                self.mode = Mode::Confirm { reset, yes: !yes };
            }
            Nav::Accept if yes => return self.do_reset(reset),
            Nav::Accept | Nav::Back => self.mode = Mode::Browse,
            _ => {}
        }
        Outcome::default()
    }

    fn do_reset(&mut self, reset: ResetKind) -> Outcome {
        self.mode = Mode::Browse;
        let before = self.overrides.clone();
        let text = match reset {
            ResetKind::Tab => {
                match self.tab.device() {
                    Some(d) => rebind::reset_device(&mut self.overrides, d),
                    None => {
                        self.overrides.assist = Default::default();
                        for a in self.overrides.actions.values_mut() {
                            a.toggle = None;
                        }
                        self.overrides.prune();
                    }
                }
                format!("{} reset", self.tab.title())
            }
            ResetKind::All => {
                self.overrides.reset_known();
                "All controls reset".to_string()
            }
        };
        let changed = before != self.overrides;
        self.toast = Some(Toast {
            text,
            until: self.clock + UNDO_SECONDS,
            undo: before,
        });
        self.rebuild_keep_focus();
        Outcome {
            changed,
            back: false,
        }
    }

    fn undo(&mut self) -> bool {
        let Some(t) = self.toast.take() else {
            return false;
        };
        self.overrides = t.undo;
        self.rebuild_keep_focus();
        self.note("Reset undone");
        true
    }
}

fn setting_cell(item: Item, top: f32, width: f32) -> Cell {
    Cell {
        item,
        rect: Rect::from_min_size(
            pos2(SETTING_X, top + CELL_PAD + 2.0),
            vec2(width, ROW_H - 2.0 * CELL_PAD - 4.0),
        ),
        art: CellArt::None,
    }
}

/// Modifier keys: alone they bind on release (spec 8.2).
fn is_modifier(c: Control) -> bool {
    matches!(
        c,
        Control::LeftShift
            | Control::RightShift
            | Control::LeftControl
            | Control::RightControl
            | Control::LeftAlt
            | Control::RightAlt
    )
}

#[cfg(test)]
#[path = "controls_tests.rs"]
mod tests;
