//! The image preview (#681): a project picture in a tab beside the viewport, opened by a double-click on it in the
//! Assets panel. The picture fills the tab, fitted at its aspect ratio (never past its own size), its facts (its path,
//! size in pixels, format, file size, the materials that use it and the zoom) in a small overlay at its top left
//! (#799).
//!
//! It zooms toward the pointer (#781, #799): the wheel zooms in and out about what is under the pointer, a drag moves
//! it, and a double-click fits it to the tab again. A pixelated picture shows crisp at every zoom.

use egui::{Color32, CornerRadius, Rect, Sense, Ui, Vec2, pos2, vec2};
use metallic_engine::metallic_project::AssetId;

use super::assets::file_size;
use super::empty_note;
use crate::dock::{Panel, PanelContext, Zone};
use crate::editor::EditorState;
use crate::editor::images::{PANEL, image_of};
use crate::keymap::Context;
use crate::theme::current_colors;
use crate::theme::icons::Icon;
use crate::theme::tokens::{radius, space, text};

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

/// The picture `id`, as zoomed and moved, and its facts over it.
fn preview_ui(ui: &mut Ui, editor: &mut EditorState, id: AssetId) {
    let Some(path) = editor.project.assets().path(id).map(str::to_owned) else {
        empty_note(ui, "This picture is no longer in the project.");
        return;
    };
    let Some(shown) = editor.image_preview(ui.ctx(), id) else {
        empty_note(ui, &format!("{path} could not be read as a picture."));
        return;
    };
    let (width, height) = shown.size;
    let size = vec2(width.max(1) as f32, height.max(1) as f32);
    let area = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(area, Sense::click_and_drag());
    let view_id = egui::Id::unique(("metallic.image.view", id.to_string()));
    let mut view = ui.data(|data| data.get_temp::<View>(view_id)).unwrap_or_default();
    let fit = fit_scale(area, size);
    // The wheel over it zooms toward the pointer; a drag moves it; a double-click fits it again.
    if response.hovered() {
        let (scroll, zoom) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
        let factor = zoom * (scroll * WHEEL_ZOOM).exp();
        if factor != 1.0
            && let Some(pointer) = response.hover_pos()
        {
            view = view.zoomed(factor, pointer - area.center(), fit);
        }
    }
    if response.dragged() {
        view.offset += response.drag_delta();
        view.scale.get_or_insert(fit);
    }
    if response.double_clicked() {
        view = View::default();
    }
    ui.data_mut(|data| data.insert_temp(view_id, view));
    let scale = view.scale.unwrap_or(fit);
    let picture = Rect::from_center_size(area.center() + view.offset, size * scale);
    let painter = ui.painter_at(area);
    painter.image(
        shown.texture.id(),
        picture,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    ui.interact(picture.intersect(area), view_id.with("picture"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &path));
    let facts = Facts {
        path: &path,
        size: (width, height),
        scale,
    };
    overlay(ui, editor, id, &facts, area);
}

/// What the overlay says of a picture: its path, its size in pixels and how far it is zoomed.
struct Facts<'a> {
    path: &'a str,
    size: (u32, u32),
    scale: f32,
}

/// The facts, in a plate at the top left of `area`: the path, the size, format and file size, what uses the picture
/// `id`, and the zoom.
fn overlay(ui: &mut Ui, editor: &mut EditorState, id: AssetId, facts: &Facts<'_>, area: Rect) {
    let p = current_colors(ui.ctx()).palette;
    let (width, height) = facts.size;
    let format = facts
        .path
        .rsplit_once('.')
        .map_or_else(String::new, |(_, extension)| extension.to_ascii_uppercase());
    let bytes = editor
        .project
        .fingerprint(facts.path)
        .map(|fingerprint| fingerprint.size)
        .unwrap_or(0);
    let using = editor.materials_using(id);
    let sizes = format!(
        "{width} \u{00d7} {height} px  \u{2022}  {format}  \u{2022}  {}",
        file_size(bytes)
    );
    let used = if using.is_empty() {
        "No material uses it.".to_owned()
    } else {
        format!("Used by {}", using.join(", "))
    };
    let zoom = format!("{:.0} %", facts.scale * 100.0);
    let lines = [
        (facts.path.to_owned(), text::BODY, p.text),
        (sizes, text::SMALL, p.muted),
        (used, text::SMALL, p.subtle),
        (zoom.clone(), text::SMALL, p.muted),
    ];
    let wrap = (area.width() - 4.0 * space::S4).clamp(80.0, 520.0);
    let galleys: Vec<_> = lines
        .iter()
        .map(|(line, size, ink)| {
            ui.painter()
                .layout(line.clone(), egui::FontId::proportional(*size), *ink, wrap)
        })
        .collect();
    let inner = vec2(
        galleys.iter().map(|galley| galley.size().x).fold(0.0, f32::max),
        galleys.iter().map(|galley| galley.size().y).sum::<f32>() + space::S1 * (galleys.len() - 1) as f32,
    );
    let plate = Rect::from_min_size(area.min + Vec2::splat(space::S4), inner + Vec2::splat(2.0 * space::S3));
    let painter = ui.painter_at(area);
    painter.rect_filled(plate, CornerRadius::same(radius::ITEM), p.surface.gamma_multiply(0.88));
    let mut at = plate.min + Vec2::splat(space::S3);
    for (index, galley) in galleys.into_iter().enumerate() {
        let line = Rect::from_min_size(at, galley.size());
        at.y += galley.size().y + space::S1;
        painter.galley(line.min, galley, p.text);
        let label = match index {
            3 => format!("Zoom {zoom}"),
            _ => lines[index].0.clone(),
        };
        ui.interact(line, egui::Id::unique(("metallic.image.fact", index)), Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
    }
}

/// How far the wheel zooms, per point it scrolls.
const WHEEL_ZOOM: f32 = 0.0025;
/// The least it zooms.
const MIN_ZOOM: f32 = 0.0625;
/// The most it zooms.
const MAX_ZOOM: f32 = 64.0;

/// How the picture is shown (#799): its scale (`None` fits it to the tab) and how far its centre is from the tab's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct View {
    scale: Option<f32>,
    offset: Vec2,
}

impl View {
    /// Zoomed by `factor` about `pointer` (from the tab's centre), so what is under the pointer stays under it; `fit`
    /// is the scale while it fits.
    fn zoomed(self, factor: f32, pointer: Vec2, fit: f32) -> Self {
        let old = self.scale.unwrap_or(fit);
        let new = (old * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = new / old;
        Self {
            scale: Some(new),
            offset: pointer - (pointer - self.offset) * ratio,
        }
    }
}

/// The scale that fits a picture `size` pixels across in `area`, never past its own size.
fn fit_scale(area: Rect, size: Vec2) -> f32 {
    (area.width() / size.x).min(area.height() / size.y).clamp(0.01, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_keeps_what_is_under_the_pointer_under_it() {
        let view = View::default();
        let pointer = vec2(100.0, -40.0);
        // The picture's point under the pointer, at the fitting scale 0.5.
        let under = (pointer - view.offset) / 0.5;
        let zoomed = view.zoomed(4.0, pointer, 0.5);
        assert_eq!(zoomed.scale, Some(2.0));
        assert_eq!(zoomed.offset + under * 2.0, pointer, "the same point");
        assert_eq!(view.zoomed(1.0e9, pointer, 0.5).scale, Some(MAX_ZOOM));
    }
}
