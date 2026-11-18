//! The sound preview (#831): a project sound in a tab beside the viewport, opened by a double-click on it in the Assets
//! panel. Its waveform fills a timeline across the tab, the part played in the accent and a line where it is; a click
//! or a drag on the timeline moves there. Play and Pause (or Space) under it play it on the system's device, with the
//! time it is at and its length. Its facts (its path, length, rate, channels, format, file size and the sounds that
//! play it) are in a small overlay at its top left, as the picture preview's are (#799).

use egui::{Align2, CornerRadius, FontId, Key, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use metallic_engine::metallic_project::AssetId;

use super::assets::file_size;
use super::empty_note;
use crate::dock::{Panel, PanelContext, Zone};
use crate::editor::EditorState;
use crate::editor::sounds::{PANEL, Preview, sound_of};
use crate::keymap::Context;
use crate::theme::current_colors;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, space, text};

/// The sound preview.
#[derive(Clone, Copy, Debug)]
pub struct SoundPreview;

impl Panel for SoundPreview {
    fn id(&self) -> &'static str {
        PANEL
    }

    fn title(&self) -> &'static str {
        "Sound"
    }

    fn icon(&self) -> Icon {
        Icon::AssetAudio
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
        empty_note(ui, "Double-click a sound in the Assets panel to hear it.");
    }

    fn pane_title(&self, pane: &str, cx: &PanelContext<'_>) -> String {
        sound_of(pane)
            .and_then(|id| cx.editor.project.assets().path(id))
            .map(|path| crate::editor::assets::file_name(path).to_owned())
            .unwrap_or_else(|| self.title().to_owned())
    }

    fn pane_ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>, pane: &str) {
        let Some(id) = sound_of(pane) else {
            return;
        };
        preview_ui(ui, cx.editor, id);
    }
}

/// How tall the waveform's timeline is, at most, in points.
const TIMELINE_HEIGHT: f32 = 220.0;

/// `seconds` as minutes, seconds and hundredths: `1:02.35`.
pub fn clock(seconds: f64) -> String {
    let hundredths = (seconds.max(0.0) * 100.0).round() as u64;
    format!(
        "{}:{:02}.{:02}",
        hundredths / 6000,
        hundredths / 100 % 60,
        hundredths % 100
    )
}

/// The sound `id`'s timeline, its controls and its facts.
fn preview_ui(ui: &mut Ui, editor: &mut EditorState, id: AssetId) {
    let Some(path) = editor.project.assets().path(id).map(str::to_owned) else {
        empty_note(ui, "This sound is no longer in the project.");
        return;
    };
    let Some(preview) = editor.sound_preview(id) else {
        empty_note(ui, &format!("{path} could not be read as a sound."));
        return;
    };
    editor.sounds.update();
    let p = current_colors(ui.ctx()).palette;
    let accent = current_colors(ui.ctx()).accent;
    let area = ui.available_rect_before_wrap();
    let height = (area.height() * 0.4).clamp(40.0, TIMELINE_HEIGHT);
    let timeline = Rect::from_center_size(
        area.center() - vec2(0.0, space::S6),
        vec2((area.width() - 2.0 * space::S6).max(40.0), height),
    );
    let painter = ui.painter_at(area);

    // A click or a drag on the timeline moves there.
    let response = ui.interact(
        timeline,
        egui::Id::unique(("metallic.sound.timeline", id.to_string())),
        Sense::click_and_drag(),
    );
    if let Some(at) = response.interact_pointer_pos()
        && (response.dragged() || response.clicked() || response.drag_started())
    {
        let fraction = ((at.x - timeline.left()) / timeline.width()).clamp(0.0, 1.0);
        editor.sounds.seek(id, f64::from(fraction) * preview.seconds);
    }
    let place = editor.sounds.place(id).min(preview.seconds);
    let played = if preview.seconds > 0.0 {
        (place / preview.seconds) as f32
    } else {
        0.0
    };
    response.widget_info(|| {
        egui::WidgetInfo::slider(
            true,
            place,
            format!("Time {} of {}", clock(place), clock(preview.seconds)),
        )
    });

    painter.rect_filled(timeline, CornerRadius::same(radius::ITEM), p.surface);
    waveform(&painter, timeline, &preview, played, accent, p.muted);
    let head = timeline.left() + timeline.width() * played;
    painter.line_segment(
        [pos2(head, timeline.top()), pos2(head, timeline.bottom())],
        Stroke::new(2.0, p.text),
    );

    // Play or Pause, and where it is, under the timeline.
    let playing = editor.sounds.is_playing(id);
    let button = Rect::from_min_size(pos2(timeline.left(), timeline.bottom() + space::S4), Vec2::splat(28.0));
    let label = if playing { "Pause the sound" } else { "Play the sound" };
    let pressed = ui.interact(
        button,
        egui::Id::unique(("metallic.sound.play", id.to_string())),
        Sense::click(),
    );
    pressed.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let hovered = pressed.hovered();
    painter.rect_filled(
        button,
        CornerRadius::same(radius::ITEM),
        if hovered { p.surface_hover } else { p.surface },
    );
    icons::paint_at(
        &painter,
        button.center(),
        Align2::CENTER_CENTER,
        if playing { Icon::ToolbarPause } else { Icon::ToolbarPlay },
        text::BODY_LG,
        p.text,
    );
    let space_bar = response.hovered() && ui.input(|input| input.key_pressed(Key::Space));
    if pressed.clicked() || space_bar {
        if playing {
            editor.sounds.pause();
        } else {
            editor.sounds.play(id, &preview);
        }
    }
    painter.text(
        pos2(button.right() + space::S4, button.center().y),
        Align2::LEFT_CENTER,
        format!("{} / {}", clock(place), clock(preview.seconds)),
        FontId::monospace(text::BODY),
        p.muted,
    );
    if editor.sounds.is_playing(id) {
        ui.ctx().request_repaint();
    }
    ui.allocate_rect(area, Sense::hover());
    overlay(ui, editor, id, &path, &preview, area);
}

