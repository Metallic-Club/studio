//! The demo editor: two sample clips playing on a 2D bone rig and a blocky 3D figure at once,
//! with the dope sheet, curve editor, transport bar and key inspector around them.
//!
//! The same clip drives both rigs: joint names match, and a number track on a joint's rotation
//! turns it about +Z, which is the screen plane for the 2D rig and the frontal plane for the 3D
//! figure. That is the "2D, 3D and in between" model in one picture.

use crate::preview::{self, BoxPart, OrbitCamera};
use crate::state::EditorState;
use crate::theme::Theme;
use crate::{CurveEditor, DopeSheet, KeyInspector, TransportBar};
use egui::{Color32, RichText, Stroke, vec2};
use metallic_anim::glam::{Quat, Vec2, Vec3};
use metallic_anim::{
    AnimError, Bone2dBuilder, Channel, ChannelValue, Clip, ClipBinding, Color3, Curve,
    EasingDirection, EasingStyle, Interpolation, Joint, JointChannel, Keyframe, LoopMode, Pose,
    Skeleton, TrackTarget, Transform, sample_clip,
};
use std::f32::consts::{FRAC_PI_2, PI};

/// Which editor is shown under the transport bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTab {
    /// Dope sheet.
    DopeSheet,
    /// Curve editor.
    Curves,
}

/// The 3D figure's skeleton: Root (hips), Torso, Head, two arms, two legs. Units are studs.
///
/// # Errors
/// Never in practice; the joints are fixed.
pub fn figure_skeleton() -> Result<Skeleton, AnimError> {
    let j = |name: &str, parent: Option<usize>, at: Vec3| Joint {
        name: name.into(),
        parent,
        rest: Transform::from_translation(at),
        length: 0.0,
    };
    Skeleton::new(vec![
        j("Root", None, Vec3::new(0.0, 2.0, 0.0)),
        j("Torso", Some(0), Vec3::ZERO),
        j("Head", Some(1), Vec3::new(0.0, 2.0, 0.0)),
        j("LeftArm", Some(1), Vec3::new(1.5, 1.9, 0.0)),
        j("RightArm", Some(1), Vec3::new(-1.5, 1.9, 0.0)),
        j("LeftLeg", Some(0), Vec3::new(0.5, 0.0, 0.0)),
        j("RightLeg", Some(0), Vec3::new(-0.5, 0.0, 0.0)),
    ])
}

