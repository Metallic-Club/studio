//! The view the parts are drawn from: `Workspace.CurrentCamera`'s `CFrame` and `FieldOfView`, as a camera-relative
//! reverse-Z projection.

use metallic_core::glam::{Mat4, Quat, Vec3};
use metallic_core::{CFrame, DataModel, Id, PropId};

use crate::depth::reverse_z_perspective_infinite;

/// A point of view: where the camera is, where it looks (−Z of `rotation`, as a `CFrame`), and how wide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World position.
    pub position: Vec3,
    /// World rotation; the camera looks along its −Z and +Y is up.
    pub rotation: Quat,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    /// Distance to the near plane in metres. There is no far plane (reverse-Z, see [`crate::depth`]).
    pub near: f32,
}

impl Camera {
    /// The near plane distance [`Camera::from_cframe`] uses.
    pub const DEFAULT_NEAR: f32 = 0.1;

    /// A camera at `cframe` with a vertical field of view of `field_of_view` degrees (clamped to 1 to 120, as
    /// `Camera.FieldOfView` is).
    pub fn from_cframe(cframe: CFrame, field_of_view: f64) -> Self {
        let degrees = if field_of_view.is_finite() {
            field_of_view.clamp(1.0, 120.0)
        } else {
            70.0
        };
        Self {
            position: cframe.position(),
            rotation: cframe.rotation(),
            fov_y: (degrees as f32).to_radians(),
            near: Self::DEFAULT_NEAR,
        }
    }

    /// The camera-relative view-projection: positions relative to the camera (world minus [`Camera::position`])
    /// to clip space, for a target `aspect` (width / height) wide. Leaving the translation out keeps f32 precise
    /// far from the world origin (ADR 0008 §11).
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        let projection = reverse_z_perspective_infinite(self.fov_y, aspect, self.near.max(1e-4));
        projection * Mat4::from_quat(self.rotation.inverse())
    }
}

/// Reads `Workspace.CurrentCamera` (or the `CurrentCamera` of whatever world root it is given) from a data model,
/// looking property ids up by name only when the camera or the data model changes.
#[derive(Clone, Copy, Debug, Default)]
pub struct CameraReader {
    /// The data model ([`DataModel::uid`]) and world root the ids below belong to, and `CurrentCamera`'s id.
    world: Option<(u64, Id, Option<PropId>)>,
    /// The camera last read and its `CFrame` and `FieldOfView` property ids.
    camera: Option<(Id, PropId, Option<PropId>)>,
}

impl CameraReader {
    /// A reader that has resolved nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current camera of `world` in `dm`; `None` when `CurrentCamera` is unset, names a destroyed instance, or
    /// the camera has no `CFrame`. A missing `FieldOfView` reads as 70°.
    pub fn read(&mut self, dm: &DataModel, world: Id) -> Option<Camera> {
        let current_camera = match self.world {
            Some((uid, id, prop)) if uid == dm.uid() && id == world => prop,
            _ => {
                let prop = dm.prop_id(world, "CurrentCamera").ok();
                self.world = Some((dm.uid(), world, prop));
                self.camera = None;
                prop
            }
        };
        let camera = dm.reference(world, current_camera?).ok()??;
        let (cframe, fov) = match self.camera {
            Some((id, cframe, fov)) if id == camera => (cframe, fov),
            _ => {
                let cframe = dm.prop_id(camera, "CFrame").ok()?;
                let fov = dm.prop_id(camera, "FieldOfView").ok();
                self.camera = Some((camera, cframe, fov));
                (cframe, fov)
            }
        };
        let placement = dm.cframe(camera, cframe).ok()?;
        let field_of_view = fov.and_then(|prop| dm.number(camera, prop).ok()).unwrap_or(70.0);
        Some(Camera::from_cframe(placement, field_of_view))
    }
}

#[cfg(test)]
mod tests {
    use metallic_core::Value;
    use metallic_core::glam::Vec4;

    use super::*;

    fn workspace(dm: &DataModel) -> Id {
        let class = dm.registry().class_id("Workspace").unwrap();
        dm.service(class).unwrap()
    }

    #[test]
    fn reads_the_current_camera() {
        let mut dm = metallic_classes::new_game().unwrap();
        let ws = workspace(&dm);
        let mut reader = CameraReader::new();
        let camera = reader.read(&dm, ws).expect("a new game has a camera");
        let Ok(Value::Ref(Some(id))) = dm.get_by_name(ws, "CurrentCamera") else {
            panic!("no CurrentCamera");
        };
        let Ok(Value::CFrame(start)) = dm.get_by_name(id, "CFrame") else {
            panic!("no CFrame");
        };
        assert_eq!(camera.position, start.position());
        assert!((camera.fov_y - 70f32.to_radians()).abs() < 1e-6);

        let place = CFrame::look_at(Vec3::new(1.0, 2.0, 3.0), Vec3::ZERO, Vec3::Y);
        dm.set_by_name(id, "CFrame", Value::CFrame(place)).unwrap();
        dm.set_by_name(id, "FieldOfView", Value::Number(40.0)).unwrap();
        let camera = reader.read(&dm, ws).unwrap();
        assert_eq!(camera.position, Vec3::new(1.0, 2.0, 3.0));
        assert!((camera.fov_y - 40f32.to_radians()).abs() < 1e-6);
    }

    #[test]
    fn a_missing_camera_reads_as_none() {
        let mut dm = metallic_classes::new_game().unwrap();
        let ws = workspace(&dm);
        let mut reader = CameraReader::new();
        assert!(reader.read(&dm, ws).is_some());
        dm.set_by_name(ws, "CurrentCamera", Value::Ref(None)).unwrap();
        assert_eq!(reader.read(&dm, ws), None);
        // A world root without the property at all.
        let folder = dm.create(dm.registry().class_id("Folder").unwrap()).unwrap();
        assert_eq!(reader.read(&dm, folder), None);
    }

    #[test]
    fn the_view_looks_down_minus_z_with_reverse_z() {
        let camera = Camera::from_cframe(CFrame::new(Vec3::new(100.0, 0.0, 0.0)), 90.0);
        let m = camera.view_projection(1.0);
        // A point 10 m straight ahead, given relative to the camera, lands in the centre of the screen.
        let clip = m * Vec4::new(0.0, 0.0, -10.0, 1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() < 1e-6 && ndc.y.abs() < 1e-6);
        assert!((0.0..1.0).contains(&ndc.z));
        // Nearer is larger depth.
        let near = m * Vec4::new(0.0, 0.0, -1.0, 1.0);
        assert!(near.z / near.w > ndc.z);
        // At 90° the top edge of the view is at 45°.
        let top = m * Vec4::new(0.0, 10.0, -10.0, 1.0);
        assert!((top.y / top.w - 1.0).abs() < 1e-5);
    }
}
