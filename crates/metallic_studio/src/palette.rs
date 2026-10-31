//! The command palette (Ctrl+K) and Insert at cursor (Shift+A), research doc section 5.4.
//!
//! **Ctrl+K** opens a 640 px palette under the title bar. One query searches, in groups:
//!
//! 1. **In this level**: instances whose name or class matches, one row per name and class with a
//!    count ("Coin · Part · 7 objects"); Enter selects them all.
//! 2. **Assets**: files in the game folder, with type and size; Enter copies the path.
//! 3. **Commands**: every registered action (and each Insert class) with its shortcut. With an
//!    empty query, recently run commands come first.
//! 4. **Learn**: guides supplied with [`CommandPalette::set_guides`] (none are bundled yet).
//! 5. **Ask Litha about "…"** (Ctrl+Enter), always the last row.
//!
//! Prefixes narrow the search: `>` commands only, `@` objects, `#` assets. Up and Down move,
//! Tab jumps to the next group, Enter runs, Esc or a click outside closes. Ctrl+Shift+P opens it
//! with `>` typed.
//!
//! **Shift+A** over the viewport opens the Insert palette at the cursor: a short searchable list of
//! classes; the new item lands where the cursor pointed when the palette opened.
//!
//! The palette only finds and chooses; [`CommandPalette::show`] returns a [`Choice`] and the
//! editor carries it out, so every change still goes through actions and undo. Results are
//! computed when the query changes, not every frame.
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2};
use metallic_core::{InstanceId, Value, Vec3, World};

use crate::actions::ActionRegistry;
use crate::editor::{EditorState, INSERTABLE};
use crate::keymap::Keymap;
use crate::theme::{self, fonts};
use crate::widgets;

/// Palette width (research doc 5.4).
pub const PALETTE_WIDTH: f32 = 640.0;
/// Insert palette width.
pub const INSERT_WIDTH: f32 = 260.0;
/// Distance from the top of the window (under the title bar).
pub const PALETTE_TOP: f32 = 44.0;
/// Most recently run commands remembered.
pub const RECENT_COMMANDS: usize = 8;
/// Most files the asset scan lists.
pub const MAX_ASSETS: usize = 5000;

const ROW: f32 = 28.0;
const HEADER: f32 = 22.0;
const MAX_LIST_HEIGHT: f32 = 420.0;

/// What the palette was opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    /// Ctrl+K: objects, assets, commands, guides, Litha.
    Search,
    /// Shift+A: classes to insert at the cursor.
    Insert,
}

/// A guide or docs page the Learn group can offer.
#[derive(Debug, Clone, PartialEq)]
pub struct LearnEntry {
    /// Title ("Make a coin pickup").
    pub title: String,
    /// Kind and length ("Guide · 4 min").
    pub detail: String,
    /// Where it opens.
    pub url: String,
}

/// What the creator picked; the editor carries it out.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    /// Select these instances.
    Select(Vec<InstanceId>),
    /// Run this action.
    Run(&'static str),
    /// Insert `class` under `parent` (Workspace when `None`) at `at`.
    Insert {
        /// Class name.
        class: &'static str,
        /// Parent, if a container is active.
        parent: Option<InstanceId>,
        /// Where a part lands.
        at: Vec3,
    },
    /// An asset file, by its path relative to the game folder.
    Asset(String),
    /// Open a guide.
    Learn(String),
    /// Ask Litha this question.
    AskLitha(String),
}

/// The result groups, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// Instances in the edit world.
    Objects,
    /// Files in the game folder.
    Assets,
    /// Actions and Insert commands.
    Commands,
    /// Guides.
    Learn,
    /// Classes (Insert palette).
    Insert,
    /// The Ask Litha row.
    Litha,
}

impl Group {
    /// The group heading.
    pub fn title(self) -> &'static str {
        match self {
            Group::Objects => "In this level",
            Group::Assets => "Assets",
            Group::Commands => "Commands",
            Group::Learn => "Learn",
            Group::Insert => "Insert",
            Group::Litha => "Litha",
        }
    }
}

/// One result row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Its group.
    pub group: Group,
    /// Main text.
    pub title: String,
    /// Dim text after the title (path, type and size, count).
    pub detail: String,
    /// Shortcut shown as a keycap on the right.
    pub shortcut: String,
    /// False for a command that cannot run now (shown dim).
    pub enabled: bool,
    /// Class for the row icon, if any.
    pub class: Option<&'static str>,
    /// What Enter does.
    pub choice: Choice,
}

