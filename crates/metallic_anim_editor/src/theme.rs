//! Colour, type and size tokens for the animation editor, from the Studio UI design
//! (`docs/research/2026-10-30-studio-ui-design.md`, section 5.13, PR #127).
//!
//! Graphite surfaces from `#111015` to `#35323e`, text `#e6e4ea`, silver icons, one restrained
//! violet accent `#9d8be0` used only for selection, focus, the primary button and the playhead,
//! with dark text on it (white on the accent fails contrast). Radii 3/5/8 px; shadows only on
//! floating surfaces. No glow, no gradients on chrome.
//!
//! Fonts: egui's `default_fonts` feature stays off (its licence fails `deny.toml`), so
//! [`install_system_fonts`] loads fonts already installed on the computer at runtime (IBM Plex
//! if present, else Segoe UI / Consolas on Windows, the platform fonts elsewhere). Nothing is
//! redistributed.

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke,
    TextStyle, Visuals,
};
use std::sync::Arc;

/// Editor colour tokens (names follow the design doc's token table).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// `bg.deep` #111015: canvases, inputs, title bar.
    pub deep: Color32,
    /// `bg.base` #16151b: window background.
    pub base: Color32,
    /// `bg.panel` #1c1b22: docked panels, tab bars.
    pub panel: Color32,
    /// `bg.raised` #24222b: buttons, chips, popovers.
    pub raised: Color32,
    /// `bg.hover` #2c2a34.
    pub hover: Color32,
    /// `bg.active` #35323e: pressed, selected segment.
    pub active: Color32,
    /// `select` #3a3354: selected rows.
    pub select: Color32,
    /// `select.weak` #2a2638: rows related to the selection, drop zones.
    pub select_weak: Color32,
    /// `line` #2b2933: separators and grid lines (decoration only).
    pub line: Color32,
    /// `line.strong` #3b3845: popover borders, emphasised separators.
    pub line_strong: Color32,
    /// Grid lines fainter than `line`, for dense canvases.
    pub grid: Color32,
    /// `text` #e6e4ea: primary text.
    pub text: Color32,
    /// `text.2` #aeaab6: labels, inactive tabs.
    pub text_dim: Color32,
    /// `text.3` #8e8a97: meta, hints (not on hover/active fills).
    pub text_meta: Color32,
    /// `icon` #bdbac4: silver icons, unselected keys, bones.
    pub silver: Color32,
    /// `accent` #9d8be0: selection, focus, primary button, playhead.
    pub accent: Color32,
    /// `accent.hover` #ab9be6.
    pub accent_hover: Color32,
    /// `on.accent` #111015: text and icons on accent fills.
    pub on_accent: Color32,
    /// Accent at low opacity for box-select fills.
    pub accent_soft: Color32,
    /// `error` #ec7a72: errors and the Record keys state.
    pub error: Color32,
    /// `warning` #e0ad55: event markers.
    pub warning: Color32,
    /// `axis.x` / `axis.y` / `axis.z`: X/Y/Z (or R/G/B) curves, always shown with the letter.
    pub axis: [Color32; 3],
}

impl Default for Theme {
    fn default() -> Self {
        Self::METALLIC
    }
}

/// Corner radius `r.sm` (rows, fields, chips, keys).
pub const R_SM: u8 = 3;
/// Corner radius `r.md` (buttons, segments, menus).
pub const R_MD: u8 = 5;
/// Corner radius `r.lg` (popovers, floating windows).
pub const R_LG: u8 = 8;

