//! Key inspector: the selected keys' time and value, interpolation, and an easing gallery that
//! shows every curve as a small picture (beginners pick a shape, not a formula name).

use crate::scalar;
use crate::state::EditorState;
use crate::theme::Theme;
use egui::{RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use metallic_anim::{EasingDirection, EasingStyle, Interpolation, LoopMode, TangentMode, ease};

/// The key inspector widget.
#[derive(Debug, Clone, Copy)]
pub struct KeyInspector {
    theme: Theme,
}

impl Default for KeyInspector {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}

fn section(ui: &mut egui::Ui, theme: &Theme, title: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(title)
            .size(12.0)
            .color(theme.text_dim)
            .strong(),
    );
    ui.add_space(2.0);
}

impl KeyInspector {
    /// An inspector with these colours.
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Shows the inspector.
    pub fn show(self, ui: &mut egui::Ui, state: &mut EditorState) {
        let t = self.theme;
        section(ui, &t, "Clip");
        egui::Grid::new("clip_grid")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                let lab = ui.label(RichText::new("Length").color(t.text_dim));
                let mut d = state.clip.duration;
                if ui
                    .add(
                        egui::DragValue::new(&mut d)
                            .speed(0.01)
                            .range(0.05..=600.0)
                            .suffix(" s"),
                    )
                    .labelled_by(lab.id)
                    .changed()
                {
                    state.clip.duration = d;
                    state.dirty = true;
                }
                ui.end_row();
                let lab = ui.label(RichText::new("Frame rate").color(t.text_dim));
                let mut f = state.clip.frame_rate;
                if ui
                    .add(
                        egui::DragValue::new(&mut f)
                            .speed(1.0)
                            .range(1.0..=240.0)
                            .suffix(" fps"),
                    )
                    .labelled_by(lab.id)
                    .changed()
                {
                    state.clip.frame_rate = f;
                    state.dirty = true;
                }
                ui.end_row();
                ui.label(RichText::new("At the end").color(t.text_dim));
                egui::ComboBox::from_id_salt("loop_mode")
                    .selected_text(match state.clip.loop_mode {
                        LoopMode::Once => "Stop",
                        LoopMode::Loop => "Loop",
                        LoopMode::PingPong => "Ping-pong",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut state.clip.loop_mode, LoopMode::Once, "Stop");
                        ui.selectable_value(&mut state.clip.loop_mode, LoopMode::Loop, "Loop");
                        ui.selectable_value(
                            &mut state.clip.loop_mode,
                            LoopMode::PingPong,
                            "Ping-pong",
                        );
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "At the end")
                    });
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.separator();

        let n = state.selection.len();
        section(ui, &t, if n == 1 { "Key" } else { "Keys" });
        if n == 0 {
            ui.label(RichText::new("Click a key in the timeline to edit it. Drag on empty space to select several.").color(t.text_dim).size(12.0));
            return;
        }
        if let Some(&k) = state.selection.iter().next()
            && n == 1
        {
            egui::Grid::new("key_grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    let lab = ui.label(RichText::new("Time").color(t.text_dim));
                    if let Some(time) = state.key_time(k) {
                        let mut frame = (time * state.clip.frame_rate).round();
                        if ui
                            .add(
                                egui::DragValue::new(&mut frame)
                                    .speed(0.2)
                                    .range(0.0..=100_000.0)
                                    .fixed_decimals(0)
                                    .prefix("frame "),
                            )
                            .labelled_by(lab.id)
                            .changed()
                        {
                            let new_t = frame / state.clip.frame_rate;
                            state.move_selected(new_t - time);
                        }
                    }
                    ui.end_row();
                    if let Some(track) = state.clip.tracks.get(k.track) {
                        let labels = scalar::labels(&track.channel);
                        let values: Vec<Option<f32>> = (0..labels.len())
                            .map(|c| scalar::key_value(&track.channel, k.key, c))
                            .collect();
                        for (c, label) in labels.iter().enumerate() {
                            let lab = ui.label(RichText::new(*label).color(t.text_dim));
                            if let Some(mut v) = values[c]
                                && ui
                                    .add(egui::DragValue::new(&mut v).speed(0.01).max_decimals(3))
                                    .labelled_by(lab.id)
                                    .changed()
                                && let Some(tr) = state.clip.tracks.get_mut(k.track)
                            {
                                scalar::set_key_value(&mut tr.channel, k.key, c, v);
                                state.dirty = true;
                            }
                            ui.end_row();
                        }
                    }
                });
        } else {
            ui.label(RichText::new(format!("{n} keys selected")).color(t.text));
        }
        ui.add_space(6.0);

        section(ui, &t, "Motion to next key");
        let current = state.selected_interpolation();
        ui.horizontal(|ui| {
            for (label, i) in [
                ("Smooth", Interpolation::Bezier),
                ("Linear", Interpolation::Linear),
                ("Step", Interpolation::Constant),
            ] {
                if ui.selectable_label(current == Some(i), label).clicked() {
                    state.set_selected_interpolation(i);
                }
            }
        });
        if matches!(current, Some(Interpolation::Bezier)) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Handles").color(t.text_dim));
                let mode = state.selected_tangent_mode();
                egui::ComboBox::from_id_salt("tangent_mode")
                    .selected_text(mode.map_or("Mixed", TangentMode::label))
                    .show_ui(ui, |ui| {
                        for m in TangentMode::ALL {
                            if ui.selectable_label(mode == Some(m), m.label()).clicked() {
                                state.set_selected_tangent_mode(m);
                            }
                        }
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Handles")
                    });
            });
        }
        ui.add_space(4.0);
        let (style_sel, dir_sel) = match current {
            Some(Interpolation::Eased(s, d)) => (Some(s), d),
            _ => (None, EasingDirection::Out),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Ease").color(t.text_dim));
            for d in EasingDirection::ALL {
                let on = style_sel.is_some() && dir_sel == d;
                if ui.selectable_label(on, d.label()).clicked() {
                    state.set_selected_interpolation(Interpolation::Eased(
                        style_sel.unwrap_or(EasingStyle::Quad),
                        d,
                    ));
                }
            }
        });
        ui.add_space(4.0);
        let cols = 4;
        egui::Grid::new("ease_gallery")
            .spacing([6.0, 6.0])
            .show(ui, |ui| {
                for (i, style) in EasingStyle::ALL.into_iter().enumerate() {
                    if self
                        .ease_tile(ui, style, dir_sel, style_sel == Some(style))
                        .clicked()
                    {
                        state.set_selected_interpolation(Interpolation::Eased(style, dir_sel));
                    }
                    if i % cols == cols - 1 {
                        ui.end_row();
                    }
                }
            });
    }

    fn ease_tile(
        &self,
        ui: &mut egui::Ui,
        style: EasingStyle,
        dir: EasingDirection,
        selected: bool,
    ) -> egui::Response {
        let t = &self.theme;
        let (rect, resp) = ui.allocate_exact_size(vec2(58.0, 52.0), Sense::click());
        let p = ui.painter_at(rect);
        let bg = if selected {
            t.accent.gamma_multiply(0.22)
        } else if resp.hovered() {
            t.raised
        } else {
            t.deep
        };
        p.rect_filled(rect, 5.0, bg);
        p.rect_stroke(
            rect,
            5.0,
            Stroke::new(1.0, if selected { t.accent } else { t.line }),
            StrokeKind::Inside,
        );
        let plot = egui::Rect::from_min_max(rect.min + vec2(9.0, 8.0), rect.max - vec2(9.0, 17.0));
        p.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, t.line));
        p.hline(plot.x_range(), plot.top(), Stroke::new(1.0, t.grid));
        let pts: Vec<_> = (0..=32)
            .map(|i| {
                let x = i as f32 / 32.0;
                let y = ease(style, dir, x);
                pos2(
                    plot.left() + x * plot.width(),
                    plot.bottom() - y * plot.height(),
                )
            })
            .collect();
        let col = if selected {
            t.accent
        } else if resp.hovered() {
            t.text
        } else {
            t.silver
        };
        p.add(egui::Shape::line(pts, Stroke::new(1.5, col)));
        p.text(
            pos2(rect.center().x, rect.bottom() - 8.0),
            egui::Align2::CENTER_CENTER,
            style.label(),
            egui::FontId::proportional(9.5),
            if selected { t.text } else { t.text_dim },
        );
        resp.on_hover_text(format!("{} {}", style.label(), dir.label()))
    }
}
