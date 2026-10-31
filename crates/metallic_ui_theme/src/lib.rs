//! The Metallic UI theme, shared by Studio and the animation and VFX editors (it depends on egui
//! and the tokens only, never on Studio or the renderer).
//!
//! The Metallic Studio look: machined graphite with one restrained violet accent.
//!
//! Every value comes from the shared design tokens (`design/tokens/metallic.tokens.json`, read
//! through [`metallic_tokens::consts`]); nothing here names a colour or a size of its own. The
//! widget states follow `docs/research/2026-10-30-ui-overhaul.md` section 3.7 as far as egui's
//! [`Visuals`] can express them; the raised-plate edges and the cut come with Studio's own widgets.

pub mod fonts;
pub mod icons;
pub mod saved;

pub use saved::{ThemeChoice, saved_theme_choice};

use std::cell::Cell;
use std::sync::OnceLock;

use egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Visuals,
    style::{TextCursorStyle, WidgetVisuals},
};
use metallic_tokens::consts as tk;

const fn rgba(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied_const(c[0], c[1], c[2], c[3])
}

const fn shadow(s: metallic_tokens::Shadow) -> Shadow {
    Shadow {
        offset: [s.offset[0] as i8, s.offset[1] as i8],
        blur: s.blur as u8,
        spread: s.spread as u8,
        color: rgba(s.color),
    }
}

/// Title bar, text fields, viewport surround.
const BG_DEEP: Color32 = rgba(tk::BG_DEEP);
/// Window ground.
const BG_BASE: Color32 = rgba(tk::BG_BASE);
/// Docked panels, rail, cards at rest.
const BG_PANEL: Color32 = rgba(tk::BG_PANEL);
/// Secondary buttons, selected card, menus.
const BG_RAISED: Color32 = rgba(tk::BG_RAISED);
/// Hover fill, toasts.
const BG_HOVER: Color32 = rgba(tk::BG_HOVER);
/// Pressed fill, selected segment, keycaps.
const BG_ACTIVE: Color32 = rgba(tk::BG_ACTIVE);
/// Selected row.
const SELECT: Color32 = rgba(tk::SELECT);
/// Multi-selected row.
const SELECT_WEAK: Color32 = rgba(tk::SELECT_WEAK);
/// Dividers.
const LINE: Color32 = rgba(tk::LINE);
/// Panel edges.
const LINE_STRONG: Color32 = rgba(tk::LINE_STRONG);
/// Input borders (3:1 against their surroundings).
const LINE_CONTROL: Color32 = rgba(tk::LINE_CONTROL);
/// Body text.
const TEXT_1: Color32 = rgba(tk::TEXT_1);
/// Secondary text.
const TEXT_2: Color32 = rgba(tk::TEXT_2);
/// Hints.
const TEXT_3: Color32 = rgba(tk::TEXT_3);
/// Disabled text.
const TEXT_OFF: Color32 = rgba(tk::TEXT_OFF);
/// Default icon glyph.
const ICON: Color32 = rgba(tk::ICON);
/// The one action per view and the selection ring.
const ACCENT: Color32 = rgba(tk::ACCENT);
/// Accent under the pointer.
const ACCENT_HOVER: Color32 = rgba(tk::ACCENT_HOVER);
/// Accent while pressed.
const ACCENT_PRESS: Color32 = rgba(tk::ACCENT_PRESS);
/// Links.
const ACCENT_TEXT: Color32 = rgba(tk::ACCENT_TEXT);
/// Text on the accent.
const ON_ACCENT: Color32 = rgba(tk::ON_ACCENT);
/// Keyboard focus ring (2 px).
const FOCUS: Color32 = rgba(tk::FOCUS);
/// 1 px top edge of raised plates.
const HIGHLIGHT: Color32 = rgba(tk::HIGHLIGHT);
/// Error text and icons.
const ERROR: Color32 = rgba(tk::ERROR);
/// Error background.
const ERROR_BG: Color32 = rgba(tk::ERROR_BG);
/// Warning text and icons.
const WARNING: Color32 = rgba(tk::WARNING);
/// Warning background.
const WARNING_BG: Color32 = rgba(tk::WARNING_BG);
/// Success text and icons.
const SUCCESS: Color32 = rgba(tk::SUCCESS);
/// Success background.
const SUCCESS_BG: Color32 = rgba(tk::SUCCESS_BG);
/// Information text and icons.
const INFO: Color32 = rgba(tk::INFO);
/// Information background.
const INFO_BG: Color32 = rgba(tk::INFO_BG);
/// Destructive buttons.
const DANGER: Color32 = rgba(tk::DANGER);
/// Destructive buttons under the pointer.
const DANGER_HOVER: Color32 = rgba(tk::DANGER_HOVER);
/// Text on the destructive fill.
const ON_DANGER: Color32 = rgba(tk::ON_DANGER);
/// Text on [`ART_SCRIM`] (the same in every theme).
pub const ART_TEXT: Color32 = rgba(tk::G_100);
/// Icons on [`ART_SCRIM`].
pub const ART_ICON: Color32 = rgba(tk::G_200);

