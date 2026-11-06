//! Studio's typefaces, bundled (`apps/studio/assets/fonts`, licences beside them; see `NOTICE.txt` there): Inter for
//! the interface, as on the website, JetBrains Mono for code and changing numbers, and Phosphor for icons.
//!
//! egui's own fonts are never loaded ([`definitions`] starts empty, and the workspace turns off egui's
//! `default_fonts`): one of them is under the Ubuntu Font Licence, which `deny.toml` does not allow.
//!
//! The voices ([`STRONG`] for controls, [`HEADING`] for headings) are families of their own so that each can take a
//! heavier Inter instance without touching the code that uses them. Only Inter's regular weight is bundled for now,
//! so they share it and differ by size.

use std::sync::Arc;

use egui::epaint::text::VariationCoords;
use egui::{FontData, FontDefinitions, FontFamily, FontId, FontTweak};

const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Variable.ttf");
const PHOSPHOR: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");

/// Controls: button text, labels, names.
pub const STRONG: &str = "strong";
/// Headings, titles and the wordmark.
pub const HEADING: &str = "heading";
/// Phosphor's regular icons alone. Also the last fallback of every text family, so an icon can sit in a label.
pub const ICONS: &str = "icons";

/// The whole font set: Inter for proportional text and the voices, JetBrains Mono for monospace, each falling back
/// to Phosphor; and the icon family.
pub fn definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::empty();
    defs.font_data
        .insert("inter".into(), Arc::new(FontData::from_static(INTER)));
    defs.font_data
        .insert("phosphor".into(), Arc::new(FontData::from_static(PHOSPHOR)));
    defs.font_data.insert(
        "jetbrains-mono".into(),
        Arc::new(FontData::from_static(JETBRAINS_MONO).tweak(FontTweak {
            coords: VariationCoords::new([(b"wght", 400.0)]),
            ..FontTweak::default()
        })),
    );
    let text = || vec!["inter".to_owned(), "phosphor".to_owned()];
    for family in [
        FontFamily::Proportional,
        FontFamily::Name(STRONG.into()),
        FontFamily::Name(HEADING.into()),
    ] {
        defs.families.insert(family, text());
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
        assert_eq!(names, ["inter", "jetbrains-mono", "phosphor"]);
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            assert!(defs.families.contains_key(&family));
        }
        for name in [STRONG, HEADING, ICONS] {
            let fonts = &defs.families[&FontFamily::Name(name.into())];
            assert!(fonts.iter().all(|font| defs.font_data.contains_key(font)), "{name}");
        }
    }

    #[test]
    fn every_family_lays_out_text() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
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
        // Nothing paints this pass: the font atlas' upload is dropped on purpose.
        output.textures_delta.clear();
    }

    #[test]
    fn licences_sit_beside_the_fonts() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        for (font, licence) in [
            ("Inter-Regular.ttf", "OFL-Inter.txt"),
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
