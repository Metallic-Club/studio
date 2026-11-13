//! Studio's look, after the Metallic website: the design [`tokens`], dark and light modes (or the system's), the
//! user's accent colour, a high-contrast option, the bundled [`fonts`], and the `egui::Style` a look becomes (ADR
//! 0009, Theme).
//!
//! [`ThemeState`] holds the look Studio wants and re-applies the style only when the look (or, in System mode, the
//! system's mode) changes, so a frame never rebuilds it.
//!
//! A focused text field is outlined in the accent, a point wide (#588), by Studio's own text field style
//! ([`FieldStyle`]): egui outlines it with the selection's stroke, which is the ink drawn on selected text, dark and
//! three points wide.

pub mod fonts;
pub mod icons;
pub mod tokens;

use egui::class::HasClasses as _;
use egui::epaint::{CornerRadius, Margin, Stroke};
use egui::style::{Selection, WidgetVisuals, Widgets};
use egui::theme::{DefaultStyle, StyleProvider};
use egui::widget_style::{StyleArgs, TextEditStyle, WidgetState};
use egui::{Color32, FontFamily, FontId, Pos2, Rect, TextEdit, TextStyle, Vec2, pos2, vec2};

use tokens::{ACCENT_INK, Accent, Hue, Motion, Palette, radius, shadow, size, space, text};

/// Dark, light, or whichever the system uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeMode {
    /// The default, and always the splash's.
    #[default]
    Dark,
    /// The light mode.
    Light,
    /// Follows the system's setting (dark when the system says nothing).
    System,
}

impl ThemeMode {
    /// Every mode, in the order Settings lists them.
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Dark, ThemeMode::Light, ThemeMode::System];

    /// The id the settings store.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
            ThemeMode::System => "system",
        }
    }

    /// Reads an id in any letter case; `None` for an unknown one.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str().eq_ignore_ascii_case(id.trim()))
    }

    /// Whether it is dark, given whether the system is (`None`: the system did not say).
    pub fn is_dark(self, system_dark: Option<bool>) -> bool {
        match self {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => system_dark.unwrap_or(true),
        }
    }
}

/// What the interface looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Look {
    /// Dark, light or the system's.
    pub mode: ThemeMode,
    /// The accent colour.
    pub accent: Accent,
    /// Secondary text as strong as body text, and stronger borders.
    pub high_contrast: bool,
    /// Turns off fades and eases.
    pub reduce_motion: bool,
}

impl Look {
    /// The durations this look moves with.
    pub fn motion(self) -> Motion {
        Motion::new(self.reduce_motion)
    }

    /// The colours of this look, given whether the system is dark.
    pub fn colors(self, system_dark: Option<bool>) -> Colors {
        let dark = self.mode.is_dark(system_dark);
        let base = if dark { tokens::DARK } else { tokens::LIGHT };
        Colors {
            dark,
            palette: if self.high_contrast {
                base.high_contrast(dark)
            } else {
                base
            },
            accent: self.accent.on(dark),
            accent_fill: self.accent.colors().color,
            accent_hover: self.accent.colors().hover,
            high_contrast: self.high_contrast,
        }
    }
}

/// The resolved colours of a [`Look`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    /// Whether they are the dark mode's.
    pub dark: bool,
    /// The neutrals.
    pub palette: Palette,
    /// The accent as an outline or bar on this mode's ground.
    pub accent: Color32,
    /// The accent as a fill (text selection), with [`tokens::ACCENT_INK`] on it.
    pub accent_fill: Color32,
    /// The accent under the pointer.
    pub accent_hover: Color32,
    /// Whether the high-contrast option is on (the icon hues follow it).
    pub high_contrast: bool,
}

/// How a play control's icon is drawn (#302), on the top bar and as a toolbar tool alike (#325).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayControl {
    /// Play: a filled green triangle.
    Play,
    /// Stop: a filled red square.
    Stop,
    /// Pause and Step: the text colour, outlined.
    Plain,
}