/// Plate behind text and chips on game art (`art-scrim`).
const ART_SCRIM: Color32 = rgba(tk::ART_SCRIM);
/// 1 px bottom edge of raised plates (`edge-low`).
const EDGE_LOW: Color32 = rgba(tk::EDGE_LOW);
/// Keycap fill on the accent (`keycap-on-accent`).
const KEYCAP_ON_ACCENT: Color32 = rgba(tk::KEYCAP_ON_ACCENT);
/// Modal scrim (`backdrop`).
const BACKDROP: Color32 = rgba(tk::BACKDROP);

/// One of the themes in the shared token file (`$themes`; Graphite is the base set).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ThemeId {
    /// The default dark theme.
    #[default]
    Graphite,
    /// A lighter dark theme.
    Titanium,
    /// The light theme.
    Light,
}

impl ThemeId {
    /// Every theme, in the order Settings lists them.
    pub const ALL: [ThemeId; 3] = [ThemeId::Graphite, ThemeId::Titanium, ThemeId::Light];

    /// The id used in the token file and in `settings.toml`.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeId::Graphite => "graphite",
            ThemeId::Titanium => "titanium",
            ThemeId::Light => "light",
        }
    }

    /// The name creators see.
    pub fn label(self) -> &'static str {
        match self {
            ThemeId::Graphite => "Graphite",
            ThemeId::Titanium => "Titanium",
            ThemeId::Light => "Light",
        }
    }

    /// Reads an id (any letter case); `None` for an unknown name.
    pub fn parse(text: &str) -> Option<ThemeId> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str().eq_ignore_ascii_case(text.trim()))
    }

    /// True for the two dark themes.
    pub fn is_dark(self) -> bool {
        self != ThemeId::Light
    }

    fn index(self) -> usize {
        self as usize
    }
}

macro_rules! palette {
    ($($name:ident, $field:ident, $css:literal;)*) => {
        /// Every colour role of one theme.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Palette {
            $(#[doc = concat!("`", $css, "`")] pub $field: Color32,)*
        }

        impl Palette {
            const GRAPHITE: Palette = Palette { $($field: $name,)* };

            /// The colours of `theme` in `set`; a missing token falls back to Graphite's value.
            fn from_tokens(set: &metallic_tokens::TokenSet, theme: ThemeId) -> Palette {
                Palette {
                    $($field: metallic_tokens::themed_color(set, theme.as_str(), $css)
                        .map_or(Self::GRAPHITE.$field, rgba),)*
                }
            }
        }

        $(
            #[doc = concat!("The `", $css, "` colour of the active theme.")]
            pub fn $field() -> Color32 {
                palette().$field
            }
        )*
    };
}