#[derive(Debug, Clone)]
struct AssetFile {
    rel: String,
    kind: &'static str,
    size: u64,
}

/// The palette's state. One per editor window.
#[derive(Debug, Default)]
pub struct CommandPalette {
    mode: Option<PaletteMode>,
    query: String,
    /// The query and mode the rows were computed for.
    computed: Option<(PaletteMode, String)>,
    rows: Vec<Row>,
    selected: usize,
    recent: VecDeque<&'static str>,
    assets: Vec<AssetFile>,
    guides: Vec<LearnEntry>,
    /// Insert palette: screen position, world point and parent taken when it opened.
    insert_at: Option<(Pos2, Vec3, Option<InstanceId>)>,
    focus: bool,
    rect: Option<Rect>,
    scroll_to_selected: bool,
}

impl CommandPalette {
    /// A closed palette.
    pub fn new() -> CommandPalette {
        CommandPalette::default()
    }

    /// True while the palette (either mode) is open.
    pub fn is_open(&self) -> bool {
        self.mode.is_some()
    }

    /// The open mode.
    pub fn mode(&self) -> Option<PaletteMode> {
        self.mode
    }

    /// The query text.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Sets the query (as typing does).
    pub fn set_query(&mut self, text: &str) {
        text.clone_into(&mut self.query);
    }

    /// The rows for the current query, after [`CommandPalette::refresh`] or a drawn frame.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The highlighted row.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The guides the Learn group searches.
    pub fn set_guides(&mut self, guides: Vec<LearnEntry>) {
        self.guides = guides;
        self.computed = None;
    }

    /// Commands run from the palette, most recent first.
    pub fn recent(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.recent.iter().copied()
    }

    /// Opens the search palette with `text` already typed ("" or ">"), listing the files of
    /// `project_dir` for the Assets group.
    pub fn open(&mut self, text: &str, project_dir: &Path) {
        self.mode = Some(PaletteMode::Search);
        self.set_query(text);
        self.assets = scan_assets(project_dir);
        self.reset();
    }

    /// Opens the Insert palette at screen point `pos`; a part lands at `at` under `parent`.
    pub fn open_insert(&mut self, pos: Pos2, at: Vec3, parent: Option<InstanceId>) {
        self.mode = Some(PaletteMode::Insert);
        self.query.clear();
        self.insert_at = Some((pos, at, parent));
        self.reset();
    }

    fn reset(&mut self) {
        self.computed = None;
        self.selected = 0;
        self.focus = true;
        self.rect = None;
    }

    /// Closes the palette.
    pub fn close(&mut self) {
        self.mode = None;
        self.insert_at = None;
        self.rows.clear();
        self.computed = None;
    }

    /// Notes that command `id` ran (the empty query lists recent commands first).
    pub fn remember(&mut self, id: &'static str) {
        self.recent.retain(|r| *r != id);
        self.recent.push_front(id);
        self.recent.truncate(RECENT_COMMANDS);
    }

    /// Recomputes the rows if the query or mode changed since the last time.
    pub fn refresh(&mut self, state: &EditorState, actions: &ActionRegistry, keymap: &Keymap) {
        let Some(mode) = self.mode else { return };
        if self
            .computed
            .as_ref()
            .is_some_and(|(m, q)| *m == mode && *q == self.query)
        {
            return;
        }
        self.rows.clear();
        match mode {
            PaletteMode::Search => self.search(state, actions, keymap),
            PaletteMode::Insert => self.search_insert(),
        }
        self.selected = self
            .rows
            .iter()
            .position(|r| r.enabled)
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
        self.computed = Some((mode, self.query.clone()));
    }