impl PlayControl {
    /// Its icon's colour while it can be used, on the dark or the light mode. Greyed out when it cannot is the
    /// caller's: the text at 48 %.
    pub fn ink(self, colors: &Colors) -> Color32 {
        match (self, colors.dark) {
            (PlayControl::Play, true) => tokens::play::ICON_DARK,
            (PlayControl::Play, false) => tokens::play::ICON_LIGHT,
            (PlayControl::Stop, true) => tokens::play::STOP_DARK,
            (PlayControl::Stop, false) => tokens::play::STOP_LIGHT,
            (PlayControl::Plain, _) => colors.palette.text,
        }
    }

    /// Whether its icon is filled.
    pub fn filled(self) -> bool {
        self != PlayControl::Plain
    }
}

impl Colors {
    /// The colour of an icon hue in these colours (#231).
    pub fn hue(&self, hue: Hue) -> Color32 {
        hue.color(self.dark, self.high_contrast)
    }
}

impl Default for Colors {
    fn default() -> Self {
        Look::default().colors(None)
    }
}

/// The look Studio wants, and what was last applied to the egui context.
#[derive(Debug, Default)]
pub struct ThemeState {
    wanted: Look,
    applied: Option<Colors>,
    applied_look: Option<Look>,
}

impl ThemeState {
    /// A state that wants `look` and has applied nothing yet.
    pub fn new(look: Look) -> Self {
        Self {
            wanted: look,
            applied: None,
            applied_look: None,
        }
    }

    /// The look wanted.
    pub fn look(&self) -> Look {
        self.wanted
    }

    /// Asks for `look`; it is applied at the next [`ThemeState::apply`].
    pub fn set(&mut self, look: Look) {
        self.wanted = look;
    }

    /// The colours applied last ([`Colors::default`] before any).
    pub fn colors(&self) -> Colors {
        self.applied.unwrap_or_default()
    }

    /// Applies the wanted look to `ctx` unless it is already applied. Returns whether it did.
    pub fn apply(&mut self, ctx: &egui::Context) -> bool {
        let system_dark = ctx.system_theme().map(|theme| theme == egui::Theme::Dark);
        let colors = self.wanted.colors(system_dark);
        if self.applied == Some(colors) && self.applied_look == Some(self.wanted) {
            return false;
        }
        let egui_theme = if colors.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        };
        ctx.set_style_of(egui_theme, style(&colors, self.wanted.motion()));
        // Replaces egui's own, which `Context::default` registered; it reads the colours from the style it is given.
        ctx.replace_widget_theme::<TextEditStyle>(FieldStyle);
        ctx.set_theme(egui_theme);
        ctx.data_mut(|data| data.insert_temp(colors_key(), colors));
        self.applied = Some(colors);
        self.applied_look = Some(self.wanted);
        true
    }
}

/// The colours `ctx` shows now ([`ThemeState::apply`] records them), the default look's before any.
pub fn current_colors(ctx: &egui::Context) -> Colors {
    ctx.data(|data| data.get_temp::<Colors>(colors_key()))
        .unwrap_or_default()
}

/// Where the applied colours are kept in the context's memory.
fn colors_key() -> egui::Id {
    egui::Id::unique("metallic.theme")
}

/// Studio's text field style (#588): egui's, but a focused field is outlined as an active widget is, in the accent a
/// point wide, rather than in the selection's stroke (the dark ink of selected text, three points wide). The selection
/// keeps that stroke, which egui also draws selected text in.
#[derive(Clone, Copy, Debug)]
pub struct FieldStyle;

impl StyleProvider<TextEditStyle> for FieldStyle {
    fn style(&mut self, args: &StyleArgs<'_>) -> TextEditStyle {
        let mut field = StyleProvider::<TextEditStyle>::style(&mut DefaultStyle, args);
        if args.state == WidgetState::Active && !args.classes.has_class(&TextEdit::CLASS_READ_ONLY) {
            // Taking the frame's stroke back to a point widens its margin by the difference: the text stays put.
            field.atom_layout.frame = field
                .atom_layout
                .frame
                .apply_stroke_and_expansion_without_layout_shift(args.style.visuals.widgets.active.bg_stroke, 0.0);
        }
        field
    }
}

