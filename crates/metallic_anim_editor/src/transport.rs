//! Transport bar: play/pause, jump to start/end, previous/next key, loop, snapping, a frame
//! counter, Record keys (K, explicit, off by default), Key (I) and speed. Space plays/pauses.

use crate::paint::{self, Icon};
use crate::state::EditorState;
use crate::theme::Theme;
use egui::{Align, FontId, Layout, RichText, Sense, StrokeKind, vec2};

/// The transport bar widget.
#[derive(Debug, Clone, Copy)]
pub struct TransportBar {
    theme: Theme,
}

impl Default for TransportBar {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}

impl TransportBar {
    /// A transport bar with these colours.
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Shows the bar on one row.
    pub fn show(self, ui: &mut egui::Ui, state: &mut EditorState) {
        let t = self.theme;
        if !ui.ctx().egui_wants_keyboard_input() {
            let (space, k) =
                ui.input(|i| (i.key_pressed(egui::Key::Space), i.key_pressed(egui::Key::K)));
            if space {
                state.playing = !state.playing;
            }
            if k {
                state.recording = !state.recording;
            }
        }
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if paint::icon_button(ui, &t, Icon::ToStart, false, "Go to start").clicked() {
                state.time = 0.0;
            }
            if paint::icon_button(ui, &t, Icon::PrevKey, false, "Previous key").clicked()
                && let Some(k) = state.prev_key_time()
            {
                state.time = k;
            }
            let play_icon = if state.playing {
                Icon::Pause
            } else {
                Icon::Play
            };
            if self.play_button(ui, play_icon, state.playing).clicked() {
                if !state.playing && state.time >= state.clip.duration - 1e-4 {
                    state.time = 0.0;
                }
                state.playing = !state.playing;
            }
            if paint::icon_button(ui, &t, Icon::NextKey, false, "Next key").clicked()
                && let Some(k) = state.next_key_time()
            {
                state.time = k;
            }
            if paint::icon_button(ui, &t, Icon::ToEnd, false, "Go to end").clicked() {
                state.time = state.clip.duration;
            }
            ui.add_space(10.0);
            if paint::icon_button(ui, &t, Icon::Loop, state.looping, "Loop playback").clicked() {
                state.looping = !state.looping;
            }
            if paint::icon_button(
                ui,
                &t,
                Icon::Snap,
                state.snap,
                "Snap to frames (hold Alt to move freely)",
            )
            .clicked()
            {
                state.snap = !state.snap;
            }
            ui.add_space(12.0);
            // Frame counter "12 / 60", then seconds.
            let fps = state.clip.frame_rate;
            let (r, _) = ui.allocate_exact_size(vec2(112.0, 24.0), Sense::hover());
            ui.painter().rect(
                r,
                f32::from(crate::theme::R_SM),
                t.deep,
                egui::Stroke::new(1.0, t.line),
                StrokeKind::Inside,
            );
            let total = paint::format_time(state.clip.duration, fps);
            ui.painter().text(
                r.left_center() + vec2(10.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{:>width$}", state.frame(), width = total.len()),
                FontId::monospace(13.0),
                t.text,
            );
            ui.painter().text(
                r.right_center() - vec2(10.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                format!("/ {total}"),
                FontId::monospace(12.0),
                t.text_dim,
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("{:.2} s", state.time))
                    .size(12.0)
                    .color(t.text_meta),
            );
            ui.add_space(14.0);
            if self.record_button(ui, state.recording).clicked() {
                state.recording = !state.recording;
            }
            if ui
                .button(RichText::new("Key  I").size(12.0))
                .on_hover_text("Key the selected track at the playhead (I)")
                .clicked()
                && let Some(track) = state.active_track
            {
                state.key_track_at_playhead(track);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                egui::ComboBox::from_id_salt("transport_speed")
                    .width(64.0)
                    .selected_text(format!("{}x", state.speed))
                    .show_ui(ui, |ui| {
                        for s in [0.1, 0.25, 0.5, 1.0, 1.5, 2.0] {
                            ui.selectable_value(&mut state.speed, s, format!("{s}x"));
                        }
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Playback speed")
                    });
                ui.label(RichText::new("Speed").size(12.0).color(t.text_dim));
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("{:.0} fps", fps))
                        .size(12.0)
                        .color(t.text_dim),
                );
            });
        });
    }

    fn record_button(&self, ui: &mut egui::Ui, on: bool) -> egui::Response {
        let t = &self.theme;
        let (rect, resp) = ui.allocate_exact_size(vec2(118.0, 26.0), Sense::click());
        let r = f32::from(crate::theme::R_MD);
        let p = ui.painter();
        if on {
            p.rect_filled(rect, r, t.error);
        } else {
            p.rect_filled(rect, r, if resp.hovered() { t.hover } else { t.raised });
        }
        let fg = if on { t.on_accent } else { t.text };
        let dot = rect.left_center() + vec2(14.0, 0.0);
        if on {
            p.circle_filled(dot, 5.0, t.on_accent);
        } else {
            p.circle(
                dot,
                4.5,
                egui::Color32::TRANSPARENT,
                egui::Stroke::new(1.5, t.error),
            );
        }
        p.text(
            dot + vec2(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "Record keys",
            FontId::proportional(12.5),
            fg,
        );
        p.text(
            rect.right_center() - vec2(9.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "K",
            FontId::monospace(11.0),
            if on { t.on_accent } else { t.text_meta },
        );
        resp.on_hover_text(
            "Record keys (K): while on, changing a value at the playhead adds or updates a key",
        )
    }

    fn play_button(&self, ui: &mut egui::Ui, ic: Icon, playing: bool) -> egui::Response {
        let t = &self.theme;
        let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 26.0), Sense::click());
        let fill = if resp.hovered() {
            t.accent_hover
        } else {
            t.accent
        };
        ui.painter()
            .rect_filled(rect, f32::from(crate::theme::R_MD), fill);
        paint::icon(ui.painter(), rect, ic, t.on_accent);
        resp.on_hover_text(if playing {
            "Pause (Space)"
        } else {
            "Play (Space)"
        })
    }
}
