//! The design tokens: every colour, size and duration Studio's interface uses (ADR 0009, Theme).
//!
//! The reference is the Metallic website (`Metallic-Club/website`: `src/design.css` and `src/themes.js`), so Studio
//! and the site look like one product. Nothing outside this module names a colour or a size of its own.

use egui::Color32;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Every neutral colour role of one mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The window's ground (`--bg`).
    pub bg: Color32,
    /// Title and menu bars (`--header`).
    pub header: Color32,
    /// Docked panels and the side rail (`--sidebar`).
    pub panel: Color32,
    /// Raised things: cards, secondary buttons, menus (`--surface`).
    pub surface: Color32,
    /// A surface under the pointer (`--surface-hover`).
    pub surface_hover: Color32,
    /// Text fields (`--input`).
    pub input: Color32,
    /// 1 px borders and dividers (`--border`).
    pub border: Color32,
    /// Body text (`--text`).
    pub text: Color32,
    /// Secondary text (`--muted`).
    pub muted: Color32,
    /// Hints, metadata, and borders under the pointer (`--subtle`).
    pub subtle: Color32,
    /// The primary button's fill: neutral, never the accent (`--action-bg`).
    pub action: Color32,
    /// The primary button's text (`--action-text`).
    pub action_text: Color32,
    /// The primary button under the pointer (`--action-hover`).
    pub action_hover: Color32,
}

/// The dark mode, Studio's default (and always the splash's).
pub const DARK: Palette = Palette {
    bg: hex(0x161616),
    header: hex(0x1D1D1D),
    panel: hex(0x1B1B1B),
    surface: hex(0x262626),
    surface_hover: hex(0x333333),
    input: hex(0x1C1C1C),
    border: hex(0x414141),
    text: hex(0xF6F6F6),
    muted: hex(0xC1C1C1),
    subtle: hex(0xADADAD),
    action: hex(0xEBEBEB),
    action_text: hex(0x242424),
    action_hover: hex(0xFFFFFF),
};

/// The light mode.
pub const LIGHT: Palette = Palette {
    bg: hex(0xF5F5F5),
    header: hex(0xFFFFFF),
    panel: hex(0xFFFFFF),
    surface: hex(0xFFFFFF),
    surface_hover: hex(0xEFEFEF),
    input: hex(0xF7F7F7),
    border: hex(0xCFCFCF),
    text: hex(0x262626),
    muted: hex(0x565656),
    subtle: hex(0x646464),
    action: hex(0x303030),
    action_text: hex(0xFFFFFF),
    action_hover: hex(0x474747),
};

impl Palette {
    /// The palette with stronger contrast: secondary text becomes body text, and borders darken (dark) or lighten
    /// (light) to read against every ground.
    pub const fn high_contrast(self, dark: bool) -> Self {
        Self {
            muted: self.text,
            subtle: self.text,
            border: if dark { hex(0x898989) } else { hex(0x6D6D6D) },
            ..self
        }
    }

    /// The striped rows' colours (the Properties panel, #230), derived from this palette's surfaces so every mode
    /// has them: a shade a little darker than the panel (towards the window's ground), one a little brighter (towards
    /// a raised surface), and the border at low alpha for the 1 px lines between rows and columns.
    pub const fn rows(self) -> RowShades {
        RowShades {
            darker: mix(self.panel, self.bg, 0.75),
            brighter: mix(self.panel, self.surface, 0.5),
            line: Color32::from_rgba_unmultiplied_const(
                self.border.r(),
                self.border.g(),
                self.border.b(),
                ROW_LINE_ALPHA,
            ),
        }
    }
}

/// The opacity of the lines between striped rows: faint, so they separate without boxing.
pub const ROW_LINE_ALPHA: u8 = 0x66;

/// The striped rows' colours of one palette ([`Palette::rows`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowShades {
    /// Every other row, starting with the first.
    pub darker: Color32,
    /// The rows between.
    pub brighter: Color32,
    /// The 1 px lines between rows and between the name and value columns: the border at [`ROW_LINE_ALPHA`].
    pub line: Color32,
}

