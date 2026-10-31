//! Studio's typefaces: Archivo for all interface text, JetBrains Mono for code and changing
//! numbers, Noto Sans as the Greek and Cyrillic fallback, Phosphor for icons. The files and their
//! licences are in `crates/metallic_studio/assets/fonts` (see `NOTICE.txt` there).
//!
//! Archivo and Noto Sans are variable fonts: each voice below is one instance picked with
//! [`FontTweak::coords`] (`wght`, `wdth`), so one file gives every weight. egui's own fonts are
//! never used: [`definitions`] starts from an empty set.

use std::sync::Arc;

use eframe::egui::{
    self, FontData, FontDefinitions, FontFamily, FontId, FontTweak, epaint::text::VariationCoords,
};
use metallic_tokens::consts as tk;

const ARCHIVO: &[u8] = include_bytes!("../../assets/fonts/Archivo-Variable.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Variable.ttf");
const NOTO_SANS: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Variable.ttf");
const PHOSPHOR: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");
const PHOSPHOR_FILL: &[u8] = include_bytes!("../../assets/fonts/Phosphor-Fill.ttf");

/// Family of labels, button text and names (weight 580).
pub const UI_STRONG: &str = "ui-strong";
/// Family of panel and section headings (weight 620, width 112.5).
pub const HEADING: &str = "heading";
/// Family of the display and title voices (weight 700, width 125).
pub const DISPLAY: &str = "display";
/// Phosphor regular icons alone (also the last fallback of every text family).
pub const ICONS: &str = "icons";
/// Phosphor fill icons (status only). It is a family of its own: both Phosphor fonts use the same
/// code points, so it could never be reached as a fallback.
pub const ICONS_FILL: &str = "icons-fill";

/// One Archivo instance: the family it leads, `wght` and `wdth`.
struct Voice {
    family: FontFamily,
    weight: f32,
    width: f32,
}

fn voices() -> [Voice; 4] {
    let named = |name: &str| FontFamily::Name(name.into());
    [
        Voice {
            family: FontFamily::Proportional,
            weight: 400.0,
            width: 100.0,
        },
        Voice {
            family: named(UI_STRONG),
            weight: 580.0,
            width: 100.0,
        },
        Voice {
            family: named(HEADING),
            weight: 620.0,
            width: 112.5,
        },
        Voice {
            family: named(DISPLAY),
            weight: 700.0,
            width: 125.0,
        },
    ]
}

fn instance(bytes: &'static [u8], coords: &[(&[u8; 4], f32)]) -> Arc<FontData> {
    Arc::new(FontData::from_static(bytes).tweak(FontTweak {
        coords: VariationCoords::new(coords.iter().copied()),
        ..Default::default()
    }))
}

/// The font set: one family per voice, each Archivo, then Noto Sans at the same weight, then
/// Phosphor; monospace is JetBrains Mono 400 with the same fallbacks; plus the two icon families.
pub fn definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::empty();
    defs.font_data
        .insert("phosphor".into(), Arc::new(FontData::from_static(PHOSPHOR)));
    defs.font_data.insert(
        "phosphor-fill".into(),
        Arc::new(FontData::from_static(PHOSPHOR_FILL)),
    );
    for voice in voices() {
        let archivo = format!("archivo-{}-{}", voice.weight, voice.width);
        let noto = format!("noto-sans-{}", voice.weight);
        defs.font_data.insert(
            archivo.clone(),
            instance(ARCHIVO, &[(b"wght", voice.weight), (b"wdth", voice.width)]),
        );
        defs.font_data.insert(
            noto.clone(),
            instance(NOTO_SANS, &[(b"wght", voice.weight)]),
        );
        defs.families
            .insert(voice.family, vec![archivo, noto, "phosphor".into()]);
    }
    defs.font_data.insert(
        "jetbrains-mono".into(),
        instance(JETBRAINS_MONO, &[(b"wght", 400.0)]),
    );
    defs.families.insert(
        FontFamily::Monospace,
        vec![
            "jetbrains-mono".into(),
            "noto-sans-400".into(),
            "phosphor".into(),
        ],
    );
    defs.families
        .insert(FontFamily::Name(ICONS.into()), vec!["phosphor".into()]);
    defs.families.insert(
        FontFamily::Name(ICONS_FILL.into()),
        vec!["phosphor-fill".into()],
    );
    defs
}

/// Installs [`definitions`] into `ctx` (takes effect from the next pass).
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

/// `strong`: labels, button text, names.
pub fn strong() -> FontId {
    FontId::new(tk::T_STRONG_SIZE, FontFamily::Name(UI_STRONG.into()))
}

/// `caption`: counts and keycaps (never smaller).
pub fn caption() -> FontId {
    FontId::new(tk::T_CAPTION_SIZE, FontFamily::Name(UI_STRONG.into()))
}

/// `small`: metadata and help text.
pub fn small() -> FontId {
    FontId::proportional(tk::T_SMALL_SIZE)
}

/// `body`: default interface text.
pub fn body() -> FontId {
    FontId::proportional(tk::T_BODY_SIZE)
}

/// `body-lg`: descriptions.
pub fn body_lg() -> FontId {
    FontId::proportional(tk::T_BODY_LG_SIZE)
}

/// `heading`: panel and section headings.
pub fn heading() -> FontId {
    FontId::new(tk::T_HEADING_SIZE, FontFamily::Name(HEADING.into()))
}

/// `title`: stage and dialog titles.
pub fn title() -> FontId {
    FontId::new(tk::T_TITLE_SIZE, FontFamily::Name(DISPLAY.into()))
}

/// `display`: the one big line of a screen.
pub fn display() -> FontId {
    FontId::new(tk::T_DISPLAY_SIZE, FontFamily::Name(DISPLAY.into()))
}

/// A Phosphor regular icon font at `size`.
pub fn icon(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

/// A Phosphor fill icon font at `size` (status icons only).
pub fn icon_fill(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS_FILL.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one pass with the fonts installed and asks whether `font` has every char of `text`.
    fn has_glyphs(font: &FontId, text: &str) -> bool {
        let ctx = egui::Context::default();
        install(&ctx);
        // The font atlas upload is not needed here.
        ctx.run_ui(egui::RawInput::default(), |_| {})
            .textures_delta
            .clear();
        ctx.fonts_mut(|f| text.chars().all(|c| f.has_glyph(font, c)))
    }

    #[test]
    fn every_family_starts_with_a_bundled_font() {
        let defs = definitions();
        assert!(!defs.font_data.keys().any(|k| k.contains("Ubuntu")));
        for family in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            strong().family,
            heading().family,
            display().family,
            icon(16.0).family,
            icon_fill(16.0).family,
        ] {
            let list = &defs.families[&family];
            assert!(defs.font_data.contains_key(&list[0]), "{family:?}");
        }
    }

    #[test]
    fn text_and_icon_glyphs_are_covered() {
        // Latin, Latin Extended, Greek and Cyrillic (via Noto Sans), and an icon in a text family.
        let text = "Metallic Studio ÄéŁőș € Ωλ Жд \u{e3d4}";
        assert!(has_glyphs(&FontId::proportional(13.0), text));
        assert!(has_glyphs(&display(), text));
        assert!(has_glyphs(&FontId::monospace(12.0), "fn x = 0.25; {}"));
        // plus, check-circle, warning-circle in both icon weights.
        assert!(has_glyphs(&icon(16.0), "\u{e3d4}\u{e184}\u{e4e2}"));
        assert!(has_glyphs(&icon_fill(16.0), "\u{e3d4}\u{e184}\u{e4e2}"));
    }
}
