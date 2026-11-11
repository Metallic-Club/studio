//! Lights in the viewport (#506): each light in Workspace draws its Explorer icon where it is, a fixed size on screen,
//! so a light (which draws nothing of its own) can be seen and clicked. A click on an icon selects its light, as a
//! click on a part does; the nearest icon wins where they overlap. Viewport options → View shows or hides them.

use egui::{Color32, Painter, Pos2, Rect, vec2};
use metallic_engine::metallic_classes::world::lights::light::world_cframe;
use metallic_engine::metallic_core::{DataModel, Id};

use super::View;
use super::pick::{is_a, workspace};
use crate::panels::class_icon;
use crate::theme::icons::{self, Icon};

/// An icon's size on screen, in points: large enough to see and click at once (#522).
pub const SIZE: f32 = 32.0;

/// One light's icon on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightIcon {
    /// The light.
    pub id: Id,
    /// Where its icon is.
    pub rect: Rect,
    /// Its icon: its class's, as the Explorer shows it.
    pub icon: Icon,
    /// How far it is from the camera.
    pub depth: f32,
}

/// The icons of the lights in `dm`'s Workspace that `view` sees, the farthest first (the order they are drawn in).
pub fn light_icons(dm: &DataModel, view: &View) -> Vec<LightIcon> {
    let Some(world) = workspace(dm) else {
        return Vec::new();
    };
    let Ok(descendants) = dm.descendants(world) else {
        return Vec::new();
    };
    let mut out: Vec<LightIcon> = descendants
        .filter(|&id| is_a(dm, id, "Light"))
        .filter_map(|id| {
            let at = world_cframe(dm, id).ok()?.position();
            let centre = view.project(at)?;
            let icon = dm
                .class_of(id)
                .map_or(Icon::ExplorerInstance, |class| class_icon(dm.registry(), class));
            Some(LightIcon {
                id,
                rect: Rect::from_center_size(centre, vec2(SIZE, SIZE)),
                icon,
                depth: (at - view.pos).length(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

/// The light whose icon is under `at`, the nearest where icons overlap.
pub fn icon_at(icons: &[LightIcon], at: Pos2) -> Option<Id> {
    icons
        .iter()
        .rev()
        .find(|icon| icon.rect.contains(at))
        .map(|icon| icon.id)
}

/// Draws `icons`, the farthest first: the icon alone, with nothing behind it (#522), in its class's colour, lighter
/// when `hovered` and in the accent when its light is selected.
pub fn paint(
    painter: &Painter,
    icons: &[LightIcon],
    selected: &[Id],
    hovered: Option<Id>,
    colors: &crate::theme::Colors,
) {
    for icon in icons {
        icons::paint(painter, icon.icon, icon.rect, ink(icon, selected, hovered, colors));
    }
}

/// An icon's colour: the accent when its light is selected, its class's colour lightened under the pointer, and its
/// class's colour otherwise.
fn ink(icon: &LightIcon, selected: &[Id], hovered: Option<Id>, colors: &crate::theme::Colors) -> Color32 {
    if selected.contains(&icon.id) {
        colors.accent
    } else if hovered == Some(icon.id) {
        icon.icon.color(colors).lerp_to_gamma(Color32::WHITE, 0.4)
    } else {
        icon.icon.color(colors)
    }
}

#[cfg(test)]
mod tests {
    use metallic_engine::metallic_core::glam::Vec3;
    use metallic_engine::metallic_core::{CFrame, Value};

    use super::*;
    use crate::viewport::camera::EditorCamera;

    #[test]
    fn a_light_s_icon_is_where_it_is_and_the_nearest_is_clicked() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let world = workspace(&dm).unwrap();
        let light = |dm: &mut DataModel, class: &str, at: Vec3| {
            let id = dm.create(dm.registry().class_id(class).unwrap()).unwrap();
            dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(at))).unwrap();
            dm.set_parent(id, Some(world)).unwrap();
            id
        };
        let near = light(&mut dm, "PointLight", Vec3::new(0.0, 5.0, 0.0));
        let far = light(&mut dm, "SpotLight", Vec3::new(0.0, 5.0, -20.0));
        let camera = EditorCamera::looking_at(Vec3::new(0.0, 5.0, 20.0), Vec3::new(0.0, 5.0, 0.0));
        let view = View::new(&camera, Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let icons = light_icons(&dm, &view);
        assert_eq!(
            icons.iter().map(|icon| icon.id).collect::<Vec<_>>(),
            [far, near],
            "the farthest first"
        );
        assert_ne!(icons[0].icon, icons[1].icon, "each its class's icon");
        // Both straight ahead, one behind the other: a click there picks the nearer.
        let centre = icons[1].rect.center();
        assert_eq!(icon_at(&icons, centre), Some(near));
        assert_eq!(icon_at(&icons, centre + vec2(SIZE, SIZE)), None);
        // Behind the camera: no icon.
        let camera = EditorCamera::looking_at(Vec3::new(0.0, 5.0, -40.0), Vec3::new(0.0, 5.0, -80.0));
        let behind = View::new(&camera, Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        assert!(light_icons(&dm, &behind).is_empty());
    }

    /// #522: the icon alone, its colour telling selected and hovered apart.
    #[test]
    fn an_icon_s_colour_tells_selected_and_hovered() {
        let colors = crate::theme::Colors::default();
        let icon = LightIcon {
            id: Id::new(7, 0),
            rect: Rect::from_center_size(Pos2::ZERO, vec2(SIZE, SIZE)),
            icon: Icon::ExplorerInstance,
            depth: 1.0,
        };
        let plain = ink(&icon, &[], None, &colors);
        assert_eq!(plain, icon.icon.color(&colors));
        assert_eq!(ink(&icon, &[icon.id], Some(icon.id), &colors), colors.accent);
        assert_ne!(ink(&icon, &[], Some(icon.id), &colors), plain);
        assert_eq!(ink(&icon, &[Id::new(8, 0)], Some(Id::new(8, 0)), &colors), plain);
    }
}