palette! {
    BG_DEEP, bg_deep, "bg-deep";
    BG_BASE, bg_base, "bg-base";
    BG_PANEL, bg_panel, "bg-panel";
    BG_RAISED, bg_raised, "bg-raised";
    BG_HOVER, bg_hover, "bg-hover";
    BG_ACTIVE, bg_active, "bg-active";
    SELECT, select, "select";
    SELECT_WEAK, select_weak, "select-weak";
    LINE, line, "line";
    LINE_STRONG, line_strong, "line-strong";
    LINE_CONTROL, line_control, "line-control";
    TEXT_1, text_1, "text-1";
    TEXT_2, text_2, "text-2";
    TEXT_3, text_3, "text-3";
    TEXT_OFF, text_off, "text-off";
    ICON, icon, "icon";
    ACCENT, accent, "accent";
    ACCENT_HOVER, accent_hover, "accent-hover";
    ACCENT_PRESS, accent_press, "accent-press";
    ACCENT_TEXT, accent_text, "accent-text";
    ON_ACCENT, on_accent, "on-accent";
    FOCUS, focus, "focus";
    HIGHLIGHT, highlight, "highlight";
    ERROR, error, "error";
    ERROR_BG, error_bg, "error-bg";
    WARNING, warning, "warning";
    WARNING_BG, warning_bg, "warning-bg";
    SUCCESS, success, "success";
    SUCCESS_BG, success_bg, "success-bg";
    INFO, info, "info";
    INFO_BG, info_bg, "info-bg";
    DANGER, danger, "danger";
    DANGER_HOVER, danger_hover, "danger-hover";
    ON_DANGER, on_danger, "on-danger";
    ART_SCRIM, art_scrim, "art-scrim";
    EDGE_LOW, edge_low, "edge-low";
    KEYCAP_ON_ACCENT, keycap_on_accent, "keycap-on-accent";
    BACKDROP, backdrop, "backdrop";
}

/// The palettes of all themes, read once from the token file (Graphite everywhere if the file
/// cannot be parsed, which a test rules out).
fn palettes() -> &'static [Palette; 3] {
    static PALETTES: OnceLock<[Palette; 3]> = OnceLock::new();
    PALETTES.get_or_init(
        || match metallic_tokens::parse(metallic_tokens::TOKENS_JSON) {
            Ok(set) => ThemeId::ALL.map(|t| Palette::from_tokens(&set, t)),
            Err(e) => {
                log::warn!("design tokens unreadable, using Graphite everywhere: {e}");
                [Palette::GRAPHITE; 3]
            }
        },
    )
}

/// The colours of `theme`.
pub fn palette_of(theme: ThemeId) -> &'static Palette {
    &palettes()[theme.index()]
}

thread_local! {
    // egui draws on one thread, so the active look lives here; a test that switches theme
    // cannot disturb another test.
    static ACTIVE: Cell<ThemeId> = const { Cell::new(ThemeId::Graphite) };
    static REDUCED: Cell<bool> = const { Cell::new(false) };
}

/// The theme [`apply`] last installed on this thread.
pub fn active() -> ThemeId {
    ACTIVE.with(Cell::get)
}

/// The colours of the active theme.
pub fn palette() -> &'static Palette {
    palette_of(active())
}

/// True while animations are turned off or shortened (Settings > Reduce motion).
pub fn reduce_motion() -> bool {
    REDUCED.with(Cell::get)
}

/// What the `$reduced-motion` block of the token file says `name` (e.g. `t-fast`) becomes, in
/// seconds; 0 when the file has no entry.
fn reduced_seconds(name: &str) -> f32 {
    static REDUCED_TIMES: OnceLock<Vec<(String, f32)>> = OnceLock::new();
    let times = REDUCED_TIMES.get_or_init(|| {
        let Ok(set) = metallic_tokens::parse(metallic_tokens::TOKENS_JSON) else {
            return Vec::new();
        };
        set.reduced_motion()
            .iter()
            .filter_map(|(n, v)| {
                Some((
                    n.clone(),
                    v.strip_suffix("ms")?.trim().parse::<f32>().ok()? / 1000.0,
                ))
            })
            .collect()
    });
    times
        .iter()
        .find(|(n, _)| n == name)
        .map_or(0.0, |&(_, s)| s)
}

fn token_motion(normal: f32, name: &str) -> f32 {
    if reduce_motion() {
        reduced_seconds(name)
    } else {
        normal
    }
}

/// A duration of Studio's own (not a token): zero while motion is reduced.
pub fn motion(normal: f32) -> f32 {
    if reduce_motion() { 0.0 } else { normal }
}