/// The full `egui::Style` of `colors`, moving with `motion`.
pub fn style(colors: &Colors, motion: Motion) -> egui::Style {
    let mut style = egui::Style {
        animation_time: motion.fast,
        ..egui::Style::default()
    };
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(text::SMALL)),
        (TextStyle::Body, FontId::proportional(text::BODY)),
        (TextStyle::Button, fonts::named(fonts::STRONG, text::BODY)),
        (TextStyle::Heading, fonts::named(fonts::HEADING, text::TITLE)),
        (TextStyle::Monospace, FontId::new(text::MONO, FontFamily::Monospace)),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(space::S4, space::S4);
    spacing.button_padding = vec2(space::S6, space::S3);
    spacing.interact_size = vec2(size::CONTROL, size::ROW);
    spacing.window_margin = Margin::same(space::S5 as i8);
    spacing.menu_margin = Margin::same(space::S2 as i8);
    spacing.indent = space::S6;

    style.visuals = visuals(colors);
    style
}

fn visuals(colors: &Colors) -> egui::Visuals {
    let p = &colors.palette;
    let mut visuals = if colors.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let widget = |fill: Color32, edge: Color32, ink: Color32| WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(size::BORDER, edge),
        corner_radius: CornerRadius::same(radius::ITEM),
        fg_stroke: Stroke::new(1.0, ink),
        expansion: 0.0,
    };
    // Widgets are the secondary button's states: a surface with a 1 px border, whose border goes to `subtle` under
    // the pointer. The primary and quiet buttons set their own fills ([`button`]).
    visuals.widgets = Widgets {
        noninteractive: widget(p.panel, p.border, p.muted),
        inactive: widget(p.surface, p.border, p.text),
        hovered: widget(p.surface_hover, p.subtle, p.text),
        active: widget(p.surface_hover, colors.accent, p.text),
        open: widget(p.surface, p.subtle, p.text),
    };
    visuals.selection = Selection {
        bg_fill: colors.accent_fill,
        stroke: Stroke::new(size::ACCENT_BAR, ACCENT_INK),
    };
    visuals.hyperlink_color = colors.accent;
    visuals.faint_bg_color = p.panel;
    visuals.extreme_bg_color = p.input;
    visuals.text_edit_bg_color = Some(p.input);
    visuals.code_bg_color = p.input;
    visuals.window_fill = p.surface;
    visuals.window_stroke = Stroke::new(size::BORDER, p.border);
    visuals.window_corner_radius = CornerRadius::same(radius::DIALOG);
    visuals.menu_corner_radius = CornerRadius::same(radius::PANEL);
    visuals.panel_fill = p.bg;
    visuals.popup_shadow = shadow::MENU;
    visuals.window_shadow = shadow::MENU;
    visuals.disabled_alpha = 0.48;
    visuals
}

/// The outline of `rect` with every corner rounded by `radius`: clockwise from the top-right corner, convex, with
/// `arc_segments` segments per corner. The splash's window region follows it.
pub fn rounded_outline(rect: Rect, radius: f32, arc_segments: usize) -> Vec<Pos2> {
    let r = radius.clamp(0.0, rect.width().min(rect.height()) / 2.0);
    let n = arc_segments.max(1);
    let quarter = std::f32::consts::FRAC_PI_2;
    let mut points = Vec::with_capacity(4 * (n + 1));
    // Each corner is a quarter circle from its start angle (y grows downwards, so angles turn clockwise).
    for (centre, start) in [
        (pos2(rect.right() - r, rect.top() + r), -quarter),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), quarter),
        (pos2(rect.left() + r, rect.top() + r), 2.0 * quarter),
    ] {
        for i in 0..=n {
            points.push(centre + r * Vec2::angled(start + quarter * i as f32 / n as f32));
        }
    }
    points
}

