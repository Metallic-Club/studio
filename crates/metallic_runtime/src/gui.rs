//! The in-game GUI: draws the local player's `PlayerGui` (ScreenGui, Frame, TextLabel) with an
//! egui painter over the 3D view.
//!
//! Layout follows Roblox: each element's `Position` and `Size` are `UDim2`s, a fraction of the
//! parent's rectangle plus an offset in pixels (egui points). The screen is the parent of a
//! ScreenGui's children. A disabled ScreenGui or an invisible Frame hides its whole subtree.
//! A top inset (the Player's touch menu button, [`crate::Game::set_gui_inset`]) moves the
//! ScreenGuis' screen down and makes it shorter by that much.
//! `BackgroundTransparency` 1 draws no background; a TextLabel's `Text` is centred in its
//! rectangle in `TextColor3` at `TextSize` points.
//!
//! [`layout`] does the maths without egui (and is unit tested); [`paint`] draws its result.
use egui::{Align2, Color32, FontId, Painter, Rect};
use metallic_core::{Color3, InstanceId, UDim2, Value, World};
use metallic_script::LOCAL_PLAYER_NAME;

/// One element to draw, in screen points.
#[derive(Debug, Clone, PartialEq)]
pub struct GuiItem {
    /// The Frame or TextLabel.
    pub id: InstanceId,
    /// Left, top, width, height.
    pub rect: [f32; 4],
    /// Background colour (sRGB) and opacity 0..=1; `None` when fully transparent.
    pub background: Option<([u8; 3], f32)>,
    /// Text, colour and size in points; `None` for Frames and empty labels.
    pub text: Option<(String, [u8; 3], f32)>,
}

fn color_bytes(c: Color3) -> [u8; 3] {
    let ch = |v: f64| v.clamp(0.0, 255.0).round() as u8;
    [ch(c.r), ch(c.g), ch(c.b)]
}

fn number(world: &World, id: InstanceId, prop: &str) -> f64 {
    match world.get(id, prop) {
        Ok(Value::Number(n)) if n.is_finite() => n,
        _ => 0.0,
    }
}

fn flag(world: &World, id: InstanceId, prop: &str) -> bool {
    matches!(world.get(id, prop), Ok(Value::Bool(true)))
}

fn udim2(world: &World, id: InstanceId, prop: &str) -> UDim2 {
    match world.get(id, prop) {
        Ok(Value::UDim2(u)) => u,
        _ => UDim2::default(),
    }
}

fn color(world: &World, id: InstanceId, prop: &str) -> Color3 {
    match world.get(id, prop) {
        Ok(Value::Color3(c)) => c,
        _ => Color3::default(),
    }
}

/// The local player's `PlayerGui`, if any.
pub fn local_player_gui(world: &World) -> Option<InstanceId> {
    let players = world.root("Players")?;
    let player = world.find_first_child(players, LOCAL_PLAYER_NAME)?;
    world
        .children(player)
        .iter()
        .copied()
        .find(|&c| world.is_a(c, "PlayerGui"))
}

/// Everything the local player's GUI shows on a `screen` of width x height points, back to front,
/// with the top `inset` points left free.
pub fn layout(world: &World, screen: [f32; 2], inset: f32) -> Vec<GuiItem> {
    let mut out = Vec::new();
    let Some(gui) = local_player_gui(world) else {
        return out;
    };
    let top = if inset.is_finite() {
        inset.clamp(0.0, screen[1].max(0.0))
    } else {
        0.0
    };
    let full = [0.0, top, screen[0], screen[1] - top];
    for &sg in world.children(gui) {
        if world.is_a(sg, "ScreenGui") && flag(world, sg, "Enabled") {
            for &child in world.children(sg) {
                add_element(world, child, full, &mut out);
            }
        }
    }
    out
}