/// Stage swap, panel collapse, progress fill (`t-panel`, seconds).
pub fn t_panel() -> f32 {
    token_motion(tk::T_PANEL, "t-panel")
}
/// Splash in, splash to start screen (`t-splash`, seconds).
pub fn t_splash() -> f32 {
    token_motion(tk::T_SPLASH, "t-splash")
}
/// Hover and press fills (`t-fast`, seconds).
pub fn t_fast() -> f32 {
    token_motion(tk::T_FAST, "t-fast")
}
/// Menus, focus ring, toast exit (`t-base`, seconds).
pub fn t_base() -> f32 {
    token_motion(tk::T_BASE, "t-base")
}
/// Toast enter (`t-slow`, seconds).
pub fn t_slow() -> f32 {
    token_motion(tk::T_SLOW, "t-slow")
}

/// Titanium: the light metal plate behind the emblem on the splash (`g-200`).
pub const TITANIUM: Color32 = rgba(tk::G_200);

/// Keycaps, thumbnails.
pub const R_XS: u8 = tk::R_XS as u8;
/// Buttons, fields.
pub const R_SM: u8 = tk::R_SM as u8;
/// Cards, toasts, menus.
pub const R_MD: u8 = tk::R_MD as u8;
/// Stage, splash, dialogs.
pub const R_LG: u8 = tk::R_LG as u8;

/// The chamfer on the primary button.
pub const CUT_SM: f32 = tk::CUT_SM;
/// The chamfer on a selected card.
pub const CUT_MD: f32 = tk::CUT_MD;
/// The chamfer on the stage, the splash and dialogs.
pub const CUT_LG: f32 = tk::CUT_LG;

/// Tree, property and list rows.
pub const ROW_HEIGHT: f32 = tk::H_ROW;
/// Secondary buttons and inputs.
pub const CONTROL_HEIGHT: f32 = tk::H_CONTROL;
/// Icon box inside a button or row.
pub const ICON_BOX: f32 = 16.0;
/// Docked panel header and panel tab strip.
pub const HEADER_HEIGHT: f32 = tk::H_CONTROL;
/// Status bar height.
pub const STATUS_HEIGHT: f32 = tk::H_ROW;
/// Primary button, toolbar.
pub const CONTROL_HEIGHT_LG: f32 = tk::H_CONTROL_LG;

/// Floating surfaces: menus, popovers, toasts, tooltips (elevation 3).
pub const SHADOW_POP: Shadow = shadow(tk::SHADOW_POP);
/// Dialogs and the splash (elevation 4).
pub const SHADOW_MODAL: Shadow = shadow(tk::SHADOW_MODAL);

/// Archivo's line height (ascender minus descender, no line gap) as a share of the font size.
const ARCHIVO_LINE: f32 = 1.088;

fn state(fill: Color32, border: Stroke, text: Color32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: border,
        corner_radius: CornerRadius::same(R_SM),
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    }
}

