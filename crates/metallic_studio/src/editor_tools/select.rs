//! Lasso and paint selection over the viewport, and the Select similar popover (spec 10.1).
//!
//! Box selection stays with the viewport. In Lasso and Paint mode the trail is collected here;
//! on release every Workspace part becomes a [`ScreenItem`] (its [`geom::snapshot`] box
//! projected to the screen), [`lasso_hits`] or [`paint_hits`] picks the hits, and [`combine`]
//! applies Shift (add) and Ctrl (remove). A click without travel selects the part under the
//! cursor, or clears.
use eframe::egui::{self, Pos2, Stroke};
use glam::DVec2;
use metallic_core::{InstanceId, Value};

use super::{Popover, ViewContext};
use crate::editor::EditorState;
use crate::theme;
use crate::tools::geom;
use crate::tools::select::{
    PAINT_BRUSH_DIAMETER_PX, PartTraits, ScreenItem, ScreenRect, SelectMode, SelectOp, SimilarKey,
    combine, lasso_hits, paint_hits, select_similar,
};

/// A lasso or paint trail in progress.
#[derive(Debug, Default, Clone)]
pub struct Gesture {
    trail: Vec<Pos2>,
    active: bool,
}

/// Every BasePart under Workspace.
pub fn workspace_parts(state: &EditorState) -> Vec<InstanceId> {
    let Some(ws) = state.world.root("Workspace") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    crate::selection::collect_parts(&state.world, ws, &mut out, &mut seen);
    out
}

/// Screen items for every Workspace part in front of the camera.
pub fn screen_items(state: &EditorState, cx: &ViewContext<'_>) -> Vec<ScreenItem<InstanceId>> {
    let ids = workspace_parts(state);
    geom::snapshot(&state.world, &ids)
        .into_iter()
        .filter_map(|s| {
            let pts: Vec<DVec2> = s
                .corners()
                .iter()
                .map(|c| cx.view.project(*c).map(|(x, y)| DVec2::new(x, y)))
                .collect::<Option<Vec<_>>>()?;
            let (x, y) = cx.view.project(s.position)?;
            let rect = pts
                .iter()
                .fold(ScreenRect::from_corners(pts[0], pts[0]), |r, p| {
                    ScreenRect::from_corners(r.min.min(*p), r.max.max(*p))
                });
            let locked = matches!(state.world.get_ref(s.id, "Locked"), Ok(Value::Bool(true)));
            Some(ScreenItem {
                id: s.id,
                rect,
                centre: DVec2::new(x, y),
                locked,
            })
        })
        .collect()
}

/// Lasso or paint gesture handling and painting.
pub fn viewport(
    ui: &egui::Ui,
    state: &mut EditorState,
    cx: &ViewContext<'_>,
    painter: &egui::Painter,
    mode: SelectMode,
    g: &mut Gesture,
) {
    let (pressed, down, mods) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.modifiers,
        )
    });
    let pos = ui.input(|i| i.pointer.interact_pos());
    if pressed && let Some(p) = cx.pointer {
        g.trail = vec![p];
        g.active = true;
    }
    if g.active {
        if let Some(p) = pos
            && g.trail.last().is_none_or(|l| (*l - p).length() > 2.0)
        {
            g.trail.push(p);
        }
        if !down {
            g.active = false;
            finish(
                state,
                cx,
                mode,
                g,
                SelectOp::from_modifiers(mods.shift, mods.command),
            );
        }
    }
    let stroke = Stroke::new(1.5, theme::accent());
    if g.active && g.trail.len() > 1 {
        let mut pts = g.trail.clone();
        if mode == SelectMode::Lasso {
            pts.push(g.trail[0]);
        }
        painter.add(egui::Shape::line(pts, stroke));
    }
    if mode == SelectMode::Paint
        && let Some(p) = cx.pointer
    {
        painter.circle_stroke(p, (PAINT_BRUSH_DIAMETER_PX * 0.5) as f32, stroke);
    }
}

fn finish(
    state: &mut EditorState,
    cx: &ViewContext<'_>,
    mode: SelectMode,
    g: &mut Gesture,
    op: SelectOp,
) {
    let trail: Vec<DVec2> = g
        .trail
        .drain(..)
        .map(|p| {
            let (x, y) = cx.local(p);
            DVec2::new(x, y)
        })
        .collect();
    let travel = trail.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>();
    let hits = if travel < 4.0 && mode == SelectMode::Lasso {
        // A click: the part under the cursor.
        cx.hovered.into_iter().collect()
    } else {
        let items = screen_items(state, cx);
        match mode {
            SelectMode::Lasso => lasso_hits(&items, &trail),
            _ => paint_hits(&items, &trail),
        }
    };
    let next = combine(state.selection.ids(), &hits, op);
    state.selection.set(next);
}

/// What Select similar knows about each Workspace part.
pub fn part_traits(state: &EditorState) -> Vec<PartTraits<InstanceId>> {
    let ids = workspace_parts(state);
    let text = |id: InstanceId, prop: &str| match state.world.get_ref(id, prop) {
        Ok(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    geom::snapshot(&state.world, &ids)
        .into_iter()
        .map(|s| {
            let color = match state.world.get_ref(s.id, "Color") {
                Ok(Value::Color3(c)) => [c.r, c.g, c.b],
                _ => [0.0; 3],
            };
            let style =
                Some(text(s.id, "LookOverride")).filter(|t| !t.is_empty() && t != "Inherit");
            PartTraits {
                id: s.id,
                class: state.world.class_name(s.id).unwrap_or_default().to_string(),
                name: text(s.id, "Name"),
                color,
                material: text(s.id, "Material"),
                style,
                size: s.size,
                // Tags (#78) are not in the engine yet: every part has none.
                tags: Vec::new(),
                parent: state.world.parent(s.id),
            }
        })
        .collect()
}

/// Runs Select similar on `key` and replaces the selection with the result.
pub fn run_similar(state: &mut EditorState, key: SimilarKey) -> usize {
    let traits = part_traits(state);
    let selected = state.selection.parts(&state.world);
    let found = select_similar(&traits, &selected, key, None);
    let n = found.len();
    if n == 0 {
        super::report(state, "Select similar: select a part first");
        return 0;
    }
    state.selection.set(found);
    state.note(format!("{}: {n} selected", key.label()));
    n
}

/// The Select similar popover: one button per key, and the keys themselves while it is open.
pub fn similar_window(ctx: &egui::Context, state: &mut EditorState, popover: &mut Option<Popover>) {
    let mut open = true;
    let mut chosen = None;
    egui::Window::new("Select similar")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .default_pos(Pos2::new(420.0, 90.0))
        .show(ctx, |ui| {
            for key in SimilarKey::ALL {
                let b = egui::Button::new(key.label()).shortcut_text(key.key().to_string());
                if ui.add_sized([200.0, 22.0], b).clicked() {
                    chosen = Some(key);
                }
            }
        });
    if !ctx.egui_wants_keyboard_input() {
        ctx.input_mut(|i| {
            for key in SimilarKey::ALL {
                let Some(k) = egui::Key::from_name(&key.key().to_string()) else {
                    continue;
                };
                if i.consume_key(egui::Modifiers::NONE, k) {
                    chosen = Some(key);
                }
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                *popover = None;
            }
        });
    }
    if let Some(key) = chosen {
        run_similar(state, key);
        *popover = None;
    }
    if !open {
        *popover = None;
    }
}