/// The six boxes of the 3D figure.
pub fn figure_parts(s: &Skeleton) -> Vec<BoxPart> {
    let silver = Color32::from_rgb(0xb8, 0xb5, 0xc2);
    let graphite = Color32::from_rgb(0x55, 0x50, 0x60);
    let torso = Color32::from_rgb(0x8a, 0x74, 0xc2);
    let part = |name: &str, center: Vec3, size: Vec3, color| {
        s.find(name).map(|joint| BoxPart {
            joint,
            center,
            size,
            color,
        })
    };
    [
        part(
            "Torso",
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(2.0, 2.0, 1.0),
            torso,
        ),
        part(
            "Head",
            Vec3::new(0.0, 0.62, 0.0),
            Vec3::new(1.2, 1.2, 1.2),
            silver,
        ),
        part(
            "LeftArm",
            Vec3::new(0.0, -0.9, 0.0),
            Vec3::new(0.95, 2.0, 0.95),
            silver,
        ),
        part(
            "RightArm",
            Vec3::new(0.0, -0.9, 0.0),
            Vec3::new(0.95, 2.0, 0.95),
            silver,
        ),
        part(
            "LeftLeg",
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.98, 2.0, 0.98),
            graphite,
        ),
        part(
            "RightLeg",
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.98, 2.0, 0.98),
            graphite,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The 2D rig: same joint names as the 3D figure plus forearms and shins. Bones point along
/// their local +X; limbs rest pointing down.
///
/// # Errors
/// Never in practice; the bones are fixed.
pub fn bone_rig_2d() -> Result<Skeleton, AnimError> {
    let mut b = Bone2dBuilder::new();
    let root = b.root("Root", Vec2::new(0.0, 2.0), 0.0, 0.0);
    let torso = b.child_at("Torso", root, Vec2::ZERO, FRAC_PI_2, 2.0);
    b.child("Head", torso, 0.0, 1.2);
    // Torso-local: +X is up, +Y is the figure's right side of the screen (-X in world).
    let la = b.child_at("LeftArm", torso, Vec2::new(1.85, -1.0), PI, 1.05);
    b.child("LeftForearm", la, 0.0, 0.95);
    let ra = b.child_at("RightArm", torso, Vec2::new(1.85, 1.0), PI, 1.05);
    b.child("RightForearm", ra, 0.0, 0.95);
    let ll = b.child_at("LeftLeg", root, Vec2::new(0.5, 0.0), -FRAC_PI_2, 1.0);
    b.child("LeftShin", ll, 0.0, 1.0);
    let rl = b.child_at("RightLeg", root, Vec2::new(-0.5, 0.0), -FRAC_PI_2, 1.0);
    b.child("RightShin", rl, 0.0, 1.0);
    b.build()
}

fn f32_curve(keys: &[(f32, f32)]) -> Result<Curve<f32>, AnimError> {
    Curve::from_keys(keys.iter().map(|&(t, v)| Keyframe::new(t, v)).collect())
}

fn rot(joint: &str) -> TrackTarget {
    TrackTarget::joint(joint, JointChannel::Rotation)
}

/// "Wave": hello wave with a bob and sway, using number, vector, colour and switch tracks.
///
/// # Errors
/// Never in practice; the keys are fixed.
pub fn wave_clip() -> Result<Clip, AnimError> {
    let bob = Curve::from_keys(
        [
            (0.0, 0.0),
            (0.5, -0.14),
            (1.0, 0.0),
            (1.5, -0.14),
            (2.0, 0.0),
        ]
        .iter()
        .map(|&(t, y)| Keyframe::new(t, Vec3::new(0.0, y, 0.0)))
        .collect(),
    )?;
    let mut head = f32_curve(&[(0.0, 0.0), (0.6, -0.16), (1.3, 0.12), (2.0, 0.0)])?;
    if let Some(k) = head.key_mut(1) {
        k.interpolation = Interpolation::Eased(EasingStyle::Sine, EasingDirection::InOut);
    }
    if let Some(k) = head.key_mut(2) {
        k.interpolation = Interpolation::Eased(EasingStyle::Back, EasingDirection::Out);
    }
    let color = Curve::from_keys(vec![
        Keyframe::new(0.0, Color3::from_rgb(138, 116, 194)),
        Keyframe::new(1.0, Color3::from_rgb(170, 150, 222)),
        Keyframe::new(2.0, Color3::from_rgb(138, 116, 194)),
    ])?;
    let blink = Curve::from_keys(vec![
        Keyframe::new(0.0, false),
        Keyframe::new(1.1, true),
        Keyframe::new(1.25, false),
    ])?;
    let mut clip = Clip::new("Wave", 2.0)
        .with_track(
            TrackTarget::joint("Root", JointChannel::Translation),
            Channel::Vec3(bob),
        )
        .with_track(
            rot("Torso"),
            Channel::F32(f32_curve(&[
                (0.0, 0.0),
                (0.5, 0.07),
                (1.2, -0.05),
                (2.0, 0.0),
            ])?),
        )
        .with_track(rot("Head"), Channel::F32(head))
        .with_track(
            rot("LeftArm"),
            Channel::F32(f32_curve(&[
                (0.0, 0.12),
                (0.4, 2.45),
                (0.7, 2.85),
                (1.0, 2.25),
                (1.3, 2.85),
                (1.6, 2.4),
                (2.0, 0.12),
            ])?),
        )
        .with_track(
            rot("LeftForearm"),
            Channel::F32(f32_curve(&[
                (0.0, 0.0),
                (0.55, 0.45),
                (0.85, -0.35),
                (1.15, 0.45),
                (1.45, -0.35),
                (2.0, 0.0),
            ])?),
        )
        .with_track(
            rot("RightArm"),
            Channel::F32(f32_curve(&[(0.0, -0.12), (1.0, -0.3), (2.0, -0.12)])?),
        )
        .with_track(
            rot("LeftLeg"),
            Channel::F32(f32_curve(&[(0.0, 0.0), (1.0, 0.06), (2.0, 0.0)])?),
        )
        .with_track(
            rot("RightLeg"),
            Channel::F32(f32_curve(&[(0.0, 0.0), (1.0, 0.06), (2.0, 0.0)])?),
        )
        .with_track(
            TrackTarget::property("Torso", "Color"),
            Channel::Color3(color),
        )
        .with_track(TrackTarget::property("Head", "Blink"), Channel::Bool(blink))
        .with_marker(0.4, "WaveStart")
        .with_marker(1.6, "WaveEnd");
    clip.loop_mode = LoopMode::Loop;
    Ok(clip)
}

/// "Walk": a one-second walk cycle with rotation (quaternion) tracks swinging limbs forward and
/// back, a bob, and footstep markers.
///
/// # Errors
/// Never in practice; the keys are fixed.
pub fn walk_clip() -> Result<Clip, AnimError> {
    let swing = |phase: f32, amp: f32| -> Result<Curve<Quat>, AnimError> {
        Curve::from_keys(
            (0..=4)
                .map(|i| {
                    let t = i as f32 * 0.25;
                    let a = (2.0 * PI * (t + phase)).sin() * amp;
                    Keyframe::new(t, Quat::from_rotation_x(a))
                })
                .collect(),
        )
    };
    let bob = Curve::from_keys(
        (0..=4)
            .map(|i| {
                let t = i as f32 * 0.25;
                Keyframe::new(
                    t,
                    Vec3::new(0.0, if i % 2 == 0 { -0.12 } else { 0.06 }, 0.0),
                )
            })
            .collect(),
    )?;
    let mut clip = Clip::new("Walk", 1.0)
        .with_track(
            TrackTarget::joint("Root", JointChannel::Translation),
            Channel::Vec3(bob),
        )
        .with_track(rot("Torso"), Channel::Quat(swing(0.25, 0.05)?))
        .with_track(rot("LeftArm"), Channel::Quat(swing(0.0, 0.6)?))
        .with_track(rot("RightArm"), Channel::Quat(swing(0.5, 0.6)?))
        .with_track(rot("LeftLeg"), Channel::Quat(swing(0.5, 0.55)?))
        .with_track(rot("RightLeg"), Channel::Quat(swing(0.0, 0.55)?))
        .with_marker(0.0, "Footstep")
        .with_marker(0.5, "Footstep");
    clip.loop_mode = LoopMode::Loop;
    Ok(clip)
}

/// The demo application state. Embed with [`DemoApp::ui`] in any egui host.
#[derive(Debug, Clone)]
pub struct DemoApp {
    theme: Theme,
    clips: Vec<Clip>,
    current: usize,
    /// The editor state (public so hosts and tests can drive it).
    pub state: EditorState,
    rig2d: Skeleton,
    figure: Skeleton,
    parts: Vec<BoxPart>,
    camera: OrbitCamera,
    /// Which editor shows under the transport bar.
    pub tab: EditorTab,
    theme_applied: bool,
}

impl DemoApp {
    /// Builds the demo with its sample clips.
    ///
    /// # Errors
    /// Only if the built-in sample data were invalid.
    pub fn new() -> Result<Self, AnimError> {
        let clips = vec![wave_clip()?, walk_clip()?];
        let figure = figure_skeleton()?;
        let parts = figure_parts(&figure);
        let mut state = EditorState::new(clips[0].clone());
        state.active_track = Some(3);
        Ok(Self {
            theme: Theme::default(),
            clips,
            current: 0,
            state,
            rig2d: bone_rig_2d()?,
            figure,
            parts,
            camera: OrbitCamera::default(),
            tab: EditorTab::DopeSheet,
            theme_applied: false,
        })
    }

    /// Switches to sample clip `index` (keeping edits made to the current one).
    pub fn select_clip(&mut self, index: usize) {
        if index == self.current || index >= self.clips.len() {
            return;
        }
        self.clips[self.current] = self.state.clip.clone();
        self.current = index;
        let mut s = EditorState::new(self.clips[index].clone());
        s.time_view = self.state.time_view;
        s.snap = self.state.snap;
        s.looping = self.state.looping;
        s.playing = self.state.playing;
        self.state = s;
    }

    /// Draws the whole editor into `ui`.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.theme_applied {
            crate::theme::apply(ui.ctx());
            self.theme_applied = true;
        }
        // Read the palette each frame so a theme switch shows at once.
        self.theme = Theme::current();
        let t = self.theme;
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        self.state.tick(dt);
        if self.state.playing {
            ui.ctx().request_repaint();
        }
        self.state.dirty = false;

        egui::Panel::top("anim_header")
            .exact_size(46.0)
            .frame(
                egui::Frame::new()
                    .fill(t.deep)
                    .inner_margin(egui::Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| self.header(ui));
        egui::Panel::right("anim_inspector")
            .default_size(292.0)
            .frame(
                egui::Frame::new()
                    .fill(t.panel)
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .stroke(Stroke::new(1.0, t.line)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .show(ui, |ui| KeyInspector::new(t).show(ui, &mut self.state));
            });
        egui::Panel::bottom("anim_editor")
            .default_size(356.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(t.base))
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(t.panel)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.set_height(30.0);
                        ui.horizontal(|ui| {
                            self.tabs(ui);
                            ui.add_space(16.0);
                            TransportBar::new(t).show(ui, &mut self.state);
                        });
                    });
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.min_rect().bottom(),
                    Stroke::new(1.0, t.line),
                );
                match self.tab {
                    EditorTab::DopeSheet => {
                        DopeSheet::new(t).show(ui, &mut self.state);
                    }
                    EditorTab::Curves => {
                        CurveEditor::new(t).show(ui, &mut self.state);
                    }
                }
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(t.base)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| self.previews(ui));
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.horizontal_centered(|ui| {
            // Emblem: a simple faceted mark (no letter, no glow).
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
            let c = r.center();
            let pts = vec![
                c + vec2(0.0, -8.0),
                c + vec2(7.0, 0.0),
                c + vec2(0.0, 8.0),
                c + vec2(-7.0, 0.0),
            ];
            ui.painter()
                .add(egui::Shape::convex_polygon(pts, t.silver, Stroke::NONE));
            ui.painter().add(egui::Shape::convex_polygon(
                vec![c + vec2(0.0, -8.0), c + vec2(7.0, 0.0), c],
                t.accent,
                Stroke::NONE,
            ));
            ui.add_space(4.0);
            ui.label(RichText::new("Metallic Studio").size(13.5).color(t.text));
            ui.label(RichText::new("/").color(t.text_meta));
            ui.label(RichText::new("Animation").size(13.5).color(t.text_dim));
            ui.add_space(22.0);
            let names: Vec<String> = self.clips.iter().map(|c| c.name.clone()).collect();
            for (i, name) in names.iter().enumerate() {
                let on = i == self.current;
                let text =
                    RichText::new(name)
                        .size(12.5)
                        .color(if on { t.text } else { t.text_dim });
                if ui.selectable_label(on, text).clicked() {
                    self.select_clip(i);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new("One clip drives a 2D bone rig and 3D joints")
                        .size(12.0)
                        .color(t.text_meta),
                );
            });
        });
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        for (label, tab) in [
            ("Dope sheet", EditorTab::DopeSheet),
            ("Curves", EditorTab::Curves),
        ] {
            if ui
                .selectable_label(self.tab == tab, RichText::new(label).size(12.5))
                .clicked()
            {
                self.tab = tab;
                if tab == EditorTab::Curves && self.state.drag.is_none() {
                    self.state.frame_all_values();
                }
            }
        }
    }

    fn previews(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let clip = &self.state.clip;
        let time = clip.local_time(self.state.time);
        let highlight_joint = self
            .state
            .active_track
            .and_then(|i| clip.tracks.get(i))
            .and_then(|tr| match &tr.target {
                TrackTarget::Joint { joint, .. } => Some(joint.clone()),
                TrackTarget::Property { .. } => None,
            });

        let mut pose2 = Pose::rest(&self.rig2d);
        sample_clip(
            clip,
            &ClipBinding::new(clip, &self.rig2d),
            &self.rig2d,
            time,
            &mut pose2,
        );
        let mut pose3 = Pose::rest(&self.figure);
        sample_clip(
            clip,
            &ClipBinding::new(clip, &self.figure),
            &self.figure,
            time,
            &mut pose3,
        );
        // Property tracks: the torso colour follows a Color3 track when there is one.
        let mut parts = self.parts.clone();
        for (path, prop, value) in clip.sample_properties(time) {
            if let (true, ChannelValue::Color3(c)) = (path == "Torso" && prop == "Color", value)
                && let Some(torso) = self.figure.find("Torso")
            {
                let [r, g, b] = c.to_rgb();
                for p in parts.iter_mut().filter(|p| p.joint == torso) {
                    p.color = Color32::from_rgb(r, g, b);
                }
            }
        }

        let rect = ui.available_rect_before_wrap();
        let gap = 12.0;
        let w = (rect.width() - gap) / 2.0;
        let left = egui::Rect::from_min_size(rect.min, vec2(w, rect.height()));
        let right =
            egui::Rect::from_min_size(rect.min + vec2(w + gap, 0.0), vec2(w, rect.height()));
        let h2 = highlight_joint.as_deref().and_then(|n| self.rig2d.find(n));
        let h3 = highlight_joint.as_deref().and_then(|n| self.figure.find(n));
        preview::rig_2d(ui, left, &t, &self.rig2d, &pose2, h2);
        if !clip_moves_2d(clip) {
            ui.painter().text(
                left.center_bottom() + vec2(0.0, -14.0),
                egui::Align2::CENTER_BOTTOM,
                "This clip turns joints about X; 2D bones only turn in the picture plane (Z).",
                egui::FontId::proportional(12.0),
                t.text_meta,
            );
        }
        preview::figure_3d(
            ui,
            right,
            &t,
            &self.figure,
            &pose3,
            &parts,
            &mut self.camera,
            h3,
        );
        if self.state.recording {
            // Record keys is on: the viewports get an `error` outline so it cannot be missed.
            for r in [left, right] {
                ui.painter().rect_stroke(
                    r,
                    f32::from(crate::theme::R_SM),
                    Stroke::new(2.0, t.error),
                    egui::StrokeKind::Inside,
                );
            }
            ui.painter().text(
                right.right_top() + vec2(-12.0, 10.0),
                egui::Align2::RIGHT_TOP,
                "Recording keys",
                egui::FontId::proportional(12.0),
                t.error,
            );
        }
    }
}

/// True when the clip has a track that visibly moves a 2D bone: a rotation in the picture plane
/// (f32, or a quaternion with a Z component) or a translation.
fn clip_moves_2d(clip: &Clip) -> bool {
    clip.tracks
        .iter()
        .any(|tr| match (&tr.target, &tr.channel) {
            (
                TrackTarget::Joint {
                    channel: JointChannel::Rotation,
                    ..
                },
                Channel::F32(_),
            ) => true,
            (
                TrackTarget::Joint {
                    channel: JointChannel::Rotation,
                    ..
                },
                Channel::Quat(c),
            ) => c.keys().iter().any(|k| k.value.z.abs() > 1e-3),
            (
                TrackTarget::Joint {
                    channel: JointChannel::Translation,
                    joint,
                },
                _,
            ) => joint != "Root",
            _ => false,
        })
}
