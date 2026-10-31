//! The "Third-party licences" screen: the fonts and icons Studio bundles with their full licence
//! texts, and the Rust crates it is built from with their licences.
//!
//! Everything is compiled in, so the texts ship inside the exe. The font and icon texts come from
//! `assets/fonts`; the crate list is `assets/THIRD_PARTY.md`, written by `tools/gen_third_party.py`.
//! A test fails when that file no longer matches `Cargo.lock`.

use eframe::egui::{
    self, Align, Align2, Area, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Order,
    RichText, ScrollArea, Sense, Stroke, Ui, vec2,
};

use crate::theme::{self, fonts, icons};
use crate::widgets::button::Button;
use crate::widgets::segmented::segmented;

/// Width and height of the dialog.
const SIZE: egui::Vec2 = vec2(760.0, 600.0);
/// Height of one row in the crate list.
const ROW: f32 = 24.0;

/// A bundled font or icon set with its licence.
pub struct Bundled {
    /// Name shown as the heading.
    pub name: &'static str,
    /// What Studio uses it for, version and licence name.
    pub detail: &'static str,
    /// The full licence text.
    pub text: &'static str,
}

/// Every font and icon set in `assets/fonts`.
pub const BUNDLED: [Bundled; 4] = [
    Bundled {
        name: "Archivo",
        detail: "Interface text. Version 2.001. SIL Open Font License 1.1.",
        text: include_str!("../assets/fonts/OFL-Archivo.txt"),
    },
    Bundled {
        name: "JetBrains Mono",
        detail: "Code and changing numbers. Version 2.304. SIL Open Font License 1.1.",
        text: include_str!("../assets/fonts/OFL-JetBrainsMono.txt"),
    },
    Bundled {
        name: "Noto Sans",
        detail: "Greek and Cyrillic text. Version 2.015. SIL Open Font License 1.1.",
        text: include_str!("../assets/fonts/OFL-NotoSans.txt"),
    },
    Bundled {
        name: "Phosphor Icons",
        detail: "Interface icons. Version 2.1. MIT License.",
        text: include_str!("../assets/fonts/LICENSE-Phosphor.txt"),
    },
];

/// The generated crate list (see `tools/gen_third_party.py`).
const CRATES_MD: &str = include_str!("../assets/THIRD_PARTY.md");

/// One Rust crate and its licence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateLicence {
    /// Crate name.
    pub name: String,
    /// Crate version.
    pub version: String,
    /// SPDX licence expression.
    pub licence: String,
}

/// Reads the table rows of `THIRD_PARTY.md`.
pub fn parse_crates(markdown: &str) -> Vec<CrateLicence> {
    markdown
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.strip_prefix('|')?.split('|').map(str::trim).collect();
            match cells[..] {
                [name, version, licence, ""] if name != "Crate" && !name.starts_with("---") => {
                    Some(CrateLicence {
                        name: name.to_owned(),
                        version: version.to_owned(),
                        licence: licence.to_owned(),
                    })
                }
                _ => None,
            }
        })
        .collect()
}

/// The registry crates (name and version) `metallic_studio` depends on, read from `Cargo.lock` by
/// walking its dependency graph. Other crates in the workspace do not count.
pub fn studio_dependencies(cargo_lock: &str) -> Vec<(String, String)> {
    let Ok(lock) = cargo_lock.parse::<toml::Table>() else {
        return Vec::new();
    };
    let text = |p: &toml::Table, key: &str| {
        p.get(key)
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let packages: Vec<&toml::Table> = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(toml::Value::as_table).collect())
        .unwrap_or_default();
    let Some(start) = packages
        .iter()
        .position(|p| text(p, "name") == "metallic_studio")
    else {
        return Vec::new();
    };
    let mut seen = vec![false; packages.len()];
    let mut todo = vec![start];
    while let Some(i) = todo.pop() {
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        let deps = packages[i]
            .get("dependencies")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str);
        for dep in deps {
            // "name", or "name version" when several versions exist.
            let mut parts = dep.split_whitespace();
            let (name, version) = (parts.next().unwrap_or_default(), parts.next());
            let found = packages.iter().position(|p| {
                text(p, "name") == name && version.is_none_or(|v| text(p, "version") == v)
            });
            if let Some(j) = found {
                todo.push(j);
            }
        }
    }
    let mut out: Vec<(String, String)> = packages
        .iter()
        .zip(&seen)
        .filter(|(p, seen)| **seen && p.contains_key("source"))
        .map(|(p, _)| (text(p, "name"), text(p, "version")))
        .collect();
    out.sort();
    out
}

/// Which list the screen shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Fonts,
    Crates,
}

/// What the screen remembers while it is open.
pub struct Licences {
    tab: Tab,
    crates: Vec<CrateLicence>,
}

impl Default for Licences {
    fn default() -> Self {
        Self {
            tab: Tab::default(),
            crates: parse_crates(CRATES_MD),
        }
    }
}

impl Licences {
    /// Opens on the Rust libraries list (for captures).
    pub fn on_crates() -> Self {
        Self {
            tab: Tab::Crates,
            ..Self::default()
        }
    }

    /// Draws the dialog over the whole window; true when it should close.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut close = false;
        let screen = ctx.content_rect();
        let backdrop = Area::new(Id::unique("licences-backdrop"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(screen, Sense::click());
                ui.painter().rect_filled(screen, 0.0, theme::backdrop());
                response
            });
        close |= backdrop.inner.clicked();

