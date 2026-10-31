//! Pivots: the point (and axes) an object moves, rotates and scales about (spec 4.4).
//!
//! Pivots are core properties (#313, Roblox names): a BasePart's `PivotOffset` (a CFrame relative
//! to the part) and a Model's `WorldPivot` (world space; it follows the bounding box until it is
//! first set, and a Model with a `PrimaryPart` uses that part's pivot). Studio reads them through
//! `metallic_core::transform::get_pivot` and changes them as ordinary property writes
//! ([`pivot_writes`], [`reset_writes`]) through the preview overlay, so pivot edits are undoable,
//! saved with the scene and replicated like any other property.
use metallic_core::transform::{get_pivot, primary_part};
use metallic_core::{CFrame, InstanceId, Value, World};
use metallic_render::glam::{DQuat, DVec3};

use crate::selection::Selection;
use crate::transform::{Aabb, PartTransform, from_d, to_d};

/// A rigid frame: position and rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pivot {
    /// Position (relative to the part for a part's offset; world space otherwise).
    pub position: DVec3,
    /// Rotation.
    pub rotation: DQuat,
}

impl Pivot {
    /// The identity frame (a part's default offset).
    pub const IDENTITY: Pivot = Pivot {
        position: DVec3::ZERO,
        rotation: DQuat::IDENTITY,
    };

    /// A frame at `position` with world axes.
    pub fn at(position: DVec3) -> Pivot {
        Pivot {
            position,
            rotation: DQuat::IDENTITY,
        }
    }

    /// `self` followed by `local` (local expressed in `self`'s frame).
    pub fn then(&self, local: &Pivot) -> Pivot {
        Pivot {
            position: self.position + self.rotation * local.position,
            rotation: (self.rotation * local.rotation).normalize(),
        }
    }

    /// The frame `x` such that `self.then(x) == world`.
    pub fn to_local(&self, world: &Pivot) -> Pivot {
        let inv = self.rotation.inverse();
        Pivot {
            position: inv * (world.position - self.position),
            rotation: (inv * world.rotation).normalize(),
        }
    }
}

/// A part's frame as a [`Pivot`].
pub fn part_frame(t: &PartTransform) -> Pivot {
    Pivot {
        position: t.position,
        rotation: t.rotation,
    }
}

/// A core CFrame as a [`Pivot`].
pub fn from_cframe(cf: &CFrame) -> Pivot {
    let [x, y, z, w] = cf.quaternion();
    Pivot {
        position: to_d(cf.position),
        rotation: DQuat::from_xyzw(x, y, z, w).normalize(),
    }
}

/// A [`Pivot`] as a core CFrame.
pub fn to_cframe(p: &Pivot) -> CFrame {
    let q = p.rotation.normalize();
    CFrame::from_quaternion(from_d(p.position), [q.x, q.y, q.z, q.w])
}

/// The world-space pivot of `id`: core's pivot for a part or Model, the bounding-box centre for
/// other containers. `None` when `id` has no parts.
pub fn world_pivot(world: &World, id: InstanceId) -> Option<Pivot> {
    if (world.is_a(id, "BasePart") || world.is_a(id, "Model"))
        && let Ok(cf) = get_pivot(world, id)
    {
        return Some(from_cframe(&cf));
    }
    let parts = Selection::parts_of(world, id);
    Aabb::of_parts(world, &parts).map(|b| Pivot::at(b.centre()))
}

