//! `Camera`: a point of view on the world. `Workspace.CurrentCamera` is the one the screen shows.
//!
//! `Focus` (the point the camera orbits, used by camera scripts) is left out: nothing reads it until scripts and
//! input exist, and ADR 0007 lists only `CFrame` and `FieldOfView`.

use metallic_core::{ClassDesc, PropMeta, Widget};

static PROPS: &[PropMeta] = &[
    PropMeta::cframe("CFrame")
        .transform()
        .group("Transform")
        .order(0)
        .desc("Where the camera is and which way it looks, in world space. It looks along its forward direction (-Z)."),
    PropMeta::number("FieldOfView", 70.0)
        .range(1.0, 120.0)
        .widget(Widget::Slider)
        .visual()
        .group("Camera")
        .order(0)
        .desc("How much the camera sees from top to bottom, in degrees. Smaller values zoom in."),
];

/// A point of view on the world.
pub static CLASS: ClassDesc = ClassDesc::new("Camera", Some("WorldObject"))
    .describe("A point of view on the world. The screen shows what Workspace.CurrentCamera sees.")
    .category("World")
    .creatable()
    .components(&["Transform"])
    .props(PROPS);

#[cfg(test)]
mod tests {
    use metallic_core::{CFrame, Value};

    use crate::testing::{game, new_in, workspace};

    #[test]
    fn field_of_view_defaults_and_clamps() {
        let mut dm = game();
        let ws = workspace(&dm);
        let camera = new_in(&mut dm, "Camera", ws, "Camera");
        assert_eq!(dm.get_by_name(camera, "FieldOfView"), Ok(Value::Number(70.0)));
        assert_eq!(dm.get_by_name(camera, "CFrame"), Ok(Value::CFrame(CFrame::IDENTITY)));
        dm.set_by_name(camera, "FieldOfView", Value::Number(500.0)).unwrap();
        assert_eq!(dm.get_by_name(camera, "FieldOfView"), Ok(Value::Number(120.0)));
        dm.set_by_name(camera, "FieldOfView", Value::Number(0.0)).unwrap();
        assert_eq!(dm.get_by_name(camera, "FieldOfView"), Ok(Value::Number(1.0)));
    }
}