impl RowShades {
    /// The fill of row `index` (from 0) of a striped list.
    pub const fn fill(self, index: usize) -> Color32 {
        if index.is_multiple_of(2) {
            self.darker
        } else {
            self.brighter
        }
    }
}

/// `from` moved `t` (0 to 1) of the way to `to`, channel by channel. Both opaque.
const fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    const fn channel(a: u8, b: u8, t: f32) -> u8 {
        let (a, b) = (a as f32, b as f32);
        (a + (b - a) * t + 0.5) as u8
    }
    Color32::from_rgb(
        channel(from.r(), to.r(), t),
        channel(from.g(), to.g(), t),
        channel(from.b(), to.b(), t),
    )
}

/// The "play" button (run the game): green in every mode.
pub mod play {
    use super::{Color32, hex};

    /// Its fill.
    pub const FILL: Color32 = hex(0x39B878);
    /// Its fill under the pointer.
    pub const HOVER: Color32 = hex(0x53D08F);
    /// Its text.
    pub const TEXT: Color32 = hex(0x0D281A);
}

/// The text on an accent fill (selected text), the same for every accent.
pub const ACCENT_INK: Color32 = hex(0x24160D);

/// The accent colour, chosen by the user. Used sparingly: focus outlines, the active tab's and the selected item's
/// bars, and text selection. Never a button's fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Accent {
    /// Metallic orange, the default.
    #[default]
    Orange,
    /// Ocean blue.
    Blue,
    /// Electric violet.
    Violet,
    /// Fresh mint.
    Mint,
    /// Rose.
    Rose,
}

/// One accent's colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccentColors {
    /// On the dark mode, and as a fill in both modes (text selection).
    pub color: Color32,
    /// Under the pointer.
    pub hover: Color32,
    /// On the light mode, where the bright colour would not read.
    pub light: Color32,
}

impl Accent {
    /// Every accent, in the order Settings lists them.
    pub const ALL: [Accent; 5] = [Accent::Orange, Accent::Blue, Accent::Violet, Accent::Mint, Accent::Rose];

    /// The id the settings store.
    pub fn as_str(self) -> &'static str {
        match self {
            Accent::Orange => "orange",
            Accent::Blue => "blue",
            Accent::Violet => "violet",
            Accent::Mint => "mint",
            Accent::Rose => "rose",
        }
    }

    /// Reads an id in any letter case; `None` for an unknown one.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|accent| accent.as_str().eq_ignore_ascii_case(id.trim()))
    }

    /// The name people see.
    pub fn label(self) -> &'static str {
        match self {
            Accent::Orange => "Metallic orange",
            Accent::Blue => "Ocean blue",
            Accent::Violet => "Electric violet",
            Accent::Mint => "Fresh mint",
            Accent::Rose => "Rose",
        }
    }

    /// Its colours.
    pub const fn colors(self) -> AccentColors {
        let (color, hover, light) = match self {
            Accent::Orange => (0xFF9B54, 0xFFB47F, 0xA34506),
            Accent::Blue => (0x82B5FF, 0xA9CDFF, 0x225DA8),
            Accent::Violet => (0xBA9AFF, 0xD0BAFF, 0x7141AE),
            Accent::Mint => (0x72D9B1, 0xA0EBCE, 0x1D7152),
            Accent::Rose => (0xFF9BB5, 0xFFBED0, 0xA13959),
        };
        AccentColors {
            color: hex(color),
            hover: hex(hover),
            light: hex(light),
        }
    }

    /// The colour it marks things with on the dark or the light mode.
    pub const fn on(self, dark: bool) -> Color32 {
        let colors = self.colors();
        if dark { colors.color } else { colors.light }
    }
}

