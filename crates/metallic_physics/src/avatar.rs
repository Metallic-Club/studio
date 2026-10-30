//! The Metallic avatar: an R6-style block figure built from Parts and posed around its root.
//!
//! Layout relative to the `HumanoidRootPart` centre (which sits [`crate::ROOT_HEIGHT`] = 3 studs
//! above the feet), before the character's yaw is applied:
//!
//! | Part             | Size      | Offset         | Visible | CanCollide |
//! |------------------|-----------|----------------|---------|------------|
//! | HumanoidRootPart | 2 x 2 x 1 | (0, 0, 0)      | no      | yes        |
//! | Torso            | 2 x 2 x 1 | (0, 0, 0)      | yes     | yes        |
//! | Head             | 2 x 1 x 1 | (0, 1.5, 0)    | yes     | yes        |
//! | Left Arm         | 1 x 2 x 1 | (-1.5, 0, 0)   | yes     | no         |
//! | Right Arm        | 1 x 2 x 1 | (1.5, 0, 0)    | yes     | no         |
//! | Left Leg         | 1 x 2 x 1 | (-0.5, -2, 0)  | yes     | no         |
//! | Right Leg        | 1 x 2 x 1 | (0.5, -2, 0)   | yes     | no         |
//!
//! Default colours follow the classic Roblox look: [`SKIN`] head and arms, a [`SHIRT`] torso and
//! [`PANTS`] legs; the Torso carries a thin violet `Stripe` part as its child.
//! All parts are Anchored: the [`crate::CharacterController`] moves them.
use metallic_core::{Color3, CoreError, InstanceId, Value, Vec3, World};

/// Head and arm colour, a warm skin tone `#eab892`.
pub const SKIN: Color3 = Color3::new(234.0, 184.0, 146.0);
/// Torso colour, slate blue `#4a68aa`.
pub const SHIRT: Color3 = Color3::new(74.0, 104.0, 170.0);
/// Leg colour, dark graphite `#2e3240`.
pub const PANTS: Color3 = Color3::new(46.0, 50.0, 64.0);
/// Accent colour, violet `#9b82d4`: the torso stripe and reached checkpoints.
pub const VIOLET: Color3 = Color3::new(155.0, 130.0, 212.0);

/// Name of the invisible root part the controller moves.
pub(crate) const ROOT_PART: &str = "HumanoidRootPart";

/// One body part: name, size, offset from the root, visible, CanCollide.
struct Limb {
    name: &'static str,
    size: Vec3,
    offset: Vec3,
    visible: bool,
    can_collide: bool,
}

const fn limb(
    name: &'static str,
    size: (f64, f64, f64),
    offset: (f64, f64, f64),
    visible: bool,
    can_collide: bool,
) -> Limb {
    Limb {
        name,
        size: Vec3::new(size.0, size.1, size.2),
        offset: Vec3::new(offset.0, offset.1, offset.2),
        visible,
        can_collide,
    }
}

const LIMBS: [Limb; 7] = [
    limb(ROOT_PART, (2.0, 2.0, 1.0), (0.0, 0.0, 0.0), false, true),
    limb("Head", (2.0, 1.0, 1.0), (0.0, 1.5, 0.0), true, true),
    limb("Torso", (2.0, 2.0, 1.0), (0.0, 0.0, 0.0), true, true),
    limb("Left Arm", (1.0, 2.0, 1.0), (-1.5, 0.0, 0.0), true, false),
    limb("Right Arm", (1.0, 2.0, 1.0), (1.5, 0.0, 0.0), true, false),
    limb("Left Leg", (1.0, 2.0, 1.0), (-0.5, -2.0, 0.0), true, false),
    limb("Right Leg", (1.0, 2.0, 1.0), (0.5, -2.0, 0.0), true, false),
];

/// The violet band around the chest, a child of the Torso.
const STRIPE: Limb = limb("Stripe", (2.04, 0.4, 1.04), (0.0, 0.3, 0.0), true, false);

/// Creates the root, the six body parts and the stripe under `model`, posed at `root` facing -Z.
pub(crate) fn build(world: &mut World, model: InstanceId, root: Vec3) -> Result<(), CoreError> {
    for l in &LIMBS {
        let id = make_part(world, model, l)?;
        if l.name == "Torso" {
            make_part(world, id, &STRIPE)?;
        }
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
    let (color, transparency) = match (l.visible, l.name) {
        (false, _) => (SHIRT, 1.0),
        (true, "Stripe") => (VIOLET, 0.0),
        (true, "Head" | "Left Arm" | "Right Arm") => (SKIN, 0.0),
        (true, "Left Leg" | "Right Leg") => (PANTS, 0.0),
        (true, _) => (SHIRT, 0.0),
    };
    world.set(id, "Color", Value::Color3(color))?;
    world.set(id, "Transparency", Value::Number(transparency))?;
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
        if l.name == "Torso"
            && let Some(stripe) = world.find_first_child(id, STRIPE.name)
        {
            let pos = root + rotate_y(STRIPE.offset, yaw);
            let _ = world.set(stripe, "Position", Value::Vector3(pos));
            let _ = world.set(stripe, "Orientation", orientation.clone());
        }
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
