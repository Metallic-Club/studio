//! The image preview (#681): a project picture in a tab beside the viewport, opened by a double-click on it in the
//! Assets panel. The picture, scaled to fit at its aspect ratio (never past its own size), and its facts: size in
//! pixels, format, file size, and the materials that use it.
//!
//! It zooms (#781): Ctrl and the wheel over it, or the buttons above it (Zoom out, Zoom in, Fit and 100 %); zoomed
//! past the tab it scrolls. A pixelated picture shows crisp at every zoom.

use egui::{Rect, RichText, ScrollArea, Ui, pos2, vec2};
use metallic_engine::metallic_project::AssetId;

use super::assets::file_size;
use super::empty_note;
use crate::dock::{Panel, PanelContext, Zone};
use crate::editor::EditorState;
use crate::editor::images::{PANEL, image_of};
use crate::keymap::Context;
use crate::theme::current_colors;
use crate::theme::icons::Icon;
use crate::theme::tokens::{space, text};

/// The image preview.
#[derive(Clone, Copy, Debug)]
pub struct ImagePreview;

impl Panel for ImagePreview {
    fn id(&self) -> &'static str {
        PANEL
    }

    fn title(&self) -> &'static str {
        "Image"
    }

    fn icon(&self) -> Icon {
        Icon::AssetImage
    }

    fn zone(&self) -> Zone {
        Zone::Centre
    }

    fn context(&self) -> Context {
        Context::Editor
    }

    fn open_by_default(&self) -> bool {
        false
    }

    fn transient(&self) -> bool {
        true
    }

    fn ui(&self, ui: &mut Ui, _cx: &mut PanelContext<'_>) {
        empty_note(ui, "Double-click a picture in the Assets panel to see it.");
    }

    fn pane_title(&self, pane: &str, cx: &PanelContext<'_>) -> String {
        image_of(pane)
            .and_then(|id| cx.editor.project.assets().path(id))
            .map(|path| crate::editor::assets::file_name(path).to_owned())
            .unwrap_or_else(|| self.title().to_owned())
    }

    fn pane_ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>, pane: &str) {
        let Some(id) = image_of(pane) else {
            return;
        };
        preview_ui(ui, cx.editor, id);
    }
}

/// The picture `id` and its facts.
fn preview_ui(ui: &mut Ui, editor: &mut EditorState, id: AssetId) {
    let p = current_colors(ui.ctx()).palette;
    let Some(path) = editor.project.assets().path(id).map(str::to_owned) else {
        empty_note(ui, "This picture is no longer in the project.");
        return;
    };
    let Some(shown) = editor.image_preview(ui.ctx(), id) else {
        empty_note(ui, &format!("{path} could not be read as a picture."));
        return;
    };
    let (width, height) = shown.size;
    let format = path
        .rsplit_once('.')
        .map_or_else(String::new, |(_, extension)| extension.to_ascii_uppercase());
    let bytes = editor
        .project
        .fingerprint(&path)
        .map(|fingerprint| fingerprint.size)
        .unwrap_or(0);
    let using = editor.materials_using(id);
    // The facts first, so a tall picture never pushes them away.
    let facts = format!(
        "{width} \u{00d7} {height} px  \u{2022}  {format}  \u{2022}  {}",
        file_size(bytes)
    );
    ui.label(RichText::new(&path).size(text::BODY).color(p.text));
    ui.label(RichText::new(facts).size(text::SMALL).color(p.muted));
    let used = if using.is_empty() {
        "No material uses it.".to_owned()
    } else {
        format!("Used by {}", using.join(", "))
    };
    ui.label(RichText::new(used).size(text::SMALL).color(p.subtle));
    ui.add_space(space::S3);
    let zoom_id = egui::Id::unique(("metallic.image.zoom", id.to_string()));
    let mut zoom = ui.data(|data| data.get_temp::<Zoom>(zoom_id)).unwrap_or(Zoom::Fit);
    let area_before = ui.available_rect_before_wrap();
    // What fitting would scale it by: never past its own size.
    let fit = |area: Rect| {
        ((area.width() / width.max(1) as f32).min((area.height() - BAR) / height.max(1) as f32)).clamp(0.01, 1.0)
    };
    let shown_scale = |zoom: Zoom| match zoom {
        Zoom::Fit => fit(area_before),
        Zoom::Scale(scale) => scale,
    };
    ui.horizontal(|ui| {
        if ui.button("Zoom out").clicked() {
            zoom = Zoom::Scale(step(shown_scale(zoom), -1));
        }
        if ui.button("Zoom in").clicked() {
            zoom = Zoom::Scale(step(shown_scale(zoom), 1));
        }
        if ui.button("Fit").clicked() {
            zoom = Zoom::Fit;
        }
        if ui.button("100 %").clicked() {
            zoom = Zoom::Scale(1.0);
        }
        let percent = format!("{:.0} %", shown_scale(zoom) * 100.0);
        let label = ui.label(RichText::new(&percent).size(text::SMALL).color(p.muted));
        label.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("Zoom {percent}")));
    });
    ui.add_space(space::S2);
    let area = ui.available_rect_before_wrap();
    // Ctrl and the wheel over the picture zoom it, as they scale the Assets panel's tiles (#696).
    if ui.rect_contains_pointer(area) {
        let factor = ui.input(|input| input.zoom_delta());
        if factor != 1.0 {
            zoom = Zoom::Scale((shown_scale(zoom) * factor).clamp(MIN_ZOOM, MAX_ZOOM));
        }
    }
    ui.data_mut(|data| data.insert_temp(zoom_id, zoom));
    let scale = shown_scale(zoom);
    let size = vec2(width as f32 * scale, height as f32 * scale);
    ScrollArea::both()
        .id_salt(zoom_id)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Centred while it is smaller than the tab.
            let room = ui.available_size();
            let at = ui.cursor().min + vec2(((room.x - size.x) / 2.0).max(0.0), 0.0);
            let rect = Rect::from_min_size(at, size);
            ui.allocate_exact_size(vec2(size.x.max(room.x), size.y), egui::Sense::hover());
            let response = ui.interact(rect, zoom_id.with("picture"), egui::Sense::hover());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &path));
            ui.painter().image(
                shown.texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        });
}

/// How far the picture is zoomed (#781): to fit the tab, or a scale of its own pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Zoom {
    /// As large as fits, never past its own size.
    Fit,
    /// Each of its pixels this many points across.
    Scale(f32),
}

/// The zooms the buttons step through.
const STEPS: [f32; 13] = [0.0625, 0.125, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0, 32.0];
/// The least and the most it zooms.
const MIN_ZOOM: f32 = 0.0625;
const MAX_ZOOM: f32 = 32.0;
/// The height of the zoom buttons' row, kept out of what fitting fills.
const BAR: f32 = 32.0;

/// The next zoom from `scale` in direction `by` (1 in, -1 out) among [`STEPS`].
fn step(scale: f32, by: i32) -> f32 {
    if by > 0 {
        STEPS.iter().copied().find(|&s| s > scale * 1.001).unwrap_or(MAX_ZOOM)
    } else {
        STEPS
            .iter()
            .rev()
            .copied()
            .find(|&s| s < scale * 0.999)
            .unwrap_or(MIN_ZOOM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zoom_steps_between_its_ends() {
        assert_eq!(step(1.0, 1), 1.5);
        assert_eq!(step(1.0, -1), 0.75);
        assert_eq!(step(0.6, 1), 0.75, "from a fit, the next step up");
        assert_eq!(step(32.0, 1), MAX_ZOOM);
        assert_eq!(step(0.0625, -1), MIN_ZOOM);
    }
}