fn add_element(world: &World, id: InstanceId, parent: [f32; 4], out: &mut Vec<GuiItem>) {
    if !world.is_a(id, "Frame") || !flag(world, id, "Visible") {
        return;
    }
    let pos = udim2(world, id, "Position");
    let size = udim2(world, id, "Size");
    let axis = |scale: f64, offset: f64, extent: f32| (scale * f64::from(extent) + offset) as f32;
    let rect = [
        parent[0] + axis(pos.x_scale, pos.x_offset, parent[2]),
        parent[1] + axis(pos.y_scale, pos.y_offset, parent[3]),
        axis(size.x_scale, size.x_offset, parent[2]),
        axis(size.y_scale, size.y_offset, parent[3]),
    ];
    let opacity = (1.0 - number(world, id, "BackgroundTransparency")).clamp(0.0, 1.0) as f32;
    let background =
        (opacity > 0.0).then(|| (color_bytes(color(world, id, "BackgroundColor3")), opacity));
    let text = if world.is_a(id, "TextLabel") {
        match world.get(id, "Text") {
            Ok(Value::String(s)) if !s.is_empty() => Some((
                s,
                color_bytes(color(world, id, "TextColor3")),
                number(world, id, "TextSize").clamp(1.0, 200.0) as f32,
            )),
            _ => None,
        }
    } else {
        None
    };
    out.push(GuiItem {
        id,
        rect,
        background,
        text,
    });
    for &child in world.children(id) {
        add_element(world, child, rect, out);
    }
}

/// Draws the local player's GUI over the painter's clip rectangle (the whole 3D view), below a
/// top `inset` in points.
pub fn paint(world: &World, painter: &Painter, inset: f32) {
    let area = painter.clip_rect();
    for item in layout(world, [area.width(), area.height()], inset) {
        let rect = Rect::from_min_size(
            area.min + egui::vec2(item.rect[0], item.rect[1]),
            egui::vec2(item.rect[2], item.rect[3]),
        );
        if let Some(([r, g, b], opacity)) = item.background {
            let alpha = (opacity * 255.0).round() as u8;
            painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(r, g, b, alpha));
        }
        if let Some((text, [r, g, b], size)) = item.text {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(size),
                Color32::from_rgb(r, g, b),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_label() -> (World, InstanceId, InstanceId) {
        let mut w = World::new_with_services();
        let players = w.root("Players").unwrap();
        let p = w.create("Player", Some(players)).unwrap();
        w.set(p, "Name", Value::String(LOCAL_PLAYER_NAME.into()))
            .unwrap();
        let gui = w.create("PlayerGui", Some(p)).unwrap();
        let sg = w.create("ScreenGui", Some(gui)).unwrap();
        let frame = w.create("Frame", Some(sg)).unwrap();
        w.set(
            frame,
            "Position",
            Value::UDim2(UDim2::new(0.5, -100.0, 0.0, 20.0)),
        )
        .unwrap();
        w.set(
            frame,
            "Size",
            Value::UDim2(UDim2::new(0.0, 200.0, 0.1, 0.0)),
        )
        .unwrap();
        let label = w.create("TextLabel", Some(frame)).unwrap();
        w.set(label, "Size", Value::UDim2(UDim2::new(1.0, 0.0, 1.0, 0.0)))
            .unwrap();
        w.set(label, "Text", Value::String("Checkpoint 2/3".into()))
            .unwrap();
        w.set(label, "BackgroundTransparency", Value::Number(1.0))
            .unwrap();
        (w, sg, frame)
    }

    #[test]
    fn udim2_is_scale_of_parent_plus_offset() {
        let (w, _, _) = world_with_label();
        let items = layout(&w, [1000.0, 500.0], 0.0);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].rect, [400.0, 20.0, 200.0, 50.0]);
        assert_eq!(items[1].rect, [400.0, 20.0, 200.0, 50.0]);
        assert!(items[0].background.is_some());
        assert_eq!(items[1].background, None);
        assert_eq!(
            items[1].text.as_ref().map(|t| t.0.as_str()),
            Some("Checkpoint 2/3")
        );
    }

    /// #333: with the touch menu button showing, the GUI's screen starts below it.
    #[test]
    fn a_top_inset_moves_the_screen_down() {
        let (w, _, _) = world_with_label();
        let items = layout(&w, [1000.0, 500.0], 100.0);
        // 20 points below the 100-point inset, 10% of the 400 points left.
        assert_eq!(items[0].rect, [400.0, 120.0, 200.0, 40.0]);
        let none = layout(&w, [1000.0, 500.0], f32::NAN);
        assert_eq!(
            none[0].rect,
            [400.0, 20.0, 200.0, 50.0],
            "a bad inset is none"
        );
    }

    #[test]
    fn hidden_frames_and_disabled_screen_guis_draw_nothing() {
        let (mut w, sg, frame) = world_with_label();
        w.set(frame, "Visible", Value::Bool(false)).unwrap();
        assert!(layout(&w, [800.0, 600.0], 0.0).is_empty());
        w.set(frame, "Visible", Value::Bool(true)).unwrap();
        w.set(sg, "Enabled", Value::Bool(false)).unwrap();
        assert!(layout(&w, [800.0, 600.0], 0.0).is_empty());
    }
}