/// A button's kind (the website's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// The one action of a view: neutral and bright, never the accent.
    Primary,
    /// The default: a surface with a border.
    Secondary,
    /// No fill, muted text.
    Quiet,
    /// Runs the game: green.
    Play,
}

/// A button of `kind` labelled `label`.
pub fn button(ui: &mut egui::Ui, kind: ButtonKind, label: &str) -> egui::Response {
    button_of_width(ui, kind, label, 0.0)
}

/// A button of `kind` labelled `label`, as wide as the space it is in (a card's stacked actions).
pub fn wide_button(ui: &mut egui::Ui, kind: ButtonKind, label: &str) -> egui::Response {
    let width = ui.available_width();
    button_of_width(ui, kind, label, width)
}

/// A drop-down's arrow, for [`egui::ComboBox::icon`]: Lucide's chevron in place of egui's triangle (#278), in the
/// widget's text colour.
pub fn combo_icon(ui: &egui::Ui, rect: Rect, visuals: &WidgetVisuals, _open: bool) {
    let side = rect.height().min(rect.width()).max(text::SMALL);
    icons::paint(
        ui.painter(),
        icons::Icon::UiCollapse,
        Rect::from_center_size(rect.center(), Vec2::splat(side)),
        visuals.fg_stroke.color,
    );
}

/// The header of one of Studio's windows (#278), drawn in it in place of egui's title bar: `title` centred, and a
/// Lucide close button on the right, labelled "Close `title`", which sets `open` to false. A line under it, as
/// egui's.
pub fn window_header(ui: &mut egui::Ui, title: &str, open: &mut bool) {
    let p = current_colors(ui.ctx()).palette;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), WINDOW_HEADER), egui::Sense::hover());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        title,
        fonts::named(fonts::HEADING, text::BODY_LG),
        p.text,
    );
    let close = Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), Vec2::splat(24.0));
    let response = ui.interact(close, ui.scope_id().with(("close", title)), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Close {title}")));
    if response.hovered() {
        ui.painter()
            .rect_filled(close, CornerRadius::same(radius::ITEM / 2), p.surface_hover);
    }
    icons::paint_at(
        ui.painter(),
        close.center(),
        egui::Align2::CENTER_CENTER,
        icons::Icon::UiClose,
        text::BODY_LG,
        if response.hovered() { p.text } else { p.muted },
    );
    if response.clicked() {
        *open = false;
    }
    ui.separator();
}

/// The height of [`window_header`]'s row.
const WINDOW_HEADER: f32 = 32.0;

/// A checkbox's box alone (#329), for a column of them under a heading: `label` is for screen readers and its
/// tooltip only.
pub fn checkbox_only(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    let p = current_colors(ui.ctx()).palette;
    let box_side = 16.0;
    let (rect, mut response) = ui.allocate_exact_size(Vec2::splat(box_side), egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, label));
    if ui.is_rect_visible(rect) {
        let strength = if ui.is_enabled() { 1.0 } else { 0.48 };
        let edge = if *on {
            p.action
        } else if response.hovered() {
            p.subtle
        } else {
            p.border
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(4),
            (if *on { p.action } else { p.input }).gamma_multiply(strength),
            Stroke::new(size::BORDER, edge.gamma_multiply(strength)),
            egui::StrokeKind::Inside,
        );
        if *on {
            icons::paint(
                ui.painter(),
                icons::Icon::UiCheck,
                rect.shrink(2.0),
                p.action_text.gamma_multiply(strength),
            );
        }
    }
    response.on_hover_text(label)
}

/// The width of a panel's options menu (#477), so its section headers have a line to fill.
pub const MENU_WIDTH: f32 = 220.0;

