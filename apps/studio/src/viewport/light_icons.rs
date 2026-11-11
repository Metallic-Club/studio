//! Lights in the viewport (#506): each light in Workspace draws its Explorer icon where it is, a fixed size on screen,
//! so a light (which draws nothing of its own) can be seen and clicked. A click on an icon selects its light, as a
//! click on a part does; the nearest icon wins where they overlap. Viewport options → View shows or hides them.
//!
//! A selected light also shows how far it reaches, as a wireframe (#523): a PointLight's Range as a ball of three
//! circles, a SpotLight's Range and Angle as a cone.

use egui::{Color32, Painter, Pos2, Rect, vec2};
use metallic_engine::metallic_classes::world::lights::light::world_cframe;
use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{DataModel, Id, Value};

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

/// How many lines a range's circle is drawn with.
const CIRCLE_STEPS: usize = 48;

/// The wireframe of how far the light `id` reaches (#523), as lines in the world: a PointLight's Range as three
/// circles, one in each of its axis planes; a SpotLight's as a cone along its forward direction (−Z), opening Angle
/// degrees, its sides Range long: the circle where it ends and four lines from the light to it. Empty for anything
/// else, and for a light with no reach.
pub fn range_lines(dm: &DataModel, id: Id) -> Vec<[Vec3; 2]> {
    let number = |name: &str| match dm.get_by_name(id, name) {
        Ok(Value::Number(n)) => Some(n as f32),
        _ => None,
    };
    let Some(range) = number("Range").filter(|range| *range > 0.0) else {
        return Vec::new();
    };
    let Ok(frame) = world_cframe(dm, id) else {
        return Vec::new();
    };
    let (at, right, up, back) = (
        frame.position(),
        frame.right_vector(),
        frame.up_vector(),
        -frame.look_vector(),
    );
    let mut lines = Vec::new();
    if is_a(dm, id, "SpotLight") {
        let half = (number("Angle").unwrap_or(90.0) * 0.5).to_radians();
        let (centre, radius) = (at - back * range * half.cos(), range * half.sin());
        circle(&mut lines, centre, right * radius, up * radius);
        for side in [right, up, -right, -up] {
            lines.push([at, centre + side * radius]);
        }
    } else if is_a(dm, id, "PointLight") {
        for (a, b) in [(right, up), (right, back), (up, back)] {
            circle(&mut lines, at, a * range, b * range);
        }
    }
    lines
}

/// Adds the circle around `centre` through `centre + a` and `centre + b` (at right angles, as long as each other).
fn circle(lines: &mut Vec<[Vec3; 2]>, centre: Vec3, a: Vec3, b: Vec3) {
    let point = |step: usize| {
        let turn = std::f32::consts::TAU * step as f32 / CIRCLE_STEPS as f32;
        centre + a * turn.cos() + b * turn.sin()
    };
    lines.extend((0..CIRCLE_STEPS).map(|step| [point(step), point(step + 1)]));
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

    /// #523: a PointLight's range is three circles of its Range; a SpotLight's a cone of its Range and Angle.
    #[test]
    fn a_light_s_range_is_a_ball_or_a_cone() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let world = workspace(&dm).unwrap();
        let at = Vec3::new(1.0, 5.0, -2.0);
        let light = |dm: &mut DataModel, class: &str| {
            let id = dm.create(dm.registry().class_id(class).unwrap()).unwrap();
            dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(at))).unwrap();
            dm.set_by_name(id, "Range", Value::Number(10.0)).unwrap();
            dm.set_parent(id, Some(world)).unwrap();
            id
        };
        let point = light(&mut dm, "PointLight");
        let ball = range_lines(&dm, point);
        assert_eq!(ball.len(), 3 * CIRCLE_STEPS);
        for [a, b] in &ball {
            assert!(((*a - at).length() - 10.0).abs() < 1e-3 && ((*b - at).length() - 10.0).abs() < 1e-3);
        }
        // Each axis plane holds one circle: every point of the first is level with the light.
        assert!(ball[..CIRCLE_STEPS].iter().all(|[a, _]| (a.z - at.z).abs() < 1e-3));

        let spot = light(&mut dm, "SpotLight");
        dm.set_by_name(spot, "Angle", Value::Number(60.0)).unwrap();
        let cone = range_lines(&dm, spot);
        assert_eq!(cone.len(), CIRCLE_STEPS + 4);
        // Its sides run from the light, Range long, 30 degrees off its forward direction (−Z).
        for [from, to] in &cone[CIRCLE_STEPS..] {
            assert_eq!(*from, at);
            let side = *to - at;
            assert!((side.length() - 10.0).abs() < 1e-3);
            assert!((side.normalize().dot(Vec3::NEG_Z) - 30f32.to_radians().cos()).abs() < 1e-3);
        }
        assert!(range_lines(&dm, world).is_empty(), "not a light");
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