/// The hues icons are coloured in, one per icon group (`theme::icons::Group`, #231): the accent family's five colours
/// and a neutral. Calm, not neon: on the dark mode each is the accent pulled a quarter of the way towards the muted
/// grey, on the light mode the light accent a fifth of the way; high contrast is stronger: the brighter hover colour on
/// dark, the light accent pulled towards the text on light. Selection and the accent still win where they mark something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hue {
    /// Lights, transform tools.
    Orange,
    /// Services, the scene's panels, the File menu.
    Blue,
    /// Scripts, the log, the View menu.
    Violet,
    /// The world: parts and models; content; running the game.
    Mint,
    /// Cameras, inserting.
    Rose,
    /// Everything else: folders, values, Studio itself, the interface's own icons.
    Neutral,
}

/// How far a dark-mode icon hue is pulled towards the muted grey.
const ICON_CALM_DARK: f32 = 0.25;
/// How far a light-mode icon hue is pulled towards the muted grey.
const ICON_CALM_LIGHT: f32 = 0.2;
/// How far a light-mode, high-contrast icon hue is pulled towards the text colour: darker, so stronger than the calm one.
const ICON_STRONG_LIGHT: f32 = 0.3;

impl Hue {
    /// Every hue.
    pub const ALL: [Hue; 6] = [Hue::Orange, Hue::Blue, Hue::Violet, Hue::Mint, Hue::Rose, Hue::Neutral];

    /// The accent it is drawn from; `None` for the neutral.
    const fn accent(self) -> Option<Accent> {
        match self {
            Hue::Orange => Some(Accent::Orange),
            Hue::Blue => Some(Accent::Blue),
            Hue::Violet => Some(Accent::Violet),
            Hue::Mint => Some(Accent::Mint),
            Hue::Rose => Some(Accent::Rose),
            Hue::Neutral => None,
        }
    }

    /// Its colour on the dark or the light mode, with or without high contrast.
    pub const fn color(self, dark: bool, high_contrast: bool) -> Color32 {
        let palette = if dark { DARK } else { LIGHT };
        let Some(accent) = self.accent() else {
            return if high_contrast { palette.text } else { palette.subtle };
        };
        let c = accent.colors();
        match (dark, high_contrast) {
            (true, false) => mix(c.color, palette.muted, ICON_CALM_DARK),
            (true, true) => c.hover,
            (false, false) => mix(c.light, palette.muted, ICON_CALM_LIGHT),
            (false, true) => mix(c.light, palette.text, ICON_STRONG_LIGHT),
        }
    }
}

/// Spacing on a 4 px grid, in points.
pub mod space {
    /// 2.
    pub const S1: f32 = 2.0;
    /// 4.
    pub const S2: f32 = 4.0;
    /// 6.
    pub const S3: f32 = 6.0;
    /// 8.
    pub const S4: f32 = 8.0;
    /// 12: panel padding.
    pub const S5: f32 = 12.0;
    /// 16.
    pub const S6: f32 = 16.0;
    /// 18: the website's gap between blocks.
    pub const GAP: f32 = 18.0;
    /// 24.
    pub const S8: f32 = 24.0;
    /// 32.
    pub const S9: f32 = 32.0;
    /// 40.
    pub const S10: f32 = 40.0;
    /// 48.
    pub const S11: f32 = 48.0;
    /// The whole scale, smallest first.
    pub const SCALE: [f32; 11] = [S1, S2, S3, S4, S5, S6, GAP, S8, S9, S10, S11];
}

/// Corner radii, in points. Corners are only ever rounded: Studio has no chamfers.
pub mod radius {
    /// Buttons, list items, tabs.
    pub const ITEM: u8 = 8;
    /// Text fields.
    pub const INPUT: u8 = 9;
    /// Panels, cards, the splash.
    pub const PANEL: u8 = 12;
    /// Dialogs.
    pub const DIALOG: u8 = 15;
}

/// Fixed sizes, in points.
pub mod size {
    /// Tree and property rows.
    pub const ROW: f32 = 24.0;
    /// Buttons and fields.
    pub const CONTROL: f32 = 34.0;
    /// Borders.
    pub const BORDER: f32 = 1.0;
    /// The focus outline, the active tab's underline and the selected item's bar.
    pub const ACCENT_BAR: f32 = 3.0;
    /// The selected list item's bar length.
    pub const SELECTED_BAR: f32 = 19.0;
}