/// A collapsible section of a menu (#477): a chevron and an uppercase label, as Properties labels its groups, which
/// fold `add_contents` away or show it. Open at first; egui's memory keeps it as it was left, by `id_salt`. Returns
/// the header's response.
pub fn section(
    ui: &mut egui::Ui,
    id_salt: &str,
    label: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let p = current_colors(ui.ctx()).palette;
    let id = ui.make_persistent_id(id_salt);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
    let open = state.is_open();
    let width = ui.available_width().min(MENU_WIDTH).max(ui.min_rect().width());
    let (rect, response) = ui.allocate_exact_size(vec2(width, size::ROW), egui::Sense::click());
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label);
        info.selected = Some(open);
        info
    });
    if response.clicked() {
        state.toggle(ui);
    }
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM / 2), p.border.gamma_multiply(0.5));
        }
        let ink = if response.hovered() { p.text } else { p.subtle };
        let chevron = if open {
            icons::Icon::UiCollapse
        } else {
            icons::Icon::UiExpand
        };
        let at = pos2(rect.left() + space::S2, rect.center().y);
        icons::paint_at(ui.painter(), at, egui::Align2::LEFT_CENTER, chevron, text::BODY, ink);
        // Spaced like `section_label`'s, the uppercase labels of Properties and Settings.
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &label.to_uppercase(),
            0.0,
            egui::TextFormat {
                font_id: fonts::named(fonts::STRONG, text::LABEL),
                extra_letter_spacing: text::LABEL_TRACKING,
                color: ink,
                ..Default::default()
            },
        );
        let galley = ui.painter().layout_job(job);
        let at = pos2(at.x + text::BODY + space::S3, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, ink);
    }
    state.show_body_unindented(ui, |ui| {
        ui.add_space(space::S1);
        add_contents(ui);
        ui.add_space(space::S3);
    });
    state.store(ui.ctx());
    response
}

/// A checkbox (#278): a rounded box, filled with the primary action's colour and Lucide's check when `on`, then
/// `label`. Clicking either toggles it.
pub fn checkbox(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    let p = current_colors(ui.ctx()).palette;
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(text::BODY), p.text);
    let box_side = 16.0;
    let size = vec2(box_side + space::S3 + galley.size().x, box_side.max(galley.size().y));
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, label));
    if ui.is_rect_visible(rect) {
        let mark = Rect::from_min_size(
            pos2(rect.left(), rect.center().y - box_side / 2.0),
            Vec2::splat(box_side),
        );
        let edge = if *on {
            p.action
        } else if response.hovered() {
            p.subtle
        } else {
            p.border
        };
        ui.painter().rect(
            mark,
            CornerRadius::same(4),
            if *on { p.action } else { p.input },
            Stroke::new(size::BORDER, edge),
            egui::StrokeKind::Inside,
        );
        if *on {
            icons::paint(ui.painter(), icons::Icon::UiCheck, mark.shrink(2.0), p.action_text);
        }
        ui.painter().galley(
            pos2(mark.right() + space::S3, rect.center().y - galley.size().y / 2.0),
            galley,
            p.text,
        );
    }
    response
}

/// One choice of several (#478), drawn as [`checkbox`] is but round: a ring, filled with the primary action's colour
/// and a dot when `chosen`, then `label`. Greyed and inert when `enabled` is false. Returns the response; clicked
/// means chosen.
pub fn radio(ui: &mut egui::Ui, chosen: bool, enabled: bool, label: &str) -> egui::Response {
    let p = current_colors(ui.ctx()).palette;
    let strength = if enabled { 1.0 } else { 0.48 };
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(text::BODY),
        p.text.gamma_multiply(strength),
    );
    let side = 16.0;
    let size = vec2(side + space::S3 + galley.size().x, side.max(galley.size().y));
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, chosen, label));
    if ui.is_rect_visible(rect) {
        let centre = pos2(rect.left() + side / 2.0, rect.center().y);
        let edge = if chosen {
            p.action
        } else if response.hovered() && enabled {
            p.subtle
        } else {
            p.border
        };
        ui.painter().circle(
            centre,
            side / 2.0 - size::BORDER / 2.0,
            (if chosen { p.action } else { p.input }).gamma_multiply(strength),
            Stroke::new(size::BORDER, edge.gamma_multiply(strength)),
        );
        if chosen {
            ui.painter()
                .circle_filled(centre, 3.5, p.action_text.gamma_multiply(strength));
        }
        ui.painter().galley(
            pos2(rect.left() + side + space::S3, rect.center().y - galley.size().y / 2.0),
            galley,
            p.text,
        );
    }
    response
}

