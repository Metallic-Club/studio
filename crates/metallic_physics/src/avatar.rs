//! The character's parts: the `HumanoidRootPart` and the 15 R15 body parts. All are invisible:
//! they give scripts the familiar part names (`hit.Parent`, `Head`, `LeftHand`), while the runtime
//! draws the skinned default avatar in their place.
//!
//! - `HumanoidRootPart`: 2 x 2 x 1 at the root, CanCollide. It is the character part loose parts
//!   bump into.
//! - The body parts ([`BodyPart`]): CanCollide off, so the physics keeps them as sensors that
//!   never push or block. They are the boxes around the bone-attached touch volumes (see
//!   [`crate::body`]), moved every step to the pose the touches were found with.
//!
//! All parts are Anchored (the [`crate::CharacterController`] moves them) with Transparency 1.
use crate::body::{BodyPart, BodyPose};
use metallic_core::{Color3, CoreError, InstanceId, Value, Vec3, World};

/// Accent colour, violet `#9b82d4`: reached checkpoints.
pub const VIOLET: Color3 = Color3::new(155.0, 130.0, 212.0);

/// Name of the invisible root part the controller moves.
pub(crate) const ROOT_PART: &str = "HumanoidRootPart";
/// Size of the root part.
const ROOT_SIZE: Vec3 = Vec3::new(2.0, 2.0, 1.0);

/// Creates the root and the 15 body parts under `model`, posed at `root` facing -Z.
pub(crate) fn build(world: &mut World, model: InstanceId, root: Vec3) -> Result<(), CoreError> {
    make_part(world, model, ROOT_PART, ROOT_SIZE, true)?;
    for p in BodyPart::ALL {
        make_part(world, model, p.name(), p.size(), false)?;
    }
    place(world, model, root, 0.0);
    Ok(())
}

fn make_part(
    world: &mut World,
    parent: InstanceId,
    name: &str,
    size: Vec3,
    can_collide: bool,
) -> Result<InstanceId, CoreError> {
    let id = world.create("Part", Some(parent))?;
    world.set(id, "Name", Value::String(name.into()))?;
    world.set(id, "Size", Value::Vector3(size))?;
    world.set(id, "Anchored", Value::Bool(true))?;
    world.set(id, "CanCollide", Value::Bool(can_collide))?;
    world.set(id, "Material", Value::String("SmoothPlastic".into()))?;
    world.set(id, "Transparency", Value::Number(1.0))?;
    Ok(id)
}

/// The instances of a character's parts, looked up once and checked before each use.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PartIds {
    root: Option<InstanceId>,
    body: [Option<InstanceId>; crate::body::BODY_PART_COUNT],
}

impl PartIds {
    /// Finds the parts of `model` again where a cached id is missing or no longer alive.
    pub(crate) fn refresh(&mut self, world: &World, model: InstanceId) {
        let fresh = |id: Option<InstanceId>, name: &str| match id {
            Some(id) if world.is_alive(id) && world.parent(id) == Some(model) => Some(id),
            _ => world.find_first_child(model, name),
        };
        self.root = fresh(self.root, ROOT_PART);
        for p in BodyPart::ALL {
            self.body[p.index()] = fresh(self.body[p.index()], p.name());
        }
    }

    /// The instance of a body part.
    pub(crate) fn body(&self, part: BodyPart) -> Option<InstanceId> {
        self.body[part.index()]
    }

    /// The `HumanoidRootPart`.
    pub(crate) fn root(&self) -> Option<InstanceId> {
        self.root
    }
}

/// Writes Position and Orientation of the root part (at `root`, facing `yaw` degrees; 0 looks
/// along -Z) and of every body part (from `pose`). Missing parts are skipped.
pub(crate) fn place_pose(world: &mut World, ids: &PartIds, root: Vec3, yaw: f64, pose: &BodyPose) {
    // The properties exist on every Part, so these writes cannot fail.
    if let Some(id) = ids.root {
        let _ = world.set(id, "Position", Value::Vector3(root));
        let _ = world.set(id, "Orientation", Value::Vector3(Vec3::new(0.0, yaw, 0.0)));
    }
    for p in BodyPart::ALL {
        let Some(id) = ids.body(p) else {
            continue;
        };
        let frame = pose.parts[p.index()];
        let _ = world.set(id, "Position", Value::Vector3(root + frame.position));
        let _ = world.set(
            id,
            "Orientation",
            Value::Vector3(frame.orientation_degrees()),
        );
    }
}

/// Poses every part of `model` in the rest pose so the root is at `root` and the figure faces
/// `yaw` degrees.
pub(crate) fn place(world: &mut World, model: InstanceId, root: Vec3, yaw: f64) {
    let mut ids = PartIds::default();
    ids.refresh(world, model);
    place_pose(world, &ids, root, yaw, &BodyPose::rest(yaw));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_names_and_sizes_every_part() {
        let mut world = World::new_with_services();
        let ws = world.root("Workspace").unwrap();
        let model = world.create("Model", Some(ws)).unwrap();
        build(&mut world, model, Vec3::new(0.0, 3.0, 0.0)).unwrap();
        assert_eq!(world.children(model).len(), 16);
        let hand = world.find_first_child(model, "LeftHand").unwrap();
        assert_eq!(
            world.get(hand, "Size").unwrap(),
            Value::Vector3(BodyPart::LeftHand.size())
        );
        assert_eq!(world.get(hand, "CanCollide").unwrap(), Value::Bool(false));
        let root = world.find_first_child(model, ROOT_PART).unwrap();
        assert_eq!(world.get(root, "CanCollide").unwrap(), Value::Bool(true));
    }
}