    fn search(&mut self, state: &EditorState, actions: &ActionRegistry, keymap: &Keymap) {
        let query = self.query.clone();
        let raw = query.trim();
        let (only, q) = match raw.chars().next() {
            Some('>') => (Some(Group::Commands), raw[1..].trim()),
            Some('@') => (Some(Group::Objects), raw[1..].trim()),
            Some('#') => (Some(Group::Assets), raw[1..].trim()),
            _ => (None, raw),
        };
        let wants = |g: Group| only.is_none_or(|o| o == g);
        let many = only.is_some();
        if wants(Group::Objects) && !q.is_empty() {
            object_rows(&state.world, q, if many { 50 } else { 6 }, &mut self.rows);
        }
        if wants(Group::Assets) && !q.is_empty() {
            let mut hits: Vec<(i32, &AssetFile)> = self
                .assets
                .iter()
                .filter_map(|a| fuzzy_score(q, &a.rel).map(|s| (s, a)))
                .collect();
            hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.rel.cmp(&b.1.rel)));
            hits.truncate(if many { 50 } else { 5 });
            self.rows.extend(hits.into_iter().map(|(_, a)| Row {
                group: Group::Assets,
                title: a.rel.clone(),
                detail: format!("{} · {}", a.kind, size_text(a.size)),
                shortcut: String::new(),
                enabled: true,
                class: None,
                choice: Choice::Asset(a.rel.clone()),
            }));
        }
        if wants(Group::Commands) {
            self.command_rows(state, actions, keymap, q, many);
        }
        if only.is_none() && !q.is_empty() {
            let mut hits: Vec<(i32, &LearnEntry)> = self
                .guides
                .iter()
                .filter_map(|g| fuzzy_score(q, &g.title).map(|s| (s, g)))
                .collect();
            hits.sort_by_key(|h| std::cmp::Reverse(h.0));
            self.rows.extend(hits.into_iter().take(3).map(|(_, g)| Row {
                group: Group::Learn,
                title: g.title.clone(),
                detail: g.detail.clone(),
                shortcut: String::new(),
                enabled: true,
                class: None,
                choice: Choice::Learn(g.url.clone()),
            }));
        }
        let question = q.to_string();
        self.rows.push(Row {
            group: Group::Litha,
            title: if question.is_empty() {
                "Ask Litha".to_string()
            } else {
                format!("Ask Litha about \"{question}\"")
            },
            detail: String::new(),
            shortcut: "Ctrl+Enter".to_string(),
            enabled: true,
            class: None,
            choice: Choice::AskLitha(question),
        });
    }

    fn command_rows(
        &mut self,
        state: &EditorState,
        actions: &ActionRegistry,
        keymap: &Keymap,
        q: &str,
        many: bool,
    ) {
        let mut hits: Vec<(i32, Row)> = Vec::new();
        for a in actions.iter() {
            if a.id.starts_with("palette.") {
                continue;
            }
            let Some(score) = fuzzy_score(q, &a.label).or_else(|| fuzzy_score(q, a.id)) else {
                continue;
            };
            // Recent commands first for an empty query.
            let recent = self.recent.iter().position(|r| *r == a.id);
            let score = match (q.is_empty(), recent) {
                (true, Some(i)) => 10_000 - i as i32,
                _ => score,
            };
            hits.push((
                score,
                Row {
                    group: Group::Commands,
                    title: a.label.clone(),
                    detail: String::new(),
                    shortcut: keymap.label(a.id),
                    enabled: a.is_enabled(state),
                    class: None,
                    choice: Choice::Run(a.id),
                },
            ));
        }
        if !state.play.is_playing() {
            for &class in INSERTABLE {
                let title = format!("Insert {class}");
                if let Some(score) = fuzzy_score(q, &title) {
                    hits.push((
                        score - 1,
                        Row {
                            group: Group::Commands,
                            title,
                            detail: String::new(),
                            shortcut: String::new(),
                            enabled: true,
                            class: Some(class),
                            choice: Choice::Insert {
                                class,
                                parent: insert_parent(state),
                                at: state.insert_point,
                            },
                        },
                    ));
                }
            }
        }
        // Stable: equal scores keep registration order.
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        let cap = if many || q.is_empty() { 200 } else { 8 };
        self.rows
            .extend(hits.into_iter().take(cap).map(|(_, row)| row));
    }

    fn search_insert(&mut self) {
        let Some((_, at, parent)) = self.insert_at else {
            return;
        };
        let q = self.query.trim();
        let mut hits: Vec<(i32, &'static str)> = INSERTABLE
            .iter()
            .filter_map(|&c| fuzzy_score(q, c).map(|s| (s, c)))
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        self.rows.extend(hits.into_iter().map(|(_, class)| Row {
            group: Group::Insert,
            title: class.to_string(),
            detail: String::new(),
            shortcut: String::new(),
            enabled: true,
            class: Some(class),
            choice: Choice::Insert { class, parent, at },
        }));
    }

    /// Moves the highlight by `step` rows, skipping disabled ones.
    pub fn move_selection(&mut self, step: isize) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let mut i = self.selected;
        for _ in 0..n {
            i = (i as isize + step).rem_euclid(n as isize) as usize;
            if self.rows[i].enabled {
                self.selected = i;
                break;
            }
        }
        self.scroll_to_selected = true;
    }

    /// Moves the highlight to the first row of the next group (Tab).
    pub fn next_group(&mut self) {
        let Some(current) = self.rows.get(self.selected).map(|r| r.group) else {
            return;
        };
        let n = self.rows.len();
        let mut i = self.selected;
        for _ in 0..n {
            i = (i + 1) % n;
            if self.rows[i].group != current && self.rows[i].enabled {
                self.selected = i;
                self.scroll_to_selected = true;
                return;
            }
        }
    }

    /// Picks the highlighted row: returns its choice and closes. A disabled row does nothing.
    pub fn pick(&mut self) -> Option<Choice> {
        let row = self.rows.get(self.selected).filter(|r| r.enabled)?;
        let choice = row.choice.clone();
        if let Choice::Run(id) = choice {
            self.remember(id);
        }
        self.close();
        Some(choice)
    }

    /// Ctrl+Enter: asks Litha about the query, whatever is highlighted.
    pub fn ask_litha(&mut self) -> Choice {
        let q = self.query.trim().trim_start_matches(['>', '@', '#']).trim();
        let choice = Choice::AskLitha(q.to_string());
        self.close();
        choice
    }

    /// Handles the palette's keys and draws it. Returns what the creator picked this frame.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        state: &EditorState,
        actions: &ActionRegistry,
        keymap: &Keymap,
    ) -> Option<Choice> {
        let mode = self.mode?;
        // Keys first, so the text field never sees them.
        let (esc, down, up, tab, ctrl_enter, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::COMMAND, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if esc {
            self.close();
            return None;
        }
        self.refresh(state, actions, keymap);
        if down {
            self.move_selection(1);
        }
        if up {
            self.move_selection(-1);
        }
        if tab {
            self.next_group();
        }
        if ctrl_enter && mode == PaletteMode::Search {
            return Some(self.ask_litha());
        }
        if enter {
            return self.pick();
        }
        // A click outside closes.
        if let Some(rect) = self.rect
            && ctx.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
            })
        {
            self.close();
            return None;
        }

        let screen = ctx.content_rect();
        let (pos, width) = match (mode, self.insert_at) {
            (PaletteMode::Insert, Some((p, _, _))) => {
                let w = INSERT_WIDTH.min(screen.width() - 32.0);
                let x = p.x.clamp(screen.left() + 16.0, screen.right() - w - 16.0);
                let y =
                    p.y.clamp(screen.top() + 16.0, (screen.bottom() - 300.0).max(16.0));
                (Pos2::new(x, y), w)
            }
            _ => {
                let w = PALETTE_WIDTH.min(screen.width() - 32.0);
                (
                    Pos2::new(screen.center().x - w / 2.0, screen.top() + PALETTE_TOP),
                    w,
                )
            }
        };
        let mut picked = None;
        let area = egui::Area::new(egui::Id::unique("command-palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::bg_raised())
                    .stroke(Stroke::new(1.0, theme::line_strong()))
                    .corner_radius(theme::R_LG)
                    .shadow(theme::SHADOW_POP)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width(width - 16.0);
                        picked = self.contents(ui, mode);
                    });
            });
        self.rect = Some(area.response.rect);
        if picked.is_some() {
            return picked;
        }
        None
    }

    fn contents(&mut self, ui: &mut egui::Ui, mode: PaletteMode) -> Option<Choice> {
        let hint = match mode {
            PaletteMode::Search => {
                "Search objects, assets and commands  (> commands, @ objects, # assets)"
            }
            PaletteMode::Insert => "Insert…",
        };
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(egui::Id::unique("command-palette-query"))
                .hint_text(hint)
                .font(fonts::body_lg())
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut self.focus) || !field.has_focus() {
            field.request_focus();
        }
        ui.add_space(6.0);
        let mut picked = None;
        let mut hover = None;
        // The list is as tall as its rows, up to MAX_LIST_HEIGHT (sized from the rows, so a new
        // query never shows a list cut to the last frame's height).
        let headers = if mode == PaletteMode::Search {
            let mut n = 0;
            let mut last = None;
            for r in &self.rows {
                if last != Some(r.group) {
                    n += 1;
                    last = Some(r.group);
                }
            }
            n
        } else {
            0
        };
        let height =
            (headers as f32 * HEADER + self.rows.len().max(1) as f32 * ROW).min(MAX_LIST_HEIGHT);
        egui::ScrollArea::vertical()
            .id_salt("command-palette-rows")
            .min_scrolled_height(height)
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut last = None;
                for (i, row) in self.rows.iter().enumerate() {
                    if last != Some(row.group) && mode == PaletteMode::Search {
                        last = Some(row.group);
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), HEADER),
                            Sense::hover(),
                        );
                        ui.painter().text(
                            rect.left_bottom() + Vec2::new(8.0, -4.0),
                            egui::Align2::LEFT_BOTTOM,
                            row.group.title(),
                            fonts::caption(),
                            theme::text_3(),
                        );
                    }
                    let selected = i == self.selected;
                    let resp = paint_row(ui, row, selected);
                    if selected && std::mem::take(&mut self.scroll_to_selected) {
                        resp.scroll_to_me(None);
                    }
                    if resp.hovered() && ui.input(|inp| inp.pointer.delta() != Vec2::ZERO) {
                        hover = Some(i);
                    }
                    if resp.clicked() && row.enabled {
                        picked = Some(i);
                    }
                }
                if self.rows.is_empty() {
                    ui.label(
                        egui::RichText::new("Nothing matches")
                            .font(fonts::small())
                            .color(theme::text_3()),
                    );
                }
            });
        if let Some(i) = hover {
            self.selected = i;
        }
        if let Some(i) = picked {
            self.selected = i;
            return self.pick();
        }
        None
    }
}

