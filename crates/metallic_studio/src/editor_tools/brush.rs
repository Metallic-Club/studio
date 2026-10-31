//! The scatter brush (B, spec section 9) over the viewport.
//!
//! **Brush set.** "Use selection" takes up to [`MAX_BRUSH_ITEMS`] selected Parts or Models as
//! the items (weight 5 each; B with a selection does it too). Each item's size is its
//! [`geom::snapshot`] bounding box.
//!
//! **Stroke.** Left-drag paints, Shift-drag erases. Each frame is one [`Stroke::dab`] at the
//! surface under the cursor; the scene is probed with the viewport's picker. Release ends the
//! stroke: a paint stroke copies the items to their placements under
//! `<parent>/<set name>` and an erase stroke deletes the instances it removed, each as one undo
//! step. Until instance tags (#78) exist, an instance carries the tag `Brush:<set>` when it sits
//! in that set's folder, so erase never touches anything the brush did not place.
//!
//! **Preview.** The brush disk is a silver circle on the surface with an inner ring at 80%; the
//! stroke's placements so far show as small dots. `[` and `]` change the radius.
use eframe::egui::{self, Pos2, Stroke as Line};
use glam::{DQuat, DVec3};
use metallic_core::{InstanceId, Value};

use super::commit::{self, Placed, find_set_folder};
use super::{Modal, ViewContext, overlay, report};
use crate::editor::EditorState;
use crate::picking::PickCache;
use crate::theme;
use crate::tools::arrange::CopiedPart;
use crate::tools::geom;
use crate::tools::scatter::{
    BrushInstance, BrushItem, BrushSet, BrushSettings, DWELL_STEP_SECONDS, MAX_BRUSH_ITEMS, Stroke,
    StrokeMode, SurfaceHit, SurfaceProbe,
};
use crate::transform::Ray;

/// One brush item and the instance it copies.
#[derive(Debug, Clone)]
pub struct Item {
    /// The Part or Model copied.
    pub source: InstanceId,
    /// Its parts and their snapshots when the set was made.
    pub parts: Vec<geom::PartSnapshot>,
    /// The bounding box centre the placement's pivot refers to.
    pub centre: DVec3,
}

/// The brush set, its settings and the stroke in progress.
#[derive(Default)]
pub struct BrushState {
    /// The set in use (`None` until "Use selection").
    pub set: Option<(BrushSet, Vec<Item>)>,
    /// Radius, density, spacing and the rest (header fields).
    pub settings: BrushSettings,
    stroke: Option<Stroke<InstanceId>>,
    /// Stroke number, mixed into the seed so two strokes differ.
    strokes: u64,
}

/// Makes the brush set from the selection. Returns false (and says why) when nothing fits.
pub fn use_selection(b: &mut BrushState, state: &mut EditorState) -> bool {
    let mut items = Vec::new();
    let mut entries = Vec::new();
    for &id in state.selection.ids().iter().take(MAX_BRUSH_ITEMS) {
        let part_ids = crate::selection::Selection::parts_of(&state.world, id);
        let parts = geom::snapshot(&state.world, &part_ids);
        let Some(bounds) = parts.iter().map(|p| p.aabb()).reduce(|a, b| a.union(&b)) else {
            continue;
        };
        items.push(BrushItem::new(state.world.full_name(id), 5, bounds.size()));
        entries.push(Item {
            source: id,
            parts,
            centre: (bounds.min + bounds.max) * 0.5,
        });
    }
    let name = entries
        .first()
        .map_or("Brush".to_string(), |e| state.display_name(e.source));
    match BrushSet::new(name, items) {
        Ok(set) => {
            state.note(format!(
                "Brush set: {} ({} items)",
                set.name(),
                entries.len()
            ));
            b.set = Some((set, entries));
            true
        }
        Err(_) => {
            report(
                state,
                "Brush: select the Parts or Models to paint with, then Use selection",
            );
            false
        }
    }
}

/// Ray casts against the scene's parts for the brush.
struct Probe<'a> {
    picks: &'a PickCache,
    state: &'a EditorState,
    set_folder: Option<InstanceId>,
    tag: String,
}

/// The outward normal of a part's box at a surface point.
fn box_normal(picks: &PickCache, id: InstanceId, point: DVec3) -> DVec3 {
    let Some(t) = picks.transform(id) else {
        return DVec3::Y;
    };
    let local = t.rotation.inverse() * (point - t.position);
    let half = (t.size * 0.5).max(DVec3::splat(1e-6));
    let r = local / half;
    let axis = if r.x.abs() >= r.y.abs() && r.x.abs() >= r.z.abs() {
        DVec3::X * r.x.signum()
    } else if r.y.abs() >= r.z.abs() {
        DVec3::Y * r.y.signum()
    } else {
        DVec3::Z * r.z.signum()
    };
    t.rotation * axis
}

impl Probe<'_> {
    fn in_set(&self, id: InstanceId) -> bool {
        let Some(folder) = self.set_folder else {
            return false;
        };
        let mut cur = self.state.world.parent(id);
        while let Some(c) = cur {
            if c == folder {
                return true;
            }
            cur = self.state.world.parent(c);
        }
        false
    }
}