/// Type sizes, in points.
pub mod text {
    /// The wordmark (splash only).
    pub const WORDMARK: f32 = 44.0;
    /// The wordmark's letter spacing: tight, as the website's headings.
    pub const WORDMARK_TRACKING: f32 = -1.6;
    /// One per screen: the screen's title.
    pub const DISPLAY: f32 = 30.0;
    /// Its letter spacing.
    pub const DISPLAY_TRACKING: f32 = -0.8;
    /// Dialog and section titles.
    pub const TITLE: f32 = 20.0;
    /// Descriptions and lead text.
    pub const BODY_LG: f32 = 15.0;
    /// Default interface text and controls.
    pub const BODY: f32 = 14.0;
    /// Metadata and help text.
    pub const SMALL: f32 = 12.0;
    /// Uppercase section labels.
    pub const LABEL: f32 = 11.0;
    /// Their letter spacing.
    pub const LABEL_TRACKING: f32 = 1.3;
    /// Code and changing numbers.
    pub const MONO: f32 = 13.0;
}

/// Shadows, only on what floats: menus, popovers and dialogs.
pub mod shadow {
    use egui::{Color32, Shadow};

    /// Menus and popovers: `0 8 24 #0005`.
    pub const MENU: Shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(0x55),
    };
}

/// How long things move, in seconds: short and eased. With reduced motion every one is zero: no fades and no eases
/// (ADR 0009).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// Hover and press fills.
    pub fast: f32,
    /// Menus opening, the focus outline.
    pub base: f32,
    /// Panel swaps, progress fills, the splash fading in.
    pub slow: f32,
}