fn button_of_width(ui: &mut egui::Ui, kind: ButtonKind, label: &str, width: f32) -> egui::Response {
    let p = current_colors(ui.ctx()).palette;
    let (fill, hover, ink) = match kind {
        ButtonKind::Primary => (p.action, p.action_hover, p.action_text),
        ButtonKind::Secondary => return ui.add(egui::Button::new(label).min_size(vec2(width, size::CONTROL))),
        ButtonKind::Quiet => (Color32::TRANSPARENT, p.surface_hover, p.muted),
        ButtonKind::Play => (tokens::play::FILL, tokens::play::HOVER, tokens::play::TEXT),
    };
    let id = ui.next_auto_id();
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let text = egui::RichText::new(label)
        .font(fonts::named(fonts::STRONG, text::BODY))
        .color(ink);
    ui.add(
        egui::Button::new(text)
            .fill(if hovered { hover } else { fill })
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(radius::ITEM))
            .min_size(vec2(width, size::CONTROL)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse_and_resolve() {
        for mode in ThemeMode::ALL {
            assert_eq!(ThemeMode::parse(&mode.as_str().to_uppercase()), Some(mode));
        }
        assert_eq!(ThemeMode::parse("sepia"), None);
        assert_eq!(ThemeMode::default(), ThemeMode::Dark);
        assert!(ThemeMode::Dark.is_dark(Some(false)));
        assert!(!ThemeMode::Light.is_dark(Some(true)));
        assert!(ThemeMode::System.is_dark(Some(true)));
        assert!(!ThemeMode::System.is_dark(Some(false)));
        assert!(ThemeMode::System.is_dark(None), "dark when the system says nothing");
    }

    #[test]
    fn a_look_resolves_its_colours() {
        let dark = Look::default().colors(None);
        assert!(dark.dark);
        assert_eq!(dark.palette, tokens::DARK);
        assert_eq!(dark.accent, tokens::Accent::Orange.colors().color);

        let light = Look {
            mode: ThemeMode::Light,
            accent: Accent::Mint,
            ..Look::default()
        }
        .colors(None);
        assert!(!light.dark);
        assert_eq!(light.palette, tokens::LIGHT);
        assert_eq!(
            light.accent,
            Accent::Mint.colors().light,
            "the light mode's darker accent"
        );
        assert_eq!(light.accent_fill, Accent::Mint.colors().color);

        let strong = Look {
            high_contrast: true,
            ..Look::default()
        }
        .colors(None);
        assert_eq!(strong.palette.muted, tokens::DARK.text);
        assert_eq!(strong.palette.border, Color32::from_rgb(0x89, 0x89, 0x89));
    }

    #[test]
    fn a_style_carries_its_colours() {
        for look in [
            Look::default(),
            Look {
                mode: ThemeMode::Light,
                accent: Accent::Rose,
                ..Look::default()
            },
        ] {
            let colors = look.colors(None);
            let style = style(&colors, look.motion());
            let p = colors.palette;
            assert_eq!(style.visuals.dark_mode, colors.dark);
            assert_eq!(style.visuals.panel_fill, p.bg);
            assert_eq!(style.visuals.widgets.inactive.bg_fill, p.surface);
            assert_eq!(style.visuals.widgets.inactive.bg_stroke, Stroke::new(1.0, p.border));
            assert_eq!(style.visuals.widgets.hovered.bg_stroke.color, p.subtle);
            assert_eq!(style.visuals.widgets.inactive.corner_radius, CornerRadius::same(8));
            assert_eq!(style.visuals.selection.bg_fill, colors.accent_fill);
            assert_eq!(style.visuals.selection.stroke.color, ACCENT_INK);
            assert_eq!(style.visuals.disabled_alpha, 0.48);
            assert_eq!(style.animation_time, 0.12);
            let button = &style.text_styles[&TextStyle::Button];
            assert_eq!(button.family, FontFamily::Name(fonts::STRONG.into()));
            assert_eq!(button.size, 14.0);
        }
        let reduced = style(&Colors::default(), Motion::new(true));
        assert_eq!(reduced.animation_time, 0.0);
    }

    /// #588: a focused field is outlined in the accent a point wide, not in the dark ink of selected text.
    #[test]
    fn a_focused_field_is_outlined_in_the_accent() {
        let ctx = egui::Context::default();
        let mut state = ThemeState::new(Look::default());
        state.apply(&ctx);
        let field = egui::Id::unique("field");
        let mut text = String::from("Part");
        let mut outlines = |ctx: &egui::Context| -> Vec<Stroke> {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.add(TextEdit::singleline(&mut text).id(field));
            });
            output.textures_delta.clear();
            output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Rect(rect) => Some(rect.stroke),
                    _ => None,
                })
                .collect()
        };
        let idle = outlines(&ctx);
        assert!(
            idle.contains(&Stroke::new(size::BORDER, state.colors().palette.border)),
            "{idle:?}"
        );
        ctx.memory_mut(|memory| memory.request_focus(field));
        // A field is styled by its state in the frame before.
        outlines(&ctx);
        let focused = outlines(&ctx);
        assert!(
            focused.contains(&Stroke::new(size::BORDER, state.colors().accent)),
            "{focused:?}"
        );
        assert!(focused.iter().all(|stroke| stroke.color != ACCENT_INK), "{focused:?}");
    }

    #[test]
    fn the_style_is_applied_only_when_the_look_changes() {
        let ctx = egui::Context::default();
        let current = |ctx: &egui::Context| ctx.style_of(ctx.theme());
        let mut state = ThemeState::new(Look::default());
        assert!(state.apply(&ctx), "the first time");
        assert!(!state.apply(&ctx), "nothing changed");
        assert_eq!(current(&ctx).visuals.panel_fill, tokens::DARK.bg);
        assert_eq!(current_colors(&ctx), state.colors());

        state.set(Look {
            mode: ThemeMode::Light,
            ..Look::default()
        });
        assert!(state.apply(&ctx));
        assert!(!state.apply(&ctx));
        assert!(!current(&ctx).visuals.dark_mode);
        assert_eq!(current(&ctx).visuals.panel_fill, tokens::LIGHT.bg);

        state.set(Look {
            mode: ThemeMode::Light,
            reduce_motion: true,
            ..Look::default()
        });
        assert!(state.apply(&ctx), "reduced motion is part of the look");
        assert_eq!(current(&ctx).animation_time, 0.0);
    }

    #[test]
    fn the_rounded_outline_is_convex_and_inside_its_rect() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(720.0, 420.0));
        let points = rounded_outline(rect, 12.0, 4);
        assert_eq!(points.len(), 20);
        assert!(points.iter().all(|p| rect.expand(0.001).contains(*p)));
        // Convex and clockwise on screen: every turn has the same sign.
        let turns: Vec<f32> = (0..points.len())
            .map(|i| {
                let (a, b, c) = (
                    points[i],
                    points[(i + 1) % points.len()],
                    points[(i + 2) % points.len()],
                );
                (b - a).x * (c - b).y - (b - a).y * (c - b).x
            })
            .filter(|t| t.abs() > 1e-3)
            .collect();
        assert!(turns.iter().all(|t| *t > 0.0), "{turns:?}");
        // Every corner's tip is rounded off; no corner is cut.
        let inside = |p: Pos2| {
            (0..points.len()).all(|i| {
                let (a, b) = (points[i], points[(i + 1) % points.len()]);
                (b - a).x * (p - a).y - (b - a).y * (p - a).x >= 0.0
            })
        };
        assert!(inside(rect.center()));
        for corner in [pos2(0.5, 0.5), pos2(719.5, 0.5), pos2(719.5, 419.5), pos2(0.5, 419.5)] {
            assert!(!inside(corner), "{corner:?}");
        }
        assert!(inside(pos2(716.0, 4.0)), "a rounded corner, not a chamfer");
    }
}