impl SurfaceProbe for Probe<'_> {
    fn raycast(&self, origin: DVec3, direction: DVec3, max_distance: f64) -> Option<SurfaceHit> {
        let hit = self.picks.first(
            &Ray {
                origin,
                dir: direction,
            },
            &[],
        )?;
        if hit.distance > max_distance {
            return None;
        }
        let point = origin + direction * hit.distance;
        let material = match self.state.world.get_ref(hit.id, "Material") {
            Ok(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        let tags = if self.in_set(hit.id) {
            vec![self.tag.clone()]
        } else {
            Vec::new()
        };
        Some(SurfaceHit {
            point,
            normal: box_normal(self.picks, hit.id, point),
            material,
            tags,
        })
    }
}

/// The brush's own instances (the top-level children of its set folder).
fn existing(
    state: &EditorState,
    settings: &BrushSettings,
    set: &BrushSet,
) -> Vec<BrushInstance<InstanceId>> {
    let Some(folder) = find_set_folder(&state.world, &settings.parent, set.name()) else {
        return Vec::new();
    };
    state
        .world
        .children(folder)
        .iter()
        .filter(|&&id| state.is_live(id))
        .filter_map(|&id| {
            let parts = crate::selection::Selection::parts_of(&state.world, id);
            let b = geom::snapshot(&state.world, &parts)
                .iter()
                .map(|p| p.aabb())
                .reduce(|a, b| a.union(&b))?;
            Some(BrushInstance {
                id,
                // A brush instance stands on its surface point: the bottom centre of its box.
                position: DVec3::new(
                    (b.min.x + b.max.x) * 0.5,
                    b.min.y,
                    (b.min.z + b.max.z) * 0.5,
                ),
                tags: vec![set.tag()],
            })
        })
        .collect()
}

/// Where each part of `item` goes for a placement (pivot at the box centre).
pub fn place_item(item: &Item, position: DVec3, rotation: DQuat, scale: f64) -> Vec<CopiedPart> {
    let v = |d: DVec3| metallic_core::Vec3::new(d.x, d.y, d.z);
    item.parts
        .iter()
        .map(|p| {
            let rot = rotation * p.rotation;
            let (y, x, z) = rot.to_euler(glam::EulerRot::YXZ);
            CopiedPart {
                source_part: p.id,
                position: v(position + rotation * ((p.position - item.centre) * scale)),
                orientation: v(DVec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())),
                size: v(p.size * scale),
            }
        })
        .collect()
}

fn end_stroke(state: &mut EditorState, b: &mut BrushState) {
    let (Some(stroke), Some((_, items))) = (b.stroke.take(), b.set.as_ref()) else {
        return;
    };
    let Some(done) = stroke.finish() else { return };
    let name = done.undo_name();
    if !done.removed.is_empty() {
        let cmd = crate::commands::Command::Batch {
            name,
            commands: vec![crate::commands::Command::delete(&done.removed)],
        };
        let _ = state.apply(cmd);
        return;
    }
    let placed: Vec<Placed> = done
        .added
        .iter()
        .filter_map(|p| {
            let item = items.get(p.item)?;
            Some(Placed {
                source: item.source,
                parts: place_item(item, p.position, p.rotation, p.scale),
            })
        })
        .collect();
    if let Err(e) = commit::commit_placements(state, &done.parent, &done.set_name, &placed, name) {
        report(state, e);
    }
}

