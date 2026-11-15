//! The image preview (#681): a project picture in a tab beside the viewport, opened by a double-click on it in the
//! Assets panel. The picture, scaled to fit at its aspect ratio (never past its own size), and its facts: size in
//! pixels, format, file size, and the materials that use it.

use egui::{Rect, RichText, Ui, pos2, vec2};
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
    ui.add_space(space::S4);
    let area = ui.available_rect_before_wrap();
    let scale = (area.width() / width.max(1) as f32)
        .min(area.height() / height.max(1) as f32)
        .min(1.0);
    let size = vec2(width as f32 * scale, height as f32 * scale);
    let rect = Rect::from_min_size(area.center_top() - vec2(size.x / 2.0, 0.0), size);
    let response = ui.allocate_rect(rect, egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &path));
    ui.painter().image(
        shown.texture.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}