/// The visuals of the active theme.
pub fn visuals() -> Visuals {
    let mut v = if active().is_dark() {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.override_text_color = None;
    v.weak_text_color = Some(text_3());
    v.hyperlink_color = accent_text();
    v.panel_fill = bg_base();
    v.faint_bg_color = bg_panel();
    v.extreme_bg_color = bg_deep();
    v.text_edit_bg_color = Some(bg_deep());
    v.code_bg_color = bg_deep();
    v.warn_fg_color = warning();
    v.error_fg_color = error();
    // Dialogs (level 4) and menus, popups, tooltips (level 3).
    v.window_fill = bg_panel();
    // Disabled widgets are drawn in `text-off` on a flat plate (see [`add_enabled`]), not faded.
    v.disabled_alpha = 1.0;
    v.window_stroke = Stroke::new(1.0, highlight());
    v.window_shadow = SHADOW_MODAL;
    v.window_corner_radius = CornerRadius::same(R_LG);
    v.menu_corner_radius = CornerRadius::same(R_MD);
    v.popup_shadow = SHADOW_POP;
    // Selected rows and text: `select` with `text-1`; a focused input is outlined in `focus`.
    v.selection.bg_fill = select();
    v.selection.stroke = Stroke::new(2.0, focus());
    v.text_cursor = TextCursorStyle {
        stroke: Stroke::new(2.0, text_1()),
        ..v.text_cursor
    };

    // Secondary buttons and inputs share egui's widget states: raised fill (inputs keep
    // `bg-deep`), a `line-control` border at rest that brightens to `text-3` under the pointer.
    let w = &mut v.widgets;
    w.noninteractive = state(bg_panel(), Stroke::new(1.0, line()), text_1());
    w.inactive = state(bg_raised(), Stroke::new(1.0, line_control()), text_1());
    w.hovered = state(bg_hover(), Stroke::new(1.0, text_3()), text_1());
    w.active = state(bg_active(), Stroke::new(1.0, text_3()), text_1());
    w.open = state(bg_active(), Stroke::new(1.0, text_3()), text_1());
    v
}

/// Like `ui.add_enabled`, but a disabled widget is drawn with `text-off` on `bg-panel` instead of
/// egui's alpha fade. Use it for egui's own widgets; Studio's `Button` already does this itself.
pub fn add_enabled(ui: &mut egui::Ui, enabled: bool, widget: impl egui::Widget) -> egui::Response {
    if enabled {
        return ui.add(widget);
    }
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        for state in [&mut w.noninteractive, &mut w.inactive] {
            *state = self::state(bg_panel(), Stroke::new(1.0, line()), text_off());
        }
        ui.add_enabled(false, widget)
    })
    .inner
}

/// Installs the Metallic theme `theme` (fonts, colours, spacing, text sizes) into `ctx`. Safe to
/// call again to switch theme: the next frame is drawn in the new colours.
pub fn apply(ctx: &egui::Context, theme: ThemeId) {
    ACTIVE.with(|a| a.set(theme));
    // The fonts are the same in every theme; rebuilding the atlas on each switch would flicker.
    let installed = ctx.data(|d| d.get_temp::<FontsInstalled>(egui::Id::NULL).is_some());
    if !installed {
        fonts::install(ctx);
        ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, FontsInstalled));
    }
    // Studio paints its own colours in both of egui's theme slots, so the system theme never
    // swaps them behind our back.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals());
    ctx.set_visuals_of(egui::Theme::Light, visuals());
    let animation_time = if reduce_motion() { 0.0 } else { 1.0 / 12.0 };
    ctx.all_styles_mut(|style| {
        style.animation_time = animation_time;
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(tk::S4, tk::S4);
        // Text plus this padding makes a control `h-control` tall.
        let pad_y = ((CONTROL_HEIGHT - tk::T_BODY_SIZE * ARCHIVO_LINE) / 2.0).round();
        s.button_padding = egui::vec2(tk::S5, pad_y);
        s.interact_size.y = ROW_HEIGHT;
        s.window_margin = Margin::same(tk::S5 as i8);
        s.menu_margin = Margin::same(tk::S3 as i8);
        s.icon_width = tk::S6;
        s.icon_width_inner = tk::S4;
        s.icon_spacing = tk::S4;
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(tk::T_SMALL_SIZE)),
            (TextStyle::Body, FontId::proportional(tk::T_BODY_SIZE)),
            (TextStyle::Button, fonts::strong()),
            (TextStyle::Heading, fonts::title()),
            (
                TextStyle::Monospace,
                FontId::new(tk::T_MONO_SIZE, FontFamily::Monospace),
            ),
        ]
        .into();
    });
}

/// Installs the theme the creator saved in Studio Settings (Graphite when none is saved or the
/// file cannot be read), resolving "System" against the theme egui reports. For tools other than
/// Studio, which keep no settings of their own. Returns the theme installed.
pub fn apply_saved(ctx: &egui::Context) -> ThemeId {
    let id = saved_theme_choice().resolve(ctx.system_theme());
    apply(ctx, id);
    id
}

/// Marks that [`fonts::install`] has run on a context.
#[derive(Clone, Copy)]
struct FontsInstalled;

/// Turns reduced motion on or off. Takes effect with the next [`apply`] (the animation time of
/// egui's own widgets) and at once for Studio's widgets, which read it every frame.
pub fn set_reduce_motion(on: bool) {
    REDUCED.with(|r| r.set(on));
}