        Area::new(Id::unique("licences-dialog"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::bg_panel())
                    .stroke(Stroke::new(1.0, theme::line_strong()))
                    .corner_radius(CornerRadius::same(theme::R_LG))
                    .shadow(theme::SHADOW_MODAL)
                    .inner_margin(Margin::same(24))
                    .show(ui, |ui| {
                        // Never larger than the window.
                        ui.set_width(SIZE.x.min(screen.width() - 80.0).max(320.0));
                        ui.set_height(SIZE.y.min(screen.height() - 80.0).max(240.0));
                        close |= self.body(ui);
                    });
            });
        close || ctx.input(|i| i.key_pressed(Key::Escape))
    }

    fn body(&mut self, ui: &mut Ui) -> bool {
        let mut close = false;
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Third-party licences")
                    .font(fonts::title())
                    .color(theme::text_1()),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = ui
                    .add(
                        Button::ghost("")
                            .icon(icons::X)
                            .alt("Close licences")
                            .compact(),
                    )
                    .clicked();
            });
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Metallic Studio includes the fonts, icons and code libraries below. \
                 Their licences are reproduced as the authors require.",
            )
            .font(fonts::small())
            .color(theme::text_2()),
        );
        ui.add_space(14.0);
        let tabs = [
            (Tab::Fonts, "Fonts and icons"),
            (Tab::Crates, "Rust libraries"),
        ];
        segmented(ui, &mut self.tab, &tabs);
        ui.add_space(12.0);
        match self.tab {
            Tab::Fonts => fonts_list(ui),
            Tab::Crates => self.crates_list(ui),
        }
        close
    }

    fn crates_list(&self, ui: &mut Ui) {
        ui.label(
            RichText::new(format!(
                "{} libraries. Where a library offers a choice of licences, any one of them applies.",
                self.crates.len()
            ))
            .font(fonts::small())
            .color(theme::text_3()),
        );
        ui.add_space(8.0);
        let (name_w, version_w) = (230.0, 90.0);
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW, self.crates.len(), |ui, range| {
                for item in &self.crates[range] {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Label,
                            true,
                            format!("{} {}, {}", item.name, item.version, item.licence),
                        )
                    });
                    let painter = ui.painter();
                    let cell = |x: f32, text: &str, font: FontId, colour| {
                        painter.text(
                            egui::pos2(rect.left() + x, rect.center().y),
                            Align2::LEFT_CENTER,
                            text,
                            font,
                            colour,
                        );
                    };
                    cell(0.0, &item.name, FontId::monospace(12.0), theme::text_1());
                    cell(
                        name_w,
                        &item.version,
                        FontId::monospace(12.0),
                        theme::text_2(),
                    );
                    cell(
                        name_w + version_w,
                        &item.licence,
                        fonts::small(),
                        theme::text_2(),
                    );
                }
            });
    }
}

fn fonts_list(ui: &mut Ui) {
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, item) in BUNDLED.iter().enumerate() {
                if i > 0 {
                    ui.add_space(20.0);
                }
                ui.label(
                    RichText::new(item.name)
                        .font(fonts::heading())
                        .color(theme::text_1()),
                );
                ui.add_space(2.0);
                ui.label(
                    RichText::new(item.detail)
                        .font(fonts::small())
                        .color(theme::text_2()),
                );
                ui.add_space(8.0);
                Frame::new()
                    .fill(theme::bg_deep())
                    .stroke(Stroke::new(1.0, theme::line()))
                    .corner_radius(CornerRadius::same(theme::R_SM))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(item.text.trim())
                                .font(FontId::monospace(11.5))
                                .color(theme::text_2()),
                        );
                    });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_licence_has_text() {
        for item in &BUNDLED {
            assert!(item.text.len() > 500, "{}", item.name);
        }
        assert!(BUNDLED[0].text.contains("SIL OPEN FONT LICENSE"));
        assert!(BUNDLED[3].text.contains("MIT"));
    }

    #[test]
    fn the_table_is_parsed() {
        let md = "# T\n| Crate | Version | Licence |\n| --- | --- | --- |\n| a | 1.0.0 | MIT OR Apache-2.0 |\n";
        assert_eq!(
            parse_crates(md),
            [CrateLicence {
                name: "a".into(),
                version: "1.0.0".into(),
                licence: "MIT OR Apache-2.0".into()
            }]
        );
    }

    /// Fails only when Studio's own dependencies changed without regenerating the list.
    #[test]
    fn the_crate_list_matches_studios_dependencies() {
        let lock =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
                .expect("read Cargo.lock");
        let want = studio_dependencies(&lock);
        assert!(want.len() > 100, "found {} dependencies", want.len());
        let mut have: Vec<(String, String)> = parse_crates(CRATES_MD)
            .into_iter()
            .map(|c| (c.name, c.version))
            .collect();
        have.sort();
        assert!(
            have == want,
            "assets/THIRD_PARTY.md does not match metallic_studio's dependencies; run: python tools/gen_third_party.py"
        );
    }

    #[test]
    fn the_screen_draws_both_lists_and_closes() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut harness = Harness::builder()
            .with_size(vec2(1000.0, 760.0))
            .build_ui_state(
                |ui, state: &mut (Licences, bool)| {
                    let ctx = ui.ctx().clone();
                    theme::apply(&ctx, theme::ThemeId::Graphite);
                    state.1 |= state.0.show(&ctx);
                },
                (Licences::default(), false),
            );
        harness.run();
        harness.get_by_label("Archivo");
        harness.get_by_label("Rust libraries").click();
        harness.run();
        harness.get_by_label_contains("libraries. Where");
        assert!(!harness.state().1);
        harness.get_by_label("Close licences").click();
        harness.run();
        assert!(harness.state().1);
    }
}
