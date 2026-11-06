//! Studio's typefaces, bundled (`apps/studio/assets/fonts`, licences beside them; see `NOTICE.txt` there): Inter for
//! the interface, as on the website, JetBrains Mono for code and changing numbers, and Phosphor for icons.
//!
//! egui's own fonts are never loaded ([`definitions`] starts empty, and the workspace turns off egui's
//! `default_fonts`): one of them is under the Ubuntu Font Licence, which `deny.toml` does not allow.
//!
//! Inter is its variable font, and egui 0.37 renders a variable font at the axis values a [`FontTweak`] names (it
//! shapes and rasterises through `skrifa`/`harfrust` at that location). So each voice is the same font file at its
//! own weight: body text at 400, controls ([`STRONG`]) at 650 and headings ([`HEADING`]) at 800 with Inter's display
//! optical size, as ADR 0009's Theme says.

use std::sync::Arc;

use egui::epaint::text::VariationCoords;
use egui::{FontData, FontDefinitions, FontFamily, FontId, FontTweak};

const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Variable.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Variable.ttf");
const PHOSPHOR: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");

/// Controls: button text, labels, names.
pub const STRONG: &str = "strong";
/// Headings, titles and the wordmark.
pub const HEADING: &str = "heading";
/// Phosphor's regular icons alone. Also the last fallback of every text family, so an icon can sit in a label.
pub const ICONS: &str = "icons";

/// Inter's weights, on its `wght` axis.
pub mod weight {
    /// Body text.
    pub const BODY: f32 = 400.0;
    /// Controls ([`super::STRONG`]).
    pub const STRONG: f32 = 650.0;
    /// Headings ([`super::HEADING`]).
    pub const HEADING: f32 = 800.0;
}

/// Inter's optical size for headings, on its `opsz` axis (14 to 32): the display cut, tighter and finer, as the
/// website's browsers pick for large text.
const HEADING_OPTICAL_SIZE: f32 = 32.0;

/// Phosphor's regular icons Studio uses, by their code points in `Phosphor.ttf` (the package's `style.css`).
pub mod icon {
    /// A gear: settings.
    pub const GEAR: char = '\u{E270}';
    /// A magnifying glass: search.
    pub const SEARCH: char = '\u{E30C}';
    /// A warning triangle.
    pub const WARNING: char = '\u{E4E0}';
    /// A cross: close, dismiss.
    pub const X: char = '\u{E4F6}';
}

/// Inter at the axis values `coords` names.
fn inter(coords: &[(&[u8; 4], f32)]) -> Arc<FontData> {
    Arc::new(FontData::from_static(INTER).tweak(FontTweak {
        coords: VariationCoords::new(coords.iter().copied()),
        ..FontTweak::default()
    }))
}

/// The whole font set: Inter for proportional text and, at their weights, the voices, JetBrains Mono for
/// monospace, each falling back to Phosphor; and the icon family.
pub fn definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::empty();
    defs.font_data.insert("inter".into(), inter(&[(b"wght", weight::BODY)]));
    defs.font_data
        .insert("inter-strong".into(), inter(&[(b"wght", weight::STRONG)]));
    defs.font_data.insert(
        "inter-heading".into(),
        inter(&[(b"wght", weight::HEADING), (b"opsz", HEADING_OPTICAL_SIZE)]),
    );
    defs.font_data
        .insert("phosphor".into(), Arc::new(FontData::from_static(PHOSPHOR)));
    defs.font_data.insert(
        "jetbrains-mono".into(),
        Arc::new(FontData::from_static(JETBRAINS_MONO).tweak(FontTweak {
            coords: VariationCoords::new([(b"wght", 400.0)]),
            ..FontTweak::default()
        })),
    );
    for (family, inter) in [
        (FontFamily::Proportional, "inter"),
        (FontFamily::Name(STRONG.into()), "inter-strong"),
        (FontFamily::Name(HEADING.into()), "inter-heading"),
    ] {
        defs.families
            .insert(family, vec![inter.to_owned(), "phosphor".to_owned()]);
    }
    defs.families.insert(
        FontFamily::Monospace,
        vec!["jetbrains-mono".into(), "inter".into(), "phosphor".into()],
    );
    defs.families
        .insert(FontFamily::Name(ICONS.into()), vec!["phosphor".into()]);
    defs
}

/// Installs [`definitions`] in `ctx`, from the next pass on.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