impl Theme {
    /// The Metallic Studio dark theme.
    pub const METALLIC: Theme = Theme {
        deep: Color32::from_rgb(0x11, 0x10, 0x15),
        base: Color32::from_rgb(0x16, 0x15, 0x1b),
        panel: Color32::from_rgb(0x1c, 0x1b, 0x22),
        raised: Color32::from_rgb(0x24, 0x22, 0x2b),
        hover: Color32::from_rgb(0x2c, 0x2a, 0x34),
        active: Color32::from_rgb(0x35, 0x32, 0x3e),
        select: Color32::from_rgb(0x3a, 0x33, 0x54),
        select_weak: Color32::from_rgb(0x2a, 0x26, 0x38),
        line: Color32::from_rgb(0x2b, 0x29, 0x33),
        line_strong: Color32::from_rgb(0x3b, 0x38, 0x45),
        grid: Color32::from_rgb(0x1d, 0x1c, 0x23),
        text: Color32::from_rgb(0xe6, 0xe4, 0xea),
        text_dim: Color32::from_rgb(0xae, 0xaa, 0xb6),
        text_meta: Color32::from_rgb(0x8e, 0x8a, 0x97),
        silver: Color32::from_rgb(0xbd, 0xba, 0xc4),
        accent: Color32::from_rgb(0x9d, 0x8b, 0xe0),
        accent_hover: Color32::from_rgb(0xab, 0x9b, 0xe6),
        on_accent: Color32::from_rgb(0x11, 0x10, 0x15),
        accent_soft: Color32::from_rgba_premultiplied(0x9d / 5, 0x8b / 5, 0xe0 / 5, 51),
        error: Color32::from_rgb(0xec, 0x7a, 0x72),
        warning: Color32::from_rgb(0xe0, 0xad, 0x55),
        axis: [
            Color32::from_rgb(0xe5, 0x67, 0x5e),
            Color32::from_rgb(0x79, 0xc4, 0x6a),
            Color32::from_rgb(0x5d, 0x95, 0xea),
        ],
    };

    /// A 1 px hairline in the `line` colour.
    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.line)
    }

    /// Applies the tokens to egui's style: colours, type sizes, spacing, radii, motion.
    pub fn apply(&self, ctx: &egui::Context) {
        let t = *self;
        ctx.all_styles_mut(move |style| {
            let v: &mut Visuals = &mut style.visuals;
            *v = Visuals::dark();
            v.panel_fill = t.panel;
            v.window_fill = t.raised;
            v.extreme_bg_color = t.deep;
            v.faint_bg_color = t.raised;
            v.code_bg_color = t.deep;
            v.window_stroke = Stroke::new(1.0, t.line_strong);
            v.override_text_color = None;
            v.weak_text_color = Some(t.text_meta);
            v.hyperlink_color = Color32::from_rgb(0xb4, 0xa6, 0xec);
            v.error_fg_color = t.error;
            v.warn_fg_color = t.warning;
            v.selection.bg_fill = t.select;
            v.selection.stroke = Stroke::new(1.0, t.accent);
            let pop = Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(115),
            };
            v.window_shadow = pop;
            v.popup_shadow = pop;
            v.window_corner_radius = CornerRadius::same(R_LG);
            v.menu_corner_radius = CornerRadius::same(R_MD);
            let r = CornerRadius::same(R_MD);
            let w = &mut v.widgets;
            w.noninteractive.bg_fill = t.panel;
            w.noninteractive.weak_bg_fill = t.panel;
            w.noninteractive.bg_stroke = Stroke::new(1.0, t.line);
            w.noninteractive.fg_stroke = Stroke::new(1.0, t.text_dim);
            w.noninteractive.corner_radius = CornerRadius::same(R_SM);
            w.inactive.bg_fill = t.raised;
            w.inactive.weak_bg_fill = t.raised;
            w.inactive.bg_stroke = Stroke::NONE;
            w.inactive.fg_stroke = Stroke::new(1.0, t.text);
            w.inactive.corner_radius = r;
            w.hovered.bg_fill = t.hover;
            w.hovered.weak_bg_fill = t.hover;
            w.hovered.bg_stroke = Stroke::NONE;
            w.hovered.fg_stroke = Stroke::new(1.0, t.text);
            w.hovered.corner_radius = r;
            w.hovered.expansion = 0.0;
            w.active.bg_fill = t.active;
            w.active.weak_bg_fill = t.active;
            w.active.bg_stroke = Stroke::NONE;
            w.active.fg_stroke = Stroke::new(1.0, t.text);
            w.active.corner_radius = r;
            w.active.expansion = 0.0;
            w.open = w.active;
            style.spacing.item_spacing = egui::vec2(8.0, 4.0);
            style.spacing.button_padding = egui::vec2(10.0, 4.0);
            style.spacing.interact_size = egui::vec2(40.0, 26.0);
            style.spacing.indent = 18.0;
            style.spacing.menu_margin = egui::Margin::same(6);
            style.spacing.window_margin = egui::Margin::same(12);
            style.animation_time = 0.14;
            style.text_styles = [
                (TextStyle::Small, FontId::proportional(11.0)),
                (TextStyle::Body, FontId::proportional(13.0)),
                (TextStyle::Button, FontId::proportional(13.0)),
                (TextStyle::Monospace, FontId::monospace(12.0)),
                (TextStyle::Heading, FontId::proportional(15.0)),
            ]
            .into();
        });
    }
}

