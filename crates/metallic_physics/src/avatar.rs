//! The character's body parts: R6-style blocks posed around the root. They are invisible: they
//! give scripts the familiar part names (`hit.Parent`, `Head`, `Torso`) and the character its
//! collision and touch volumes, while the runtime draws the skinned default avatar in their place.
//!
//! Layout relative to the `HumanoidRootPart` centre (which sits [`crate::ROOT_HEIGHT`] = 3 studs
//! above the feet), before the character's yaw is applied:
//!
//! | Part             | Size      | Offset         | CanCollide |
//! |------------------|-----------|----------------|------------|
//! | HumanoidRootPart | 2 x 2 x 1 | (0, 0, 0)      | yes        |
//! | Torso            | 2 x 2 x 1 | (0, 0, 0)      | yes        |
//! | Head             | 2 x 1 x 1 | (0, 1.5, 0)    | yes        |
//! | Left Arm         | 1 x 2 x 1 | (-1.5, 0, 0)   | no         |
//! | Right Arm        | 1 x 2 x 1 | (1.5, 0, 0)    | no         |
//! | Left Leg         | 1 x 2 x 1 | (-0.5, -2, 0)  | no         |
//! | Right Leg        | 1 x 2 x 1 | (0.5, -2, 0)   | no         |
//!
//! All parts are Anchored (the [`crate::CharacterController`] moves them) with Transparency 1.
use metallic_core::{Color3, CoreError, InstanceId, Value, Vec3, World};

/// Accent colour, violet `#9b82d4`: reached checkpoints.
pub const VIOLET: Color3 = Color3::new(155.0, 130.0, 212.0);

/// Name of the invisible root part the controller moves.
pub(crate) const ROOT_PART: &str = "HumanoidRootPart";

/// One body part: name, size, offset from the root, CanCollide.
struct Limb {
    name: &'static str,
    size: Vec3,
    offset: Vec3,
    can_collide: bool,
}

const fn limb(
    name: &'static str,
    size: (f64, f64, f64),
    offset: (f64, f64, f64),
    can_collide: bool,
) -> Limb {
    Limb {
        name,
        size: Vec3::new(size.0, size.1, size.2),
        offset: Vec3::new(offset.0, offset.1, offset.2),
        can_collide,
    }
}

const LIMBS: [Limb; 7] = [
    limb(ROOT_PART, (2.0, 2.0, 1.0), (0.0, 0.0, 0.0), true),
    limb("Head", (2.0, 1.0, 1.0), (0.0, 1.5, 0.0), true),
    limb("Torso", (2.0, 2.0, 1.0), (0.0, 0.0, 0.0), true),
    limb("Left Arm", (1.0, 2.0, 1.0), (-1.5, 0.0, 0.0), false),
    limb("Right Arm", (1.0, 2.0, 1.0), (1.5, 0.0, 0.0), false),
    limb("Left Leg", (1.0, 2.0, 1.0), (-0.5, -2.0, 0.0), false),
    limb("Right Leg", (1.0, 2.0, 1.0), (0.5, -2.0, 0.0), false),
];

/// Creates the root and the six body parts under `model`, posed at `root` facing -Z.
pub(crate) fn build(world: &mut World, model: InstanceId, root: Vec3) -> Result<(), CoreError> {
    for l in &LIMBS {
        make_part(world, model, l)?;
    }
    place(world, model, root, 0.0);
    Ok(())
}

fn make_part(world: &mut World, parent: InstanceId, l: &Limb) -> Result<InstanceId, CoreError> {
    let id = world.create("Part", Some(parent))?;
    world.set(id, "Name", Value::String(l.name.into()))?;
    world.set(id, "Size", Value::Vector3(l.size))?;
    world.set(id, "Anchored", Value::Bool(true))?;
    world.set(id, "CanCollide", Value::Bool(l.can_collide))?;
    world.set(id, "Material", Value::String("SmoothPlastic".into()))?;
    world.set(id, "Transparency", Value::Number(1.0))?;
    Ok(id)
}

/// Rotates `v` by `yaw` degrees around +Y (Roblox orientation convention).
fn rotate_y(v: Vec3, yaw: f64) -> Vec3 {
    let (s, c) = yaw.to_radians().sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}

/// Writes Position and Orientation of every avatar part found in `model` so the root is at
/// `root` and the figure faces yaw `yaw` (degrees; 0 looks along -Z). Missing parts are skipped.
pub(crate) fn place(world: &mut World, model: InstanceId, root: Vec3, yaw: f64) {
    let orientation = Value::Vector3(Vec3::new(0.0, yaw, 0.0));
    for l in &LIMBS {
        let Some(id) = world.find_first_child(model, l.name) else {
            continue;
        };
        let pos = root + rotate_y(l.offset, yaw);
        // The properties exist on every Part, so these writes cannot fail.
        let _ = world.set(id, "Position", Value::Vector3(pos));
        let _ = world.set(id, "Orientation", orientation.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_y_matches_roblox_convention() {
        // yaw 90 turns the look vector -Z into -X
        let v = rotate_y(Vec3::new(0.0, 0.0, -1.0), 90.0);
        assert!((v - Vec3::new(-1.0, 0.0, 0.0)).magnitude() < 1e-12);
    }
}
