//! Shared geometry for the building tools: part snapshots, oriented box corners, world
//! bounding boxes and per-axis gaps.
//!
//! Every tool core reads parts through [`PartSnapshot`] (see [`snapshot`]) and uses the
//! [`Aabb`] here; no tool module defines its own box type. Rotations follow the part's
//! `Orientation` order (Y, then X, then Z), converted through `metallic_core::CFrame` so the
//! tools, the renderer and Luau agree.

use glam::{DQuat, DVec3, EulerRot};
use metallic_core::{CFrame, InstanceId, Value, Vec3, World};

/// A part's placement, read once from the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartSnapshot<I = InstanceId> {
    /// The instance.
    pub id: I,
    /// `Position`: the box centre in world space (studs).
    pub position: DVec3,
    /// World rotation, from `Orientation`.
    pub rotation: DQuat,
    /// `Size` in studs.
    pub size: DVec3,
}

impl<I: Copy> PartSnapshot<I> {
    /// The part's frame as a `metallic_core` [`CFrame`].
    pub fn cframe(&self) -> CFrame {
        let r = self.rotation;
        CFrame::from_quaternion(to_core(self.position), [r.x, r.y, r.z, r.w])
    }

    /// The eight corners of the part's box.
    pub fn corners(&self) -> [DVec3; 8] {
        box_corners(self.position, self.rotation, self.size)
    }

    /// The part's world-space bounding box.
    pub fn aabb(&self) -> Aabb {
        Aabb::from_oriented_box(self.position, self.rotation, self.size)
    }
}