/// Painting, erasing and the brush preview.
pub fn viewport(
    ui: &egui::Ui,
    state: &mut EditorState,
    cx: &ViewContext<'_>,
    painter: &egui::Painter,
    b: &mut BrushState,
) {
    let (pressed, down, shift, dt) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.modifiers.shift,
            f64::from(i.stable_dt.min(0.1)),
        )
    });
    if cx.pointer.is_some() {
        let (bigger, smaller) = ui.ctx().input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::CloseBracket),
                i.consume_key(egui::Modifiers::NONE, egui::Key::OpenBracket),
            )
        });
        if bigger || smaller {
            let r = b.settings.radius * if bigger { 1.25 } else { 0.8 };
            b.settings.radius = r.clamp(1.0, 256.0);
        }
    }
    let under = cx.pointer.and_then(|p| {
        let ray = cx.ray(p);
        let hit = cx.picks.first(&ray, &[])?;
        let point = ray.at(hit.distance);
        Some((point, box_normal(cx.picks, hit.id, point)))
    });
    let tag = b.set.as_ref().map(|(s, _)| s.tag()).unwrap_or_default();
    let set_folder = b
        .set
        .as_ref()
        .and_then(|(s, _)| find_set_folder(&state.world, &b.settings.parent, s.name()));

    if pressed && cx.pointer.is_some() {
        match &b.set {
            None => {
                report(
                    state,
                    "Brush: select the Parts or Models to paint with, then Use selection",
                );
            }
            Some((set, _)) => {
                b.strokes += 1;
                let settings = BrushSettings {
                    seed: b.settings.seed.wrapping_add(b.strokes),
                    ..b.settings.clone()
                };
                let mode = if shift {
                    StrokeMode::Erase
                } else {
                    StrokeMode::Paint
                };
                let near = existing(state, &b.settings, set);
                b.stroke = Some(Stroke::begin(set.clone(), &settings, mode, near));
                if let (Some(stroke), Some((p, n))) = (&mut b.stroke, under) {
                    let probe = Probe {
                        picks: cx.picks,
                        state,
                        set_folder,
                        tag: tag.clone(),
                    };
                    stroke.dab(&probe, p, n, DWELL_STEP_SECONDS);
                }
            }
        }
    } else if down && b.stroke.is_some() {
        if let (Some(stroke), Some((p, n))) = (&mut b.stroke, under) {
            let probe = Probe {
                picks: cx.picks,
                state,
                set_folder,
                tag,
            };
            stroke.dab(&probe, p, n, dt);
        }
        ui.ctx().request_repaint();
    } else if !down && b.stroke.is_some() {
        end_stroke(state, b);
    }

    // Preview: the disk on the surface, the stroke's placements and erased instances.
    if let Some((p, n)) = under {
        let r = b.settings.clamped().radius;
        overlay::surface_circle(painter, cx, p, n, r, Line::new(1.5, theme::text_2()));
        overlay::surface_circle(painter, cx, p, n, r * 0.8, Line::new(1.0, theme::text_3()));
    }
    if let Some(stroke) = &b.stroke {
        for pl in stroke.added() {
            if let Some(s) = cx.project(pl.surface_point) {
                painter.circle_filled(s, 3.0, theme::accent());
            }
        }
        for &id in stroke.removed() {
            if let Some(t) = cx.picks.transform(id)
                && let Some(s) = cx.project(t.position)
            {
                painter.circle_stroke(s, 4.0, Line::new(1.5, theme::error()));
            }
        }
    }
}

/// The brush header: set, radius, density, spacing, scale, yaw, alignment, slope, parent.
pub fn header_window(
    ctx: &egui::Context,
    state: &mut EditorState,
    b: &mut BrushState,
    modal: &mut Option<Modal>,
) {
    let mut open = true;
    egui::Window::new("Brush")
        .collapsible(true)
        .resizable(false)
        .open(&mut open)
        .default_pos(Pos2::new(320.0, 90.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let text = match &b.set {
                    Some((s, items)) => format!("{} ({} items)", s.name(), items.len()),
                    None => "No brush set".into(),
                };
                ui.label(egui::RichText::new(text).color(theme::text_2()));
                if crate::theme::add_enabled(
                    ui,
                    !state.selection.is_empty(),
                    egui::Button::new("Use selection"),
                )
                .clicked()
                {
                    use_selection(b, state);
                }
            });
            let s = &mut b.settings;
            egui::Grid::new("brush-settings")
                .num_columns(2)
                .show(ui, |ui| {
                    let row = |ui: &mut egui::Ui,
                               label: &str,
                               v: &mut f64,
                               range: std::ops::RangeInclusive<f64>,
                               speed: f64| {
                        ui.label(label);
                        ui.add(egui::DragValue::new(v).range(range).speed(speed));
                        ui.end_row();
                    };
                    row(ui, "Radius ([ ])", &mut s.radius, 1.0..=256.0, 0.5);
                    row(ui, "Density /100 st²", &mut s.density, 0.1..=50.0, 0.1);
                    row(ui, "Scale min", &mut s.scale_min, 0.01..=100.0, 0.01);
                    row(ui, "Scale max", &mut s.scale_max, 0.01..=100.0, 0.01);
                    row(ui, "Yaw min °", &mut s.yaw_min_deg, -360.0..=360.0, 1.0);
                    row(ui, "Yaw max °", &mut s.yaw_max_deg, -360.0..=360.0, 1.0);
                    row(
                        ui,
                        "Align to normal",
                        &mut s.align_to_normal,
                        0.0..=1.0,
                        0.01,
                    );
                    row(ui, "Max slope °", &mut s.max_slope_deg, 0.0..=90.0, 0.5);
                    ui.label("Parent");
                    ui.text_edit_singleline(&mut s.parent);
                    ui.end_row();
                });
            ui.label(
                egui::RichText::new("Drag paints · Shift-drag erases this brush · Esc leaves")
                    .small()
                    .color(theme::text_3()),
            );
        });
    if !open {
        *modal = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_item_keeps_the_box_base_on_the_point_for_a_flat_part() {
        let mut world = metallic_core::World::new_with_services();
        let ws = world.root("Workspace").unwrap();
        let part = world.create("Part", Some(ws)).unwrap();
        let snap = geom::snapshot(&world, &[part]);
        let item = Item {
            source: part,
            centre: snap[0].position,
            parts: snap,
        };
        let placed = place_item(&item, DVec3::new(10.0, 2.0, 0.0), DQuat::IDENTITY, 2.0);
        assert_eq!(placed[0].position, metallic_core::Vec3::new(10.0, 2.0, 0.0));
        assert!(placed[0].size.x > item.parts[0].size.x);
    }
}