fn paint_row(ui: &mut egui::Ui, row: &Row, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ROW),
        if row.enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, row.enabled, &row.title));
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, theme::R_SM, theme::select_weak());
    }
    let text = if row.enabled {
        theme::text_1()
    } else {
        theme::text_off()
    };
    let mut x = rect.left() + 8.0;
    if let Some(class) = row.class {
        let icon = Rect::from_center_size(
            Pos2::new(x + 8.0, rect.center().y),
            Vec2::splat(theme::ICON_BOX),
        );
        crate::icons::paint_class(painter, icon, class, theme::icon());
        x += theme::ICON_BOX + 8.0;
    }
    let mut right = rect.right() - 8.0;
    if !row.shortcut.is_empty() {
        let cap =
            widgets::button::keycap_at(painter, Pos2::new(right, rect.center().y), &row.shortcut);
        right = cap.left() - 8.0;
    }
    let title = widgets::truncated(painter, &row.title, fonts::body(), text, right - x);
    let title_w = title.size().x;
    painter.galley(
        Pos2::new(x, rect.center().y - title.size().y / 2.0),
        title,
        text,
    );
    if !row.detail.is_empty() {
        let dx = x + title_w + 10.0;
        if right - dx > 24.0 {
            let detail = widgets::truncated(
                painter,
                &row.detail,
                fonts::small(),
                theme::text_3(),
                right - dx,
            );
            painter.galley(
                Pos2::new(dx, rect.center().y - detail.size().y / 2.0),
                detail,
                theme::text_3(),
            );
        }
    }
    resp
}