/// One property write that a pivot change needs.
pub type PivotWrite = (InstanceId, &'static str, Value);

/// The property writes that put the pivot of `id` at the world frame `pivot`: the part's
/// `PivotOffset`; for a Model, its `PrimaryPart`'s `PivotOffset` when it has one, else its
/// `WorldPivot` (and the hidden `WorldPivotSet`). Empty for other classes.
pub fn pivot_writes(world: &World, id: InstanceId, pivot: &Pivot) -> Vec<PivotWrite> {
    let offset_of = |part: InstanceId| -> Option<PivotWrite> {
        let t = PartTransform::read(world, part)?;
        let local = part_frame(&t).to_local(pivot);
        Some((
            part,
            "PivotOffset",
            Value::CFrame(Box::new(to_cframe(&local))),
        ))
    };
    if world.is_a(id, "BasePart") {
        return offset_of(id).into_iter().collect();
    }
    if !world.is_a(id, "Model") {
        return Vec::new();
    }
    if let Some(p) = primary_part(world, id) {
        return offset_of(p).into_iter().collect();
    }
    vec![
        (id, "WorldPivot", Value::CFrame(Box::new(to_cframe(pivot)))),
        (id, "WorldPivotSet", Value::Bool(true)),
    ]
}

/// The writes that restore the default pivot of `id` (Reset pivot): a zero `PivotOffset`, or a
/// Model whose pivot follows its bounding box again.
pub fn reset_writes(world: &World, id: InstanceId) -> Vec<PivotWrite> {
    let identity = || Value::CFrame(Box::new(CFrame::IDENTITY));
    if world.is_a(id, "BasePart") {
        return vec![(id, "PivotOffset", identity())];
    }
    if !world.is_a(id, "Model") {
        return Vec::new();
    }
    match primary_part(world, id) {
        Some(p) => vec![(p, "PivotOffset", identity())],
        None => vec![(id, "WorldPivotSet", Value::Bool(false))],
    }
}

/// True when `id` has a pivot that is not its default.
pub fn has_custom_pivot(world: &World, id: InstanceId) -> bool {
    let target = if world.is_a(id, "Model") {
        match primary_part(world, id) {
            Some(p) => p,
            None => return matches!(world.get_ref(id, "WorldPivotSet"), Ok(Value::Bool(true))),
        }
    } else {
        id
    };
    match world.get_ref(target, "PivotOffset") {
        Ok(Value::CFrame(cf)) => **cf != CFrame::IDENTITY,
        _ => false,
    }
}

/// Models in or under `ids` whose `WorldPivot` is set, with that stored value now: a move of their
/// parts moves it too, as core's `pivot_to` does (also when a `PrimaryPart` gives the pivot).
pub fn models_with_set_pivots(world: &World, ids: &[InstanceId]) -> Vec<(InstanceId, Pivot)> {
    let mut out = Vec::new();
    let mut stack: Vec<InstanceId> = ids.to_vec();
    while let Some(id) = stack.pop() {
        if world.is_a(id, "Model")
            && matches!(world.get_ref(id, "WorldPivotSet"), Ok(Value::Bool(true)))
            && let Ok(Value::CFrame(cf)) = world.get_ref(id, "WorldPivot")
            && !out.iter().any(|(o, _)| *o == id)
        {
            out.push((id, from_cframe(cf)));
        }
        stack.extend_from_slice(world.children(id));
    }
    out
}

/// The 27 snap points of an oriented box (spec 4.4): 8 corners, 12 edge midpoints, 6 face centres
/// and the centre. `centre`, `rotation` and `half` describe the box.
pub fn box_points(centre: DVec3, rotation: DQuat, half: DVec3) -> [DVec3; 27] {
    let mut out = [DVec3::ZERO; 27];
    let mut i = 0;
    for x in [-1.0, 0.0, 1.0] {
        for y in [-1.0, 0.0, 1.0] {
            for z in [-1.0, 0.0, 1.0] {
                out[i] = centre + rotation * (half * DVec3::new(x, y, z));
                i += 1;
            }
        }
    }
    out
}

/// The 27 pivot candidates of `id`: its own box for a part, the bounding box of its parts
/// otherwise.
pub fn candidates(world: &World, id: InstanceId) -> Option<[DVec3; 27]> {
    if world.is_a(id, "BasePart") {
        let t = PartTransform::read(world, id)?;
        return Some(box_points(t.position, t.rotation, t.size * 0.5));
    }
    let b = Aabb::of_parts(world, &Selection::parts_of(world, id))?;
    Some(box_points(b.centre(), DQuat::IDENTITY, b.size() * 0.5))
}

/// A rigid motion: rotate by `rotation` about `about`, then move by `translation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rigid {
    /// Rotation.
    pub rotation: DQuat,
    /// Centre of the rotation.
    pub about: DVec3,
    /// Translation after the rotation.
    pub translation: DVec3,
}

impl Rigid {
    /// No motion.
    pub const IDENTITY: Rigid = Rigid {
        rotation: DQuat::IDENTITY,
        about: DVec3::ZERO,
        translation: DVec3::ZERO,
    };

    /// A pure translation.
    pub fn translate(d: DVec3) -> Rigid {
        Rigid {
            translation: d,
            ..Rigid::IDENTITY
        }
    }

    /// Where point `p` goes.
    pub fn point(&self, p: DVec3) -> DVec3 {
        self.about + self.rotation * (p - self.about) + self.translation
    }

    /// Where frame `f` goes.
    pub fn frame(&self, f: &Pivot) -> Pivot {
        Pivot {
            position: self.point(f.position),
            rotation: (self.rotation * f.rotation).normalize(),
        }
    }