/// The waveform of `preview` across `timeline`: each column its stretch's lowest to highest, in `played` (the accent)
/// up to `fraction` of the way and in `rest` after it.
fn waveform(
    painter: &egui::Painter,
    timeline: Rect,
    preview: &Preview,
    fraction: f32,
    played: egui::Color32,
    rest: egui::Color32,
) {
    let columns = timeline.width().max(1.0) as usize;
    let half = timeline.height() * 0.45;
    let middle = timeline.center().y;
    let outline = &preview.outline;
    if outline.is_empty() {
        return;
    }
    for column in 0..columns {
        let from = column * outline.len() / columns;
        let to = ((column + 1) * outline.len() / columns)
            .max(from + 1)
            .min(outline.len());
        let [low, high] = outline[from..to]
            .iter()
            .fold([0.0f32, 0.0], |[low, high], [l, h]| [low.min(*l), high.max(*h)]);
        let x = timeline.left() + column as f32 + 0.5;
        let ink = if (column as f32) < timeline.width() * fraction {
            played
        } else {
            rest
        };
        // At least a hair, so silence is a line.
        let (top, bottom) = (middle - high.max(0.004) * half, middle - low.min(-0.004) * half);
        painter.line_segment([pos2(x, top), pos2(x, bottom)], Stroke::new(1.0, ink));
    }
}

/// The facts, in a plate at the top left of `area`: the path; the length, rate, channels, format and file size; and
/// how many sounds of the game play it.
fn overlay(ui: &mut Ui, editor: &mut EditorState, id: AssetId, path: &str, preview: &Preview, area: Rect) {
    let p = current_colors(ui.ctx()).palette;
    let format = path
        .rsplit_once('.')
        .map_or_else(String::new, |(_, extension)| extension.to_ascii_uppercase());
    let bytes = editor
        .project
        .fingerprint(path)
        .map(|fingerprint| fingerprint.size)
        .unwrap_or(0);
    let channels = match preview.channels {
        1 => "Mono".to_owned(),
        2 => "Stereo".to_owned(),
        n => format!("{n} channels"),
    };
    let facts = format!(
        "{}  \u{2022}  {} Hz  \u{2022}  {channels}  \u{2022}  {format}  \u{2022}  {}",
        clock(preview.seconds),
        preview.rate,
        file_size(bytes)
    );
    let used = match editor.sounds_using(id) {
        0 => "No sound plays it.".to_owned(),
        1 => "Played by 1 sound".to_owned(),
        n => format!("Played by {n} sounds"),
    };
    let lines = [
        (path.to_owned(), text::BODY, p.text),
        (facts, text::SMALL, p.muted),
        (used, text::SMALL, p.subtle),
    ];
    let wrap = (area.width() - 4.0 * space::S4).clamp(80.0, 520.0);
    let galleys: Vec<_> = lines
        .iter()
        .map(|(line, size, ink)| {
            ui.painter()
                .layout(line.clone(), FontId::proportional(*size), *ink, wrap)
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
        let label = lines[index].0.clone();
        ui.interact(line, egui::Id::unique(("metallic.sound.fact", index)), Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
    }
}

#[cfg(test)]
mod tests {
    use super::clock;

    #[test]
    fn times_read_as_minutes_seconds_and_hundredths() {
        assert_eq!(clock(0.0), "0:00.00");
        assert_eq!(clock(62.346), "1:02.35");
        assert_eq!(clock(-1.0), "0:00.00");
    }
}