/// The container a new instance goes into: the active selection unless it is a part.
pub fn insert_parent(state: &EditorState) -> Option<InstanceId> {
    state
        .selection
        .active()
        .filter(|&a| !state.world.is_a(a, "BasePart"))
}

/// Instances matching `q`, one row per (name, class), best first, at most `cap` rows.
fn object_rows(world: &World, q: &str, cap: usize, out: &mut Vec<Row>) {
    struct Hit {
        score: i32,
        name: String,
        class: &'static str,
        ids: Vec<InstanceId>,
    }
    let mut hits: Vec<Hit> = Vec::new();
    let mut index: HashMap<(String, &'static str), usize> = HashMap::new();
    let mut stack: Vec<InstanceId> = world.roots().iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        stack.extend(world.children(id).iter().rev());
        let Some(class) = world.class_name(id) else {
            continue;
        };
        let name = match world.get_ref(id, "Name") {
            Ok(Value::String(s)) => s.as_str(),
            _ => class,
        };
        let score = fuzzy_score(q, name).or_else(|| fuzzy_score(q, class).map(|s| s - 50));
        let Some(score) = score else { continue };
        match index.get(&(name.to_string(), class)) {
            Some(&i) => hits[i].ids.push(id),
            None => {
                index.insert((name.to_string(), class), hits.len());
                hits.push(Hit {
                    score,
                    name: name.to_string(),
                    class,
                    ids: vec![id],
                });
            }
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
    out.extend(hits.into_iter().take(cap).map(|h| {
        let path = world
            .parent(h.ids[0])
            .map(|p| world.full_name(p))
            .unwrap_or_default();
        let count = match h.ids.len() {
            1 => String::new(),
            n => format!(" · {n} objects"),
        };
        Row {
            group: Group::Objects,
            title: h.name,
            detail: if path.is_empty() {
                format!("{}{count}", h.class)
            } else {
                format!("{} · {path}{count}", h.class)
            },
            shortcut: String::new(),
            enabled: true,
            class: Some(h.class),
            choice: Choice::Select(h.ids),
        }
    }));
}

/// How well `query` matches `text`, higher is better; `None` when it does not match. Case is
/// ignored. Best first: the text starts with the query, a word starts with it, the query is the
/// words' initials, the query is inside a word, the letters are scattered in order. Shorter texts
/// win ties. An empty query matches everything with 0.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let word_start = |i: usize| {
        i == 0 || {
            let prev = text.chars().nth(i - 1).unwrap_or(' ');
            let here = text.chars().nth(i).unwrap_or(' ');
            !prev.is_alphanumeric() || (prev.is_lowercase() && here.is_uppercase())
        }
    };
    let short = 100 - t.len().min(100) as i32;
    // Substring at the start or at a word start.
    let pos = t.windows(q.len()).position(|w| w == q.as_slice());
    match pos {
        Some(0) => return Some(3000 + short),
        Some(p) if word_start(p) => return Some(2000 + short),
        _ => {}
    }
    // Initials ("fs" for "Frame selection").
    let mut qi = 0;
    for (i, &c) in t.iter().enumerate() {
        if qi < q.len() && word_start(i) && c == q[qi] {
            qi += 1;
        }
    }
    if qi == q.len() {
        return Some(1500 + short);
    }
    // Substring inside a word.
    if pos.is_some() {
        return Some(1000 + short);
    }
    // Letters in order.
    let mut score = 0;
    let mut qi = 0;
    let mut last = None;
    for (i, &c) in t.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            score += if word_start(i) { 30 } else { 10 };
            if last == Some(i.wrapping_sub(1)) {
                score += 15;
            }
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score + short / 10)
}