/// A font of the named family `name` at `size` points.
pub fn named(name: &str, size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(name.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_bundled_fonts_are_loaded() {
        let defs = definitions();
        // egui's defaults (Ubuntu-Light, Hack, the emoji fonts) are never in the set.
        let mut names: Vec<&String> = defs.font_data.keys().collect();
        names.sort();
        assert_eq!(
            names,
            ["inter", "inter-heading", "inter-strong", "jetbrains-mono", "phosphor"]
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            assert!(defs.families.contains_key(&family));
        }
        for name in [STRONG, HEADING, ICONS] {
            let fonts = &defs.families[&FontFamily::Name(name.into())];
            assert!(fonts.iter().all(|font| defs.font_data.contains_key(font)), "{name}");
        }
    }

    /// The heading's optical size at body weight: what the heading would be if egui ignored `wght`.
    const DISPLAY_REGULAR: &str = "display-regular";

    /// Runs one pass of a context with the fonts installed, handing `check` its `Ui`.
    fn with_fonts(check: impl FnMut(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        let mut defs = definitions();
        defs.font_data.insert(
            DISPLAY_REGULAR.into(),
            inter(&[(b"wght", weight::BODY), (b"opsz", HEADING_OPTICAL_SIZE)]),
        );
        defs.families
            .insert(FontFamily::Name(DISPLAY_REGULAR.into()), vec![DISPLAY_REGULAR.into()]);
        ctx.set_fonts(defs);
        let mut output = ctx.run_ui(egui::RawInput::default(), check);
        // Nothing paints this pass: the font atlas' upload is dropped on purpose.
        output.textures_delta.clear();
    }

    #[test]
    fn every_family_lays_out_text() {
        with_fonts(|ui| {
            for family in [
                FontFamily::Proportional,
                FontFamily::Monospace,
                FontFamily::Name(STRONG.into()),
                FontFamily::Name(HEADING.into()),
            ] {
                // Latin, Greek and Cyrillic: Inter covers all three.
                let galley = ui.painter().layout_no_wrap(
                    "Metallic Studio Ωμέγα Привет".into(),
                    FontId::new(14.0, family.clone()),
                    egui::Color32::WHITE,
                );
                assert!(galley.size().x > 100.0, "{family:?}");
            }
        });
    }

    #[test]
    fn the_voices_render_at_their_weights() {
        // Inter's heavier instances have wider advances, so if egui ignored `wght` each pair would measure the
        // same. The heading is compared at its own optical size, which is tighter than the body's.
        with_fonts(|ui| {
            let width = |family: FontFamily| {
                ui.painter()
                    .layout_no_wrap(
                        "Metallic Studio settings".into(),
                        FontId::new(14.0, family),
                        egui::Color32::WHITE,
                    )
                    .size()
                    .x
            };
            let body = width(FontFamily::Proportional);
            let strong = width(FontFamily::Name(STRONG.into()));
            let heading = width(FontFamily::Name(HEADING.into()));
            assert!(body < strong, "400 {body} vs 650 {strong}");
            let display = width(FontFamily::Name(DISPLAY_REGULAR.into()));
            assert!(display < heading, "display 400 {display} vs 800 {heading}");
        });
    }

    #[test]
    fn the_icons_are_in_phosphor() {
        with_fonts(|ui| {
            for icon in [icon::GEAR, icon::SEARCH, icon::WARNING, icon::X] {
                let has = ui.ctx().fonts_mut(|fonts| fonts.has_glyph(&named(ICONS, 16.0), icon));
                assert!(has, "U+{:04X}", icon as u32);
            }
        });
    }

    #[test]
    fn licences_sit_beside_the_fonts() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        for (font, licence) in [
            ("Inter-Variable.ttf", "OFL-Inter.txt"),
            ("JetBrainsMono-Variable.ttf", "OFL-JetBrainsMono.txt"),
            ("Phosphor.ttf", "LICENSE-Phosphor.txt"),
        ] {
            assert!(dir.join(font).is_file(), "{font}");
            let text = std::fs::read_to_string(dir.join(licence)).unwrap();
            assert!(
                text.contains("Open Font License") || text.contains("MIT License"),
                "{licence}"
            );
        }
        let notice = std::fs::read_to_string(dir.join("NOTICE.txt")).unwrap();
        // Every file in the folder is accounted for.
        for entry in std::fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            assert!(
                name == "NOTICE.txt" || notice.contains(&name),
                "{name} is not in NOTICE.txt"
            );
        }
    }
}
