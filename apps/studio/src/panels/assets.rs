//! The Assets panel: the project's asset index (#188: every member but the manifest and the index itself), each
//! asset's path, kind and size, with a badge on an asset the index names but the project no longer has.

use egui::{Align2, CornerRadius, FontId, Rect, RichText, Role, ScrollArea, Sense, Ui, WidgetInfo, pos2, vec2};

use super::{empty_note, ids};
use crate::dock::{Panel, PanelContext, Zone};
use crate::editor::AssetRow;
use crate::keymap::Context;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts};

/// The Assets panel.
#[derive(Clone, Copy, Debug)]
pub struct Assets;

/// What an asset is, from its file name's extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    /// A glTF or GLB model.
    Mesh,
    /// A PNG or JPEG picture.
    Image,
    /// A sound.
    Audio,
    /// A scene file.
    Scene,
    /// A Luau script.
    Script,
    /// Anything else.
    Other,
}

impl AssetKind {
    /// The kind of the asset at `path`.
    pub fn of(path: &str) -> Self {
        let extension = path.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
        match extension.as_deref() {
            Some("glb" | "gltf") => AssetKind::Mesh,
            Some("png" | "jpg" | "jpeg" | "webp" | "ktx2") => AssetKind::Image,
            Some("ogg" | "mp3" | "wav" | "opus" | "flac" | "m4a") => AssetKind::Audio,
            Some("scene") => AssetKind::Scene,
            Some("luau" | "lua") => AssetKind::Script,
            _ => AssetKind::Other,
        }
    }

    /// The name people see.
    pub fn label(self) -> &'static str {
        match self {
            AssetKind::Mesh => "Mesh",
            AssetKind::Image => "Image",
            AssetKind::Audio => "Audio",
            AssetKind::Scene => "Scene",
            AssetKind::Script => "Script",
            AssetKind::Other => "File",
        }
    }

    fn icon(self) -> Icon {
        match self {
            AssetKind::Mesh => Icon::AssetMesh,
            AssetKind::Image => Icon::AssetImage,
            AssetKind::Audio => Icon::AssetAudio,
            AssetKind::Scene => Icon::AssetScene,
            AssetKind::Script => Icon::AssetScript,
            AssetKind::Other => Icon::AssetFile,
        }
    }
}

/// A size in bytes as people read it.
pub fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if size < 10.0 {
        format!("{size:.1} {}", UNITS[unit])
    } else {
        format!("{size:.0} {}", UNITS[unit])
    }
}

/// The badge on a missing asset.
pub const MISSING: &str = "Missing";

impl Panel for Assets {
    fn id(&self) -> &'static str {
        ids::ASSETS
    }

    fn title(&self) -> &'static str {
        "Assets"
    }

    fn icon(&self) -> Icon {
        Icon::PanelAssets
    }

    fn zone(&self) -> Zone {
        Zone::Bottom
    }

    fn context(&self) -> Context {
        Context::Assets
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        let p = current_colors(ui.ctx()).palette;
        let assets = &cx.editor.assets;
        if assets.is_empty() {
            empty_note(
                ui,
                "This project has no assets yet. Its scenes, scripts, meshes, pictures and sounds show here.",
            );
            return;
        }
        let missing = assets.iter().filter(|asset| asset.missing).count();
        let summary = match (assets.len(), missing) {
            (1, 0) => "1 asset".to_owned(),
            (n, 0) => format!("{n} assets"),
            (n, m) => format!("{n} assets, {m} missing"),
        };
        ui.label(RichText::new(summary).size(text::SMALL).color(p.subtle));
        ui.add_space(space::S3);
        ScrollArea::vertical()
            .id_salt("assets.rows")
            .auto_shrink([false, false])
            .show_rows(ui, size::ROW, assets.len(), |ui, range| {
                for asset in &assets[range] {
                    row_ui(ui, asset);
                }
            });
    }
}

/// One asset: its kind's icon and its path; its kind and size, or the missing badge, on the right.
fn row_ui(ui: &mut Ui, asset: &AssetRow) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let kind = AssetKind::of(&asset.path);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::ListItem, true, &asset.path));
    let response = response.on_hover_text(format!("{}\nId {}", asset.path, asset.id));
    if !ui.is_rect_visible(rect) {
        return;
    }
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface);
    }
    let mut right = rect.right() - space::S3;
    let detail = if asset.missing {
        // The badge: the accent, as the projects page marks a missing file.
        let font = fonts::named(fonts::STRONG, text::LABEL);
        let galley = ui.painter().layout_no_wrap(MISSING.to_uppercase(), font, colors.accent);
        let badge = Rect::from_min_size(
            pos2(right - galley.size().x - 2.0 * space::S3, rect.center().y - 9.0),
            vec2(galley.size().x + 2.0 * space::S3, 18.0),
        );
        ui.painter().rect_stroke(
            badge,
            CornerRadius::same(radius::ITEM),
            egui::Stroke::new(size::BORDER, colors.accent),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            pos2(badge.left() + space::S3, badge.center().y - galley.size().y / 2.0),
            galley,
            colors.accent,
        );
        ui.interact(badge, response.id.with("missing"), Sense::hover())
            .widget_info(|| WidgetInfo::labeled(Role::Label, true, MISSING));
        right = badge.left() - space::S3;
        None
    } else {
        Some(match asset.size {
            Some(bytes) => format!("{}  {}", kind.label(), file_size(bytes)),
            None => kind.label().to_owned(),
        })
    };
    if let Some(detail) = detail {
        let at = ui.painter().text(
            pos2(right, rect.center().y),
            Align2::RIGHT_CENTER,
            detail,
            FontId::proportional(text::SMALL),
            p.subtle,
        );
        right = at.left() - space::S3;
    }
    let (icon, ink) = if asset.missing {
        (Icon::UiWarning, colors.accent)
    } else {
        (kind.icon(), kind.icon().color(&colors))
    };
    icons::paint_at(
        ui.painter(),
        pos2(rect.left() + space::S3, rect.center().y),
        Align2::LEFT_CENTER,
        icon,
        text::BODY,
        ink,
    );
    let path_left = rect.left() + space::S3 + text::BODY + space::S3;
    let clip = Rect::from_min_max(pos2(path_left, rect.top()), pos2(right.max(path_left), rect.bottom()));
    ui.painter().with_clip_rect(clip).text(
        pos2(path_left, rect.center().y),
        Align2::LEFT_CENTER,
        &asset.path,
        FontId::proportional(text::BODY),
        if asset.missing { p.muted } else { p.text },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(AssetKind::of("Meshes/Rock.GLB"), AssetKind::Mesh);
        assert_eq!(AssetKind::of("Textures/a.b/rock.jpeg"), AssetKind::Image);
        assert_eq!(AssetKind::of("Sounds/hit.ogg"), AssetKind::Audio);
        assert_eq!(AssetKind::of("Scenes/Main.scene"), AssetKind::Scene);
        assert_eq!(AssetKind::of("Scripts/Door.luau"), AssetKind::Script);
        assert_eq!(AssetKind::of("notes"), AssetKind::Other);
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(file_size(512), "512 B");
        assert_eq!(file_size(1536), "1.5 KB");
        assert_eq!(file_size(25 * 1024 * 1024), "25 MB");
    }
}