    /// True when the motion changes nothing.
    pub fn is_identity(&self) -> bool {
        self.translation.length_squared() < 1e-24
            && self.rotation.abs_diff_eq(DQuat::IDENTITY, 1e-12)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::{Value, Vec3};

    #[test]
    fn part_pivot_follows_the_part() {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let p = w.create("Part", Some(ws)).unwrap();
        w.set(p, "Position", Value::Vector3(Vec3::new(5.0, 0.0, 0.0)))
            .unwrap();
        w.set(p, "Orientation", Value::Vector3(Vec3::new(0.0, 90.0, 0.0)))
            .unwrap();
        // Pivot one stud along the part's local +X: with a 90 degree turn about Y that is world -Z.
        let world = Pivot::at(DVec3::new(5.0, 0.0, -1.0));
        for (id, prop, v) in pivot_writes(&w, p, &world) {
            w.set(id, prop, v).unwrap();
        }
        assert!(has_custom_pivot(&w, p));
        let now = world_pivot(&w, p).unwrap();
        assert!((now.position - world.position).length() < 1e-9);
        assert!(now.rotation.abs_diff_eq(DQuat::IDENTITY, 1e-9));
        w.set(p, "Position", Value::Vector3(Vec3::new(6.0, 0.0, 0.0)))
            .unwrap();
        let moved = world_pivot(&w, p).unwrap().position;
        assert!((moved - DVec3::new(6.0, 0.0, -1.0)).length() < 1e-9);
        for (id, prop, v) in reset_writes(&w, p) {
            w.set(id, prop, v).unwrap();
        }
        assert!(!has_custom_pivot(&w, p));
        let back = world_pivot(&w, p).unwrap();
        assert!((back.position - DVec3::new(6.0, 0.0, 0.0)).length() < 1e-9);
        // Default pivot axes are the part's own (core and Studio agree on the rotation).
        let t = PartTransform::read(&w, p).unwrap();
        assert!(
            back.rotation.abs_diff_eq(t.rotation, 1e-9)
                || back.rotation.abs_diff_eq(-t.rotation, 1e-9)
        );
    }

    #[test]
    fn a_model_pivot_is_set_then_follows_the_box_again_after_reset() {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let m = w.create("Model", Some(ws)).unwrap();
        let p = w.create("Part", Some(m)).unwrap();
        w.set(p, "Position", Value::Vector3(Vec3::new(2.0, 0.0, 0.0)))
            .unwrap();
        assert!((world_pivot(&w, m).unwrap().position - DVec3::new(2.0, 0.0, 0.0)).length() < 1e-9);
        for (id, prop, v) in pivot_writes(&w, m, &Pivot::at(DVec3::new(9.0, 1.0, 0.0))) {
            w.set(id, prop, v).unwrap();
        }
        assert_eq!(models_with_set_pivots(&w, &[ws]).len(), 1);
        assert!((world_pivot(&w, m).unwrap().position - DVec3::new(9.0, 1.0, 0.0)).length() < 1e-9);
        for (id, prop, v) in reset_writes(&w, m) {
            w.set(id, prop, v).unwrap();
        }
        assert!((world_pivot(&w, m).unwrap().position - DVec3::new(2.0, 0.0, 0.0)).length() < 1e-9);
    }

    #[test]
    fn a_set_world_pivot_moves_even_with_a_primary_part() {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let m = w.create("Model", Some(ws)).unwrap();
        let p = w.create("Part", Some(m)).unwrap();
        for (id, prop, v) in pivot_writes(&w, m, &Pivot::at(DVec3::new(3.0, 0.0, 0.0))) {
            w.set(id, prop, v).unwrap();
        }
        w.set(m, "PrimaryPart", Value::Ref(Some(p))).unwrap();
        assert_eq!(models_with_set_pivots(&w, &[m]).len(), 1);
    }

    #[test]
    fn twenty_seven_points_include_corners_and_centre() {
        let pts = box_points(DVec3::ZERO, DQuat::IDENTITY, DVec3::new(1.0, 2.0, 3.0));
        assert!(pts.contains(&DVec3::new(-1.0, -2.0, -3.0)));
        assert!(pts.contains(&DVec3::ZERO));
        assert!(pts.contains(&DVec3::new(1.0, 0.0, 0.0)));
        assert_eq!(pts.len(), 27);
    }

    #[test]
    fn rigid_moves_points_and_frames() {
        let r = Rigid {
            rotation: DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2),
            about: DVec3::new(1.0, 0.0, 0.0),
            translation: DVec3::Y,
        };
        let p = r.point(DVec3::new(2.0, 0.0, 0.0));
        assert!((p - DVec3::new(1.0, 1.0, -1.0)).length() < 1e-9);
        assert!(Rigid::IDENTITY.is_identity());
    }
}