/// WCAG relative luminance of an sRGB colour.
fn luminance(c: Color32) -> f64 {
    let lin = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio between two colours (1.0 to 21.0).
pub fn contrast_ratio(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_text_contrast() {
        for text in [
            text_1(),
            text_2(),
            accent(),
            accent_text(),
            error(),
            warning(),
            success(),
            info(),
        ] {
            for surface in [bg_base(), bg_panel(), bg_deep(), bg_raised()] {
                let ratio = contrast_ratio(text, surface);
                assert!(
                    ratio >= 4.5,
                    "{text:?} on {surface:?} has contrast {ratio:.2}"
                );
            }
        }
        assert!(contrast_ratio(text_3(), bg_raised()) >= 4.5);
        assert!(contrast_ratio(on_accent(), accent()) >= 4.5);
        assert!(contrast_ratio(text_1(), select()) >= 4.5);
    }

    #[test]
    fn focus_ring_and_input_borders_are_visible() {
        // WCAG 1.4.11: focus indicators and control borders need 3:1 against what surrounds them.
        for surface in [bg_base(), bg_panel(), bg_deep(), bg_hover()] {
            assert!(contrast_ratio(focus(), surface) >= 3.0, "{surface:?}");
        }
        for surface in [bg_panel(), bg_deep()] {
            assert!(
                contrast_ratio(line_control(), surface) >= 3.0,
                "{surface:?}"
            );
        }
    }

    #[test]
    fn deep_colour_stays_near_the_icon_background() {
        // The icon's background is (20, 18, 25); DEEP must stay within 8 per channel of it.
        let icon_bg = [20i16, 18, 25];
        for (got, want) in [bg_deep().r(), bg_deep().g(), bg_deep().b()]
            .into_iter()
            .zip(icon_bg)
        {
            assert!((i16::from(got) - want).abs() <= 8);
        }
    }

    #[test]
    fn control_height_comes_from_line_height_and_padding() {
        // `apply` sizes the button padding from Archivo's line height so that a body-size button
        // is exactly `h-control` (32 px) tall; a change to either constant must not move it.
        assert_eq!(CONTROL_HEIGHT, 32.0);
        let ctx = egui::Context::default();
        apply(&ctx, ThemeId::Graphite);
        let style = ctx.global_style();
        let text = tk::T_BODY_SIZE * ARCHIVO_LINE;
        let height = text + 2.0 * style.spacing.button_padding.y;
        assert!(
            (height - CONTROL_HEIGHT).abs() <= 1.0,
            "button is {height} px tall, expected {CONTROL_HEIGHT}"
        );
    }

    /// The colour the text of a disabled or enabled `egui::Button` is drawn in.
    fn button_text_colour(enabled: bool) -> Color32 {
        let ctx = egui::Context::default();
        apply(&ctx, ThemeId::Graphite);
        let mut found = None;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            add_enabled(ui, enabled, egui::Button::new("Cut"));
        });
        out.textures_delta.clear();
        for clipped in out.shapes {
            let mut stack = vec![clipped.shape];
            while let Some(shape) = stack.pop() {
                match shape {
                    egui::Shape::Vec(inner) => stack.extend(inner),
                    egui::Shape::Text(t) => {
                        found = Some(t.override_text_color.unwrap_or(t.fallback_color));
                    }
                    _ => {}
                }
            }
        }
        found.expect("the button drew its label")
    }

    #[test]
    fn disabled_widgets_use_text_off_not_a_fade() {
        assert_eq!(visuals().disabled_alpha, 1.0);
        assert_eq!(button_text_colour(false), text_off());
        assert_eq!(button_text_colour(true), text_1());
    }

    #[test]
    fn theme_ids_match_the_token_file() {
        let set = metallic_tokens::parse(metallic_tokens::TOKENS_JSON).expect("tokens parse");
        let ours: Vec<&str> = ThemeId::ALL.iter().map(|t| t.as_str()).collect();
        assert_eq!(set.theme_ids(), ours);
        for t in ThemeId::ALL {
            assert_eq!(ThemeId::parse(t.as_str()), Some(t));
            assert_eq!(ThemeId::parse(&t.label().to_uppercase()), Some(t));
        }
        assert_eq!(ThemeId::parse("neon"), None);
        assert_eq!(ThemeId::default(), ThemeId::Graphite);
    }

    #[test]
    fn palettes_read_each_themes_tokens() {
        let graphite = palette_of(ThemeId::Graphite);
        assert_eq!(graphite.bg_base, BG_BASE);
        assert_eq!(graphite.accent, ACCENT);
        let titanium = palette_of(ThemeId::Titanium);
        assert_eq!(titanium.bg_base, Color32::from_rgb(0x24, 0x22, 0x2b));
        let light = palette_of(ThemeId::Light);
        assert_eq!(light.bg_deep, Color32::WHITE);
        assert_eq!(light.text_1, Color32::from_rgb(0x16, 0x15, 0x1b));
        assert_ne!(graphite, titanium);
        assert_ne!(graphite, light);
    }

    #[test]
    fn text_is_readable_in_every_theme() {
        for id in ThemeId::ALL {
            let p = palette_of(id);
            for text in [p.text_1, p.text_2, p.accent_text, p.error, p.success] {
                for surface in [p.bg_base, p.bg_panel, p.bg_deep] {
                    let ratio = contrast_ratio(text, surface);
                    assert!(
                        ratio >= 4.5,
                        "{id:?}: {text:?} on {surface:?} is {ratio:.2}"
                    );
                }
            }
            assert!(contrast_ratio(p.on_accent, p.accent) >= 4.5, "{id:?}");
        }
    }

    #[test]
    fn apply_repaints_in_the_chosen_theme_and_back() {
        let ctx = egui::Context::default();
        apply(&ctx, ThemeId::Light);
        assert_eq!(active(), ThemeId::Light);
        assert_eq!(bg_base(), palette_of(ThemeId::Light).bg_base);
        let v = ctx.global_style().visuals.clone();
        assert!(!v.dark_mode);
        assert_eq!(v.panel_fill, palette_of(ThemeId::Light).bg_base);
        assert_eq!(
            v.widgets.inactive.bg_fill,
            palette_of(ThemeId::Light).bg_raised
        );
        apply(&ctx, ThemeId::Titanium);
        let v = ctx.global_style().visuals.clone();
        assert!(v.dark_mode);
        assert_eq!(v.panel_fill, palette_of(ThemeId::Titanium).bg_base);
        apply(&ctx, ThemeId::Graphite);
        assert_eq!(ctx.global_style().visuals.panel_fill, BG_BASE);
        assert_eq!(accent(), ACCENT);
    }

    #[test]
    fn reduced_motion_uses_the_token_files_short_times() {
        assert!(!reduce_motion());
        assert_eq!(t_fast(), tk::T_FAST);
        assert_eq!(motion(0.09), 0.09);
        set_reduce_motion(true);
        assert_eq!(t_fast(), 0.0);
        assert_eq!(t_base(), 0.0);
        assert!((t_slow() - 0.08).abs() < 1e-6);
        assert!((t_panel() - 0.08).abs() < 1e-6);
        assert!((t_splash() - 0.08).abs() < 1e-6);
        assert_eq!(motion(0.09), 0.0);
        let ctx = egui::Context::default();
        apply(&ctx, ThemeId::Graphite);
        assert_eq!(ctx.global_style().animation_time, 0.0);
        set_reduce_motion(false);
        apply(&ctx, ThemeId::Graphite);
        assert!(ctx.global_style().animation_time > 0.0);
    }

    #[test]
    fn shadows_and_radii_come_from_the_tokens() {
        assert_eq!(SHADOW_POP.offset, [0, 8]);
        assert_eq!(SHADOW_POP.blur, 24);
        assert_eq!(SHADOW_MODAL.blur, 48);
        assert_eq!((R_XS, R_SM, R_MD, R_LG), (3, 4, 6, 10));
        let v = visuals();
        assert_eq!(v.panel_fill, bg_base());
        assert_eq!(v.widgets.inactive.corner_radius, CornerRadius::same(R_SM));
    }
}