impl Motion {
    /// The durations, or zeros when `reduced`.
    pub const fn new(reduced: bool) -> Self {
        if reduced {
            Self {
                fast: 0.0,
                base: 0.0,
                slow: 0.0,
            }
        } else {
            Self {
                fast: 0.12,
                base: 0.16,
                slow: 0.2,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2 relative luminance of an opaque colour.
    fn luminance(c: Color32) -> f64 {
        let channel = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn the_palettes_are_the_websites() {
        assert_eq!(DARK.bg, Color32::from_rgb(0x16, 0x16, 0x16));
        assert_eq!((DARK.header, DARK.panel), (hex(0x1D1D1D), hex(0x1B1B1B)));
        assert_eq!((DARK.surface, DARK.surface_hover), (hex(0x262626), hex(0x333333)));
        assert_eq!((DARK.input, DARK.border), (hex(0x1C1C1C), hex(0x414141)));
        assert_eq!(
            (DARK.text, DARK.muted, DARK.subtle),
            (hex(0xF6F6F6), hex(0xC1C1C1), hex(0xADADAD))
        );
        assert_eq!(
            (LIGHT.bg, LIGHT.header, LIGHT.surface),
            (hex(0xF5F5F5), hex(0xFFFFFF), hex(0xFFFFFF))
        );
        assert_eq!((LIGHT.border, LIGHT.text), (hex(0xCFCFCF), hex(0x262626)));
        assert_eq!((LIGHT.muted, LIGHT.subtle), (hex(0x565656), hex(0x646464)));
        assert_eq!(Accent::default(), Accent::Orange);
        assert_eq!(
            Accent::Orange.colors(),
            AccentColors {
                color: hex(0xFF9B54),
                hover: hex(0xFFB47F),
                light: hex(0xA34506),
            }
        );
    }

    #[test]
    fn every_mode_reads() {
        for (palette, dark) in [(DARK, true), (LIGHT, false)] {
            for p in [palette, palette.high_contrast(dark)] {
                // WCAG AA for body text, on every ground text sits on.
                for ground in [p.bg, p.header, p.panel, p.surface, p.surface_hover, p.input] {
                    for ink in [p.text, p.muted, p.subtle] {
                        assert!(contrast(ink, ground) >= 4.5, "dark {dark}: {ink:?} on {ground:?}");
                    }
                }
                assert!(contrast(p.action_text, p.action) >= 4.5);
                assert!(contrast(p.action_text, p.action_hover) >= 4.5);
            }
            // Every accent shows against the ground (3:1, non-text), and its ink reads on its fill.
            for accent in Accent::ALL {
                assert!(
                    contrast(accent.on(dark), palette.bg) >= 3.0,
                    "{accent:?} on dark {dark}"
                );
                assert!(contrast(ACCENT_INK, accent.colors().color) >= 4.5, "{accent:?} ink");
            }
        }
        assert!(contrast(play::TEXT, play::FILL) >= 4.5);
        assert!(contrast(play::TEXT, play::HOVER) >= 4.5);
        // High contrast strengthens the borders.
        assert!(contrast(DARK.high_contrast(true).border, DARK.bg) > contrast(DARK.border, DARK.bg));
        assert!(contrast(LIGHT.high_contrast(false).border, LIGHT.bg) > contrast(LIGHT.border, LIGHT.bg));
        assert!(contrast(DARK.high_contrast(true).border, DARK.bg) >= 3.0);
    }

    /// `over` with `line` (unmultiplied alpha) drawn on it.
    fn blend(line: Color32, over: Color32) -> Color32 {
        let a = f32::from(line.a()) / 255.0;
        let channel = |l: u8, o: u8| (f32::from(o) + (f32::from(l) - f32::from(o)) * a).round() as u8;
        let [r, g, b, _] = line.to_srgba_unmultiplied();
        Color32::from_rgb(channel(r, over.r()), channel(g, over.g()), channel(b, over.b()))
    }

    #[test]
    fn striped_rows_have_two_close_shades_and_faint_lines_in_every_mode() {
        for (palette, dark) in [(DARK, true), (LIGHT, false)] {
            for (p, strong) in [(palette, false), (palette.high_contrast(dark), true)] {
                let mode = format!("dark {dark}, high contrast {strong}");
                let rows = p.rows();
                // Two different shades, the darker one darker, both close to the panel they sit on.
                assert!(luminance(rows.darker) < luminance(rows.brighter), "{mode}: {rows:?}");
                for shade in [rows.darker, rows.brighter] {
                    let near = |a: u8, b: u8| a.abs_diff(b) <= 10;
                    assert!(
                        near(shade.r(), p.panel.r()) && near(shade.g(), p.panel.g()) && near(shade.b(), p.panel.b()),
                        "{mode}: {shade:?} is far from the panel {:?}",
                        p.panel
                    );
                    // Text reads on both (WCAG AA).
                    for ink in [p.text, p.muted, p.subtle] {
                        assert!(contrast(ink, shade) >= 4.5, "{mode}: {ink:?} on {shade:?}");
                    }
                    // The line shows on both, and is fainter than a full border.
                    let drawn = blend(rows.line, shade);
                    assert_ne!(drawn, shade, "{mode}: the line does not show on {shade:?}");
                    assert!(
                        contrast(drawn, shade) < contrast(p.border, shade),
                        "{mode}: the line is not faint"
                    );
                }
                assert_eq!(rows.line.a(), ROW_LINE_ALPHA);
                const { assert!(ROW_LINE_ALPHA < 0x80, "low alpha") };
                // The border's colour (give or take the rounding of premultiplied alpha).
                let [r, g, b, _] = rows.line.to_srgba_unmultiplied();
                for (ours, border) in [(r, p.border.r()), (g, p.border.g()), (b, p.border.b())] {
                    assert!(ours.abs_diff(border) <= 2, "{mode}: {:?} is not the border", rows.line);
                }
                // Rows alternate, starting with the darker.
                let fills: Vec<Color32> = (0..4).map(|i| rows.fill(i)).collect();
                assert_eq!(fills, [rows.darker, rows.brighter, rows.darker, rows.brighter]);
            }
        }
        // The website's dark panel gives these shades.
        assert_eq!(
            (DARK.rows().darker, DARK.rows().brighter),
            (hex(0x171717), hex(0x212121))
        );
        // High contrast keeps the shades and strengthens the lines.
        let strong = DARK.high_contrast(true).rows();
        assert_eq!(strong.darker, DARK.rows().darker);
        assert!(
            contrast(blend(strong.line, strong.darker), strong.darker)
                > contrast(blend(DARK.rows().line, strong.darker), strong.darker)
        );
    }

    #[test]
    fn icon_hues_are_distinct_calm_and_show_on_every_ground() {
        for dark in [true, false] {
            for strong in [false, true] {
                let mode = format!("dark {dark}, high contrast {strong}");
                let p = if dark { DARK } else { LIGHT };
                let p = if strong { p.high_contrast(dark) } else { p };
                let colors: Vec<Color32> = Hue::ALL.iter().map(|hue| hue.color(dark, strong)).collect();
                for (i, a) in colors.iter().enumerate() {
                    for b in &colors[i + 1..] {
                        assert_ne!(a, b, "{mode}: two hues are the same");
                    }
                    // An icon is a graphic: 3:1 against every ground it is drawn on (WCAG 1.4.11).
                    let rows = p.rows();
                    for ground in [p.bg, p.panel, p.surface, p.surface_hover, rows.darker, rows.brighter] {
                        assert!(contrast(*a, ground) >= 3.0, "{mode}: {a:?} on {ground:?}");
                    }
                }
                for hue in Hue::ALL {
                    if strong {
                        // High contrast never weakens a hue.
                        let ground = p.panel;
                        assert!(contrast(hue.color(dark, true), ground) >= contrast(hue.color(dark, false), ground));
                    }
                }
            }
            // Calm: on the dark mode each hue is softer (less saturated) than the accent it comes from.
            if dark {
                let chroma = |c: Color32| c.r().max(c.g()).max(c.b()) - c.r().min(c.g()).min(c.b());
                for (hue, accent) in Hue::ALL.iter().zip(Accent::ALL) {
                    assert!(
                        chroma(hue.color(true, false)) < chroma(accent.colors().color),
                        "{hue:?}"
                    );
                }
            }
        }
        assert_eq!(Hue::Neutral.color(true, false), DARK.subtle);
        assert_eq!(
            Hue::Orange.color(false, true),
            mix(Accent::Orange.colors().light, LIGHT.text, ICON_STRONG_LIGHT)
        );
    }

    #[test]
    fn accents_have_ids_and_names() {
        for accent in Accent::ALL {
            assert!(!accent.label().is_empty());
            assert_eq!(Accent::parse(&accent.as_str().to_uppercase()), Some(accent));
            assert_eq!(Accent::ALL.iter().filter(|a| a.as_str() == accent.as_str()).count(), 1);
            let c = accent.colors();
            assert!(c.color != c.hover && c.color != c.light);
        }
    }

    #[test]
    fn sizes_follow_the_scale() {
        assert!(space::SCALE.windows(2).all(|w| w[0] < w[1]));
        assert!(space::SCALE.iter().all(|s| s % 2.0 == 0.0));
        assert_eq!(
            [radius::ITEM, radius::INPUT, radius::PANEL, radius::DIALOG],
            [8, 9, 12, 15]
        );
        assert_eq!((size::BORDER, size::ACCENT_BAR, size::SELECTED_BAR), (1.0, 3.0, 19.0));
        assert!((10.0..=11.0).contains(&text::LABEL) && text::LABEL_TRACKING == 1.3);
        assert!((14.0..=15.0).contains(&text::BODY) && (14.0..=15.0).contains(&text::BODY_LG));
        const { assert!(text::WORDMARK_TRACKING < 0.0 && text::DISPLAY_TRACKING < 0.0) };
        assert_eq!(shadow::MENU.color.a(), 0x55);
    }

    #[test]
    fn motion_is_short_and_reduced_motion_stops_it() {
        let m = Motion::new(false);
        let all = [m.fast, m.base, m.slow];
        assert!(all.windows(2).all(|w| w[0] < w[1]));
        assert!(all.iter().all(|t| (0.12..=0.2).contains(t)));
        let r = Motion::new(true);
        assert!([r.fast, r.base, r.slow].iter().all(|t| *t == 0.0));
    }
}
