//! Pivots: the point (and axes) an object moves, rotates and scales about (spec 4.4).
//!
//! Roblox names: a BasePart's pivot is `PivotOffset` (relative to the part), a Model's is
//! `WorldPivot` (world space). Defaults: the part's centre; for a Model, the centre of its parts'
//! bounding box (it follows the parts until a pivot is set).
//!
//! Until `PivotOffset` / `WorldPivot` exist as properties in `metallic_core` (#216), Studio keeps
//! pivots set in the editor in a [`PivotTable`] owned by [`crate::commands::History`], so edits
//! are undoable like everything else. They last for the session; they are not saved in the scene
//! file yet. When #216 lands, [`Command::SetPivots`](crate::commands::Command::SetPivots) becomes
//! `SetProperties` on those properties and this table goes away; callers use [`world_pivot`] and
//! do not change.
use std::collections::HashMap;

use metallic_core::{InstanceId, World};
use metallic_render::glam::{DQuat, DVec3};

use crate::selection::Selection;
use crate::transform::{Aabb, PartTransform};

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

/// Pivots set in the editor: part offsets (relative) and Model pivots (world space).
pub type PivotTable = HashMap<InstanceId, Pivot>;

/// A part's frame as a [`Pivot`].
pub fn part_frame(t: &PartTransform) -> Pivot {
    Pivot {
        position: t.position,
        rotation: t.rotation,
    }
}

/// The world-space pivot of `id`: a part's frame times its offset, a Model's stored pivot or its
/// bounding-box centre; other containers use their bounding-box centre. `None` when `id` has no
/// parts.
pub fn world_pivot(world: &World, table: &PivotTable, id: InstanceId) -> Option<Pivot> {
    if world.is_a(id, "BasePart") {
        let t = PartTransform::read(world, id)?;
        let offset = table.get(&id).copied().unwrap_or(Pivot::IDENTITY);
        return Some(part_frame(&t).then(&offset));
    }
    if let Some(p) = table.get(&id)
        && world.is_a(id, "Model")
    {
        return Some(*p);
    }
    let parts = Selection::parts_of(world, id);
    Aabb::of_parts(world, &parts).map(|b| Pivot::at(b.centre()))
}

/// The value to store for `id` so that its world pivot becomes `pivot`: an offset for a part, the
/// world pivot itself for a Model. `None` for other classes (they have no pivot of their own).
pub fn stored_for(world: &World, id: InstanceId, pivot: &Pivot) -> Option<Pivot> {
    if world.is_a(id, "BasePart") {
        let t = PartTransform::read(world, id)?;
        return Some(part_frame(&t).to_local(pivot));
    }
    world.is_a(id, "Model").then_some(*pivot)
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
        let mut table = PivotTable::new();
        // Pivot one stud along the part's local +X: with a 90 degree turn about Y that is world -Z.
        let world = Pivot::at(DVec3::new(5.0, 0.0, -1.0));
        let offset = stored_for(&w, p, &world).unwrap();
        table.insert(p, offset);
        assert!((world_pivot(&w, &table, p).unwrap().position - world.position).length() < 1e-9);
        w.set(p, "Position", Value::Vector3(Vec3::new(6.0, 0.0, 0.0)))
            .unwrap();
        let moved = world_pivot(&w, &table, p).unwrap().position;
        assert!((moved - DVec3::new(6.0, 0.0, -1.0)).length() < 1e-9);
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