/// Loads installed fonts into egui (no font files are bundled). Prefers IBM Plex Sans / Mono when
/// installed, then the platform UI fonts. Returns how many font files were loaded; with `0`,
/// egui has no glyphs and text will not show, so hosts should report it.
pub fn install_system_fonts(ctx: &egui::Context) -> usize {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if cfg!(windows) {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned());
        dirs.push(std::path::Path::new(&windir).join("Fonts"));
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            dirs.push(
                std::path::Path::new(&local)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
    } else if cfg!(target_os = "macos") {
        dirs.push("/System/Library/Fonts".into());
        dirs.push("/Library/Fonts".into());
    } else {
        dirs.push("/usr/share/fonts/truetype/dejavu".into());
        dirs.push("/usr/share/fonts/TTF".into());
        dirs.push("/usr/share/fonts/dejavu".into());
    }
    let sans = [
        "IBMPlexSans-Regular.ttf",
        "segoeui.ttf",
        "SFNS.ttf",
        "Helvetica.ttc",
        "DejaVuSans.ttf",
    ];
    let mono = [
        "IBMPlexMono-Regular.ttf",
        "CascadiaMono.ttf",
        "consola.ttf",
        "Menlo.ttc",
        "DejaVuSansMono.ttf",
    ];
    // Symbol fallback so glyphs like the middle dot and arrows always render.
    let symbols = ["seguisym.ttf", "Apple Symbols.ttf", "DejaVuSans.ttf"];
    let find = |names: &[&str]| -> Option<(String, Vec<u8>)> {
        names.iter().find_map(|n| {
            dirs.iter().find_map(|d| {
                std::fs::read(d.join(n))
                    .ok()
                    .map(|bytes| ((*n).to_owned(), bytes))
            })
        })
    };
    let mut defs = FontDefinitions::empty();
    let mut loaded = 0;
    for (names, family) in [
        (&sans[..], FontFamily::Proportional),
        (&mono[..], FontFamily::Monospace),
        (&symbols[..], FontFamily::Proportional),
    ] {
        if let Some((name, bytes)) = find(names) {
            if !defs.font_data.contains_key(&name) {
                defs.font_data
                    .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
                loaded += 1;
            }
            defs.families.entry(family).or_default().push(name);
        }
    }
    // Each family falls back to the other so neither is ever empty.
    let prop = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let monos = defs
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    defs.families
        .entry(FontFamily::Monospace)
        .or_default()
        .extend(prop.iter().cloned());
    defs.families
        .entry(FontFamily::Proportional)
        .or_default()
        .extend(monos);
    if loaded > 0 {
        ctx.set_fonts(defs);
    }
    loaded
}

/// Relative luminance (WCAG) of a colour.
pub fn luminance(c: Color32) -> f32 {
    let ch = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
}

/// WCAG contrast ratio between two colours (1 to 21).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}