fn to_glam(v: Vec3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

fn to_core(v: DVec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// Snapshots of the given instances. Ids that are destroyed, or are not a `BasePart`, are
/// skipped; the order of `ids` is kept. Reads borrow the world's values (no clones).
pub fn snapshot(world: &World, ids: &[InstanceId]) -> Vec<PartSnapshot> {
    let vec3 = |id: InstanceId, prop: &str| match world.get_ref(id, prop) {
        Ok(Value::Vector3(v)) => Some(*v),
        _ => None,
    };
    ids.iter()
        .filter(|id| world.is_a(**id, "BasePart"))
        .filter_map(|&id| {
            let position = vec3(id, "Position")?;
            let orientation = vec3(id, "Orientation")?;
            let size = vec3(id, "Size")?;
            let [x, y, z, w] =
                CFrame::from_position_orientation(position, orientation).quaternion();
            Some(PartSnapshot {
                id,
                position: to_glam(position),
                rotation: DQuat::from_xyzw(x, y, z, w).normalize(),
                size: to_glam(size),
            })
        })
        .collect()
}

/// Rotation from a Roblox-style `Orientation` in degrees (applied Y, then X, then Z).
pub fn rotation_from_orientation_degrees(orientation: DVec3) -> DQuat {
    DQuat::from_euler(
        EulerRot::YXZ,
        orientation.y.to_radians(),
        orientation.x.to_radians(),
        orientation.z.to_radians(),
    )
}

/// The eight corners of a box with this centre, rotation and size.
pub fn box_corners(centre: DVec3, rotation: DQuat, size: DVec3) -> [DVec3; 8] {
    let h = size * 0.5;
    let mut out = [DVec3::ZERO; 8];
    for (i, c) in out.iter_mut().enumerate() {
        let sx = if i & 1 == 0 { -h.x } else { h.x };
        let sy = if i & 2 == 0 { -h.y } else { h.y };
        let sz = if i & 4 == 0 { -h.z } else { h.z };
        *c = centre + rotation * DVec3::new(sx, sy, sz);
    }
    out
}

/// An axis-aligned bounding box in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Smallest corner.
    pub min: DVec3,
    /// Largest corner.
    pub max: DVec3,
}

impl Aabb {
    /// The box around a set of points; `None` when there are none.
    pub fn from_points(points: impl IntoIterator<Item = DVec3>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        Some(it.fold(
            Self {
                min: first,
                max: first,
            },
            |b, p| Self {
                min: b.min.min(p),
                max: b.max.max(p),
            },
        ))
    }

    /// The world box around a rotated part.
    pub fn from_oriented_box(centre: DVec3, rotation: DQuat, size: DVec3) -> Self {
        let r = glam::DMat3::from_quat(rotation);
        let h = size.abs() * 0.5;
        // Half extent on each world axis: sum of |r_ij| * h_j.
        let ext = DVec3::new(
            r.x_axis.x.abs() * h.x + r.y_axis.x.abs() * h.y + r.z_axis.x.abs() * h.z,
            r.x_axis.y.abs() * h.x + r.y_axis.y.abs() * h.y + r.z_axis.y.abs() * h.z,
            r.x_axis.z.abs() * h.x + r.y_axis.z.abs() * h.y + r.z_axis.z.abs() * h.z,
        );
        Self {
            min: centre - ext,
            max: centre + ext,
        }
    }

    /// The box around both.
    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Size per axis.
    pub fn size(&self) -> DVec3 {
        self.max - self.min
    }
}

/// Gap between two boxes on each axis: positive is empty space between them, zero is
/// touching, negative is how far they overlap on that axis.
pub fn axis_gaps(a: &Aabb, b: &Aabb) -> DVec3 {
    (b.min - a.max).max(a.min - b.max)
}

/// Shortest distance between two boxes; 0 when they touch or overlap.
pub fn gap_distance(a: &Aabb, b: &Aabb) -> f64 {
    axis_gaps(a, b).max(DVec3::ZERO).length()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reads_parts_and_skips_the_rest() {
        let mut world = World::new_with_services();
        let ws = world.root("Workspace").unwrap();
        let part = world.create("Part", Some(ws)).unwrap();
        world
            .set(part, "Position", Value::Vector3(Vec3::new(1.0, 2.0, 3.0)))
            .unwrap();
        world
            .set(
                part,
                "Orientation",
                Value::Vector3(Vec3::new(10.0, 35.0, -20.0)),
            )
            .unwrap();
        world
            .set(part, "Size", Value::Vector3(Vec3::new(4.0, 1.0, 2.0)))
            .unwrap();
        let folder = world.create("Folder", Some(ws)).unwrap();
        let gone = world.create("Part", Some(ws)).unwrap();
        world.destroy(gone);

        let snaps = snapshot(&world, &[folder, part, gone]);
        assert_eq!(snaps.len(), 1);
        let s = snaps[0];
        assert_eq!(s.id, part);
        assert_eq!(s.position, DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(s.size, DVec3::new(4.0, 1.0, 2.0));
        // Same rotation as the Y-X-Z Orientation helper and as the core CFrame.
        let expect = rotation_from_orientation_degrees(DVec3::new(10.0, 35.0, -20.0));
        assert!(s.rotation.dot(expect).abs() > 1.0 - 1e-12);
        let o = s.cframe().orientation_degrees();
        assert!((to_glam(o) - DVec3::new(10.0, 35.0, -20.0)).length() < 1e-9);
        assert!((to_glam(s.cframe().position) - s.position).length() < 1e-12);
        // Corners and box agree with the free functions.
        assert_eq!(s.corners(), box_corners(s.position, s.rotation, s.size));
        let from_corners = Aabb::from_points(s.corners()).unwrap();
        assert!((s.aabb().min - from_corners.min).length() < 1e-12);
        assert!((s.aabb().max - from_corners.max).length() < 1e-12);
    }

    #[test]
    fn box_corners_are_a_box() {
        let rot = rotation_from_orientation_degrees(DVec3::new(30.0, 45.0, 0.0));
        let c = box_corners(DVec3::ONE, rot, DVec3::new(4.0, 1.0, 2.0));
        let diag = DVec3::new(4.0, 1.0, 2.0).length();
        // Opposite corners (i and 7 - i) are a full diagonal apart.
        for i in 0..4 {
            assert!((c[i].distance(c[7 - i]) - diag).abs() < 1e-12);
        }
        // Edge along local X.
        assert!((c[0].distance(c[1]) - 4.0).abs() < 1e-12);
        let centre = c.iter().copied().sum::<DVec3>() / 8.0;
        assert!((centre - DVec3::ONE).length() < 1e-12);
    }

    #[test]
    fn oriented_aabb_matches_its_corners() {
        let rot = rotation_from_orientation_degrees(DVec3::new(12.0, 70.0, -33.0));
        let centre = DVec3::new(2.0, -1.0, 5.0);
        let size = DVec3::new(3.0, 0.5, 6.0);
        let from_corners = Aabb::from_points(box_corners(centre, rot, size)).unwrap();
        let direct = Aabb::from_oriented_box(centre, rot, size);
        assert!((from_corners.min - direct.min).length() < 1e-12);
        assert!((from_corners.max - direct.max).length() < 1e-12);
        assert_eq!(Aabb::from_points([]), None);
    }

    #[test]
    fn axis_gap_readouts() {
        let a = Aabb {
            min: DVec3::ZERO,
            max: DVec3::new(2.0, 2.0, 2.0),
        };
        // 3 studs to the right on X, overlapping on Y by 1, touching on Z.
        let b = Aabb {
            min: DVec3::new(5.0, 1.0, 2.0),
            max: DVec3::new(6.0, 4.0, 3.0),
        };
        let g = axis_gaps(&a, &b);
        assert_eq!(g, DVec3::new(3.0, -1.0, 0.0));
        assert_eq!(axis_gaps(&b, &a), g, "symmetric");
        assert_eq!(gap_distance(&a, &b), 3.0);
        // Diagonal gap.
        let c = Aabb {
            min: DVec3::new(5.0, 6.0, 0.0),
            max: DVec3::new(6.0, 7.0, 1.0),
        };
        assert_eq!(gap_distance(&a, &c), 5.0);
        // A box inside another: negative on every axis, distance 0.
        let inner = Aabb {
            min: DVec3::splat(0.5),
            max: DVec3::splat(1.0),
        };
        assert!(axis_gaps(&a, &inner).cmplt(DVec3::ZERO).all());
        assert_eq!(gap_distance(&a, &inner), 0.0);
        // The selection's box is the union of its parts.
        let sel = a.union(&inner).union(&c);
        assert_eq!(sel.size(), DVec3::new(6.0, 7.0, 2.0));
    }
}