fn size_text(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b.div_ceil(1024)),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

fn asset_kind(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mscene" => "Scene",
        "luau" | "lua" => "Script",
        "png" | "jpg" | "jpeg" | "webp" | "ktx2" => "Image",
        "ogg" | "wav" | "mp3" | "flac" => "Sound",
        "glb" | "gltf" | "obj" | "fbx" => "Model",
        "ttf" | "otf" => "Font",
        "toml" | "json" | "ron" => "Settings",
        _ => "File",
    }
}

/// Files under `dir` (hidden files and `target` folders skipped), at most [`MAX_ASSETS`].
fn scan_assets(dir: &Path) -> Vec<AssetFile> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let path = e.path();
            if meta.is_dir() {
                if depth < 8 {
                    stack.push((path, depth + 1));
                }
            } else if let Ok(rel) = path.strip_prefix(dir) {
                out.push(AssetFile {
                    rel: rel.to_string_lossy().replace('\\', "/"),
                    kind: asset_kind(&path),
                    size: meta.len(),
                });
                if out.len() >= MAX_ASSETS {
                    return out;
                }
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::default_actions;
    use crate::output::OutputLog;

    fn state() -> EditorState {
        let mut world = World::new_with_services();
        let ws = world.root("Workspace").unwrap();
        for _ in 0..7 {
            let coin = world.create("Part", Some(ws)).unwrap();
            world
                .set(coin, "Name", Value::String("Coin".into()))
                .unwrap();
        }
        let lava = world.create("Part", Some(ws)).unwrap();
        world
            .set(lava, "Name", Value::String("Lava".into()))
            .unwrap();
        EditorState::new(
            world,
            Path::new("."),
            Path::new("unused.mscene"),
            "Test",
            OutputLog::new(64).sink(),
        )
    }

    #[test]
    fn substring_and_word_starts_rank_first() {
        assert!(fuzzy_score("save", "Save").unwrap() > fuzzy_score("save", "Autosave").unwrap());
        assert!(
            fuzzy_score("fs", "Frame selection").unwrap()
                > fuzzy_score("fs", "Offset").unwrap_or(-1)
        );
        assert!(fuzzy_score("dup", "Duplicate").is_some());
        assert!(fuzzy_score("xyz", "Duplicate").is_none());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        assert!(fuzzy_score("COIN", "coin").is_some(), "case is ignored");
    }

    #[test]
    fn groups_come_in_order_and_coins_count_as_one_row() {
        let s = state();
        let actions = default_actions();
        let keymap = Keymap::metallic();
        let mut p = CommandPalette::new();
        p.open("coin", Path::new("."));
        p.refresh(&s, &actions, &keymap);
        let rows = p.rows();
        assert_eq!(rows[0].group, Group::Objects);
        assert_eq!(rows[0].title, "Coin");
        assert!(rows[0].detail.contains("7 objects"), "{}", rows[0].detail);
        let Choice::Select(ids) = &rows[0].choice else {
            panic!("selects")
        };
        assert_eq!(ids.len(), 7);
        let groups: Vec<Group> = rows.iter().map(|r| r.group).collect();
        let mut sorted = groups.clone();
        sorted.sort();
        assert_eq!(groups, sorted, "groups in order");
        assert_eq!(rows.last().map(|r| r.group), Some(Group::Litha));
        assert_eq!(rows.last().unwrap().title, "Ask Litha about \"coin\"");
    }

    #[test]
    fn prefixes_narrow_and_commands_show_shortcuts() {
        let s = state();
        let actions = default_actions();
        let keymap = Keymap::metallic();
        let mut p = CommandPalette::new();
        p.open(">undo", Path::new("."));
        p.refresh(&s, &actions, &keymap);
        let undo = &p.rows()[0];
        assert_eq!(undo.group, Group::Commands);
        assert_eq!(undo.choice, Choice::Run("edit.undo"));
        assert_eq!(undo.shortcut, "Ctrl+Z");
        assert!(!undo.enabled, "nothing to undo yet");
        assert!(
            p.rows()
                .iter()
                .all(|r| matches!(r.group, Group::Commands | Group::Litha))
        );
        p.set_query("@lava");
        p.refresh(&s, &actions, &keymap);
        assert_eq!(p.rows()[0].title, "Lava");
        assert!(
            p.rows()
                .iter()
                .all(|r| matches!(r.group, Group::Objects | Group::Litha))
        );
    }

    #[test]
    fn recent_commands_lead_an_empty_query_and_tab_jumps_groups() {
        let s = state();
        let actions = default_actions();
        let keymap = Keymap::metallic();
        let mut p = CommandPalette::new();
        p.remember("view.frame_all");
        p.remember("snap.grid");
        p.open("", Path::new("."));
        p.refresh(&s, &actions, &keymap);
        assert_eq!(p.rows()[0].choice, Choice::Run("snap.grid"));
        assert_eq!(p.rows()[1].choice, Choice::Run("view.frame_all"));

        p.set_query("lava");
        p.refresh(&s, &actions, &keymap);
        assert_eq!(p.rows()[p.selected()].group, Group::Objects);
        p.next_group();
        assert_ne!(p.rows()[p.selected()].group, Group::Objects);
    }

    #[test]
    fn enter_picks_and_disabled_rows_are_skipped() {
        let s = state();
        let actions = default_actions();
        let keymap = Keymap::metallic();
        let mut p = CommandPalette::new();
        p.open(">frame all", Path::new("."));
        p.refresh(&s, &actions, &keymap);
        assert_eq!(p.pick(), Some(Choice::Run("view.frame_all")));
        assert!(!p.is_open());
        assert_eq!(p.recent().next(), Some("view.frame_all"));

        p.open("lava", Path::new("."));
        assert!(matches!(p.ask_litha(), Choice::AskLitha(q) if q == "lava"));
    }

    #[test]
    fn insert_palette_lists_classes_at_the_cursor_point() {
        let s = state();
        let actions = default_actions();
        let keymap = Keymap::metallic();
        let mut p = CommandPalette::new();
        let at = Vec3::new(4.0, 1.5, -2.0);
        p.open_insert(Pos2::new(300.0, 200.0), at, None);
        p.set_query("spawn");
        p.refresh(&s, &actions, &keymap);
        assert_eq!(
            p.pick(),
            Some(Choice::Insert {
                class: "SpawnLocation",
                parent: None,
                at
            })
        );
    }
}
