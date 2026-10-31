//! Transform maths shared by picking, gizmos and the building tools (f64, studs and degrees).
//!
//! `Orientation` follows Roblox: degrees, applied as Y, then X, then Z (the renderer uses the
//! same order), so a rotation is `Ry(y) * Rx(x) * Rz(z)`.
use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_render::glam::{DQuat, DVec3, EulerRot};

/// Converts a core vector to glam.
pub fn to_d(v: Vec3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

/// Converts a glam vector to core.
pub fn from_d(v: DVec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// The rotation an `Orientation` (degrees, Y then X then Z) stands for.
pub fn orientation_to_quat(o: Vec3) -> DQuat {
    DQuat::from_euler(
        EulerRot::YXZ,
        o.y.to_radians(),
        o.x.to_radians(),
        o.z.to_radians(),
    )
}

/// The `Orientation` (degrees) of a rotation, with float noise below 1e-6 degrees rounded away so
/// a 90 degree turn reads as 90, not 89.99999999.
pub fn quat_to_orientation(q: DQuat) -> Vec3 {
    let (y, x, z) = q.normalize().to_euler(EulerRot::YXZ);
    let clean = |r: f64| {
        let d = r.to_degrees();
        let rounded = (d * 1e6).round() / 1e6;
        if rounded == 0.0 { 0.0 } else { rounded }
    };
    Vec3::new(clean(x), clean(y), clean(z))
}

/// A ray in world space; `dir` is unit length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    /// Start point.
    pub origin: DVec3,
    /// Unit direction.
    pub dir: DVec3,
}

impl Ray {
    /// The point at distance `t`.
    pub fn at(&self, t: f64) -> DVec3 {
        self.origin + self.dir * t
    }

    /// Distance along the ray to the plane through `point` with `normal`, if in front.
    pub fn plane_hit(&self, point: DVec3, normal: DVec3) -> Option<f64> {
        let denom = self.dir.dot(normal);
        if denom.abs() < 1e-9 {
            return None;
        }
        let t = (point - self.origin).dot(normal) / denom;
        (t >= 0.0).then_some(t)
    }

    /// Parameter `s` of the point on the line `point + axis * s` closest to this ray
    /// (`axis` unit length). `None` when the ray runs parallel to the line.
    pub fn closest_on_line(&self, point: DVec3, axis: DVec3) -> Option<f64> {
        let w0 = point - self.origin;
        let b = axis.dot(self.dir);
        let denom = 1.0 - b * b;
        if denom.abs() < 1e-9 {
            return None;
        }
        let d = axis.dot(w0);
        let e = self.dir.dot(w0);
        Some((b * e - d) / denom)
    }
}

/// Position, rotation and size of one BasePart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartTransform {
    /// Centre in world space.
    pub position: DVec3,
    /// Rotation (from `Orientation`).
    pub rotation: DQuat,
    /// Size in studs.
    pub size: DVec3,
}

impl PartTransform {
    /// Reads a BasePart's `Position`, `Orientation` and `Size`; `None` for other instances.
    pub fn read(world: &World, id: InstanceId) -> Option<PartTransform> {
        let v = |p: &str| match world.get_ref(id, p) {
            Ok(Value::Vector3(v)) => Some(*v),
            _ => None,
        };
        Some(PartTransform {
            position: to_d(v("Position")?),
            rotation: orientation_to_quat(v("Orientation")?),
            size: to_d(v("Size")?),
        })
    }

    /// The local axis `i` (0 = X, 1 = Y, 2 = Z) in world space.
    pub fn axis(&self, i: usize) -> DVec3 {
        self.rotation * DVec3::AXES[i]
    }

    /// The eight corners in world space.
    pub fn corners(&self) -> [DVec3; 8] {
        let h = self.size * 0.5;
        let mut out = [DVec3::ZERO; 8];
        for (i, c) in out.iter_mut().enumerate() {
            let s = DVec3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            );
            *c = self.position + self.rotation * (h * s);
        }
        out
    }

    /// Distance along `ray` to the box surface (or 0 when the ray starts inside).
    pub fn ray_hit(&self, ray: &Ray) -> Option<f64> {
        let inv = self.rotation.inverse();
        let o = inv * (ray.origin - self.position);
        let d = inv * ray.dir;
        ray_box(o, d, self.size * 0.5)
    }

    /// Distance along `ray` to the box surface and the outward normal (world space) of the face
    /// hit. A ray starting inside reports distance 0 and the normal facing back along the ray.
    pub fn ray_hit_normal(&self, ray: &Ray) -> Option<(f64, DVec3)> {
        let inv = self.rotation.inverse();
        let o = inv * (ray.origin - self.position);
        let d = inv * ray.dir;
        let h = self.size * 0.5;
        let mut t0 = f64::NEG_INFINITY;
        let mut t1 = f64::INFINITY;
        let mut normal = DVec3::ZERO;
        for i in 0..3 {
            if d[i].abs() < 1e-12 {
                if o[i].abs() > h[i] {
                    return None;
                }
                continue;
            }
            let a = (-h[i] - o[i]) / d[i];
            let b = (h[i] - o[i]) / d[i];
            let (near, far, sign) = if a < b { (a, b, -1.0) } else { (b, a, 1.0) };
            if near > t0 {
                t0 = near;
                normal = DVec3::ZERO;
                normal[i] = sign;
            }
            t1 = t1.min(far);
            if t0 > t1 {
                return None;
            }
        }
        if t1 < 0.0 {
            return None;
        }
        if t0 < 0.0 {
            return Some((0.0, -ray.dir));
        }
        Some((t0, (self.rotation * normal).normalize_or(DVec3::Y)))
    }

    /// Distance along `ray` to the inscribed sphere (Ball parts).
    pub fn ray_hit_sphere(&self, ray: &Ray) -> Option<f64> {
        let r = self.size.min_element() * 0.5;
        let oc = ray.origin - self.position;
        let b = oc.dot(ray.dir);
        let c = oc.length_squared() - r * r;
        let disc = b * b - c;
        if disc < 0.0 {
            return None;
        }
        let s = disc.sqrt();
        let t = -b - s;
        if t >= 0.0 {
            Some(t)
        } else if -b + s >= 0.0 {
            Some(0.0)
        } else {
            None
        }
    }
}

/// Slab test of a ray against an axis-aligned box centred on the origin with half size `h`.
pub fn ray_box(o: DVec3, d: DVec3, h: DVec3) -> Option<f64> {
    let mut t0 = f64::NEG_INFINITY;
    let mut t1 = f64::INFINITY;
    for i in 0..3 {
        if d[i].abs() < 1e-12 {
            if o[i].abs() > h[i] {
                return None;
            }
            continue;
        }
        let a = (-h[i] - o[i]) / d[i];
        let b = (h[i] - o[i]) / d[i];
        let (near, far) = if a < b { (a, b) } else { (b, a) };
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    if t1 < 0.0 { None } else { Some(t0.max(0.0)) }
}

/// An axis-aligned box in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Smallest corner.
    pub min: DVec3,
    /// Largest corner.
    pub max: DVec3,
}

impl Aabb {
    /// The box around `points`; `None` when empty.
    pub fn from_points(points: impl IntoIterator<Item = DVec3>) -> Option<Aabb> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let mut b = Aabb {
            min: first,
            max: first,
        };
        for p in it {
            b.min = b.min.min(p);
            b.max = b.max.max(p);
        }
        Some(b)
    }

    /// The box around the given parts (rotated parts by their corners).
    pub fn of_parts(world: &World, parts: &[InstanceId]) -> Option<Aabb> {
        Aabb::from_points(
            parts
                .iter()
                .filter_map(|&id| PartTransform::read(world, id))
                .flat_map(|t| t.corners()),
        )
    }

    /// Centre point.
    pub fn centre(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }

    /// Size on each axis.
    pub fn size(&self) -> DVec3 {
        self.max - self.min
    }
}

/// Rounds `value` to the nearest multiple of `step` (no rounding when `step` is 0 or less).
pub fn snap(value: f64, step: f64) -> f64 {
    if step > 0.0 {
        (value / step).round() * step
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_round_trips_through_quaternions() {
        for o in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(30.0, 45.0, 60.0),
            Vec3::new(-20.0, 170.0, 5.0),
            Vec3::new(0.0, 90.0, 0.0),
        ] {
            let back = quat_to_orientation(orientation_to_quat(o));
            assert!((back - o).magnitude() < 1e-6, "{o:?} -> {back:?}");
        }
    }

    #[test]
    fn orientation_order_is_y_then_x_then_z() {
        // Orientation (90, 90, 0) must equal Ry(90) * Rx(90).
        let o = Vec3::new(90.0, 90.0, 0.0);
        let q = orientation_to_quat(o);
        let explicit =
            DQuat::from_rotation_y(90f64.to_radians()) * DQuat::from_rotation_x(90f64.to_radians());
        assert!((q * DVec3::Z - explicit * DVec3::Z).length() < 1e-9);
    }

    #[test]
    fn ray_hits_rotated_box_and_misses_beside_it() {
        let t = PartTransform {
            position: DVec3::new(0.0, 0.0, 0.0),
            rotation: DQuat::from_rotation_y(45f64.to_radians()),
            size: DVec3::new(2.0, 2.0, 2.0),
        };
        let down = Ray {
            origin: DVec3::new(0.0, 10.0, 0.0),
            dir: DVec3::NEG_Y,
        };
        assert!((t.ray_hit(&down).unwrap() - 9.0).abs() < 1e-9);
        // 1.3 studs out along X is inside the rotated square's corner (reaches 1.414) ...
        let corner = Ray {
            origin: DVec3::new(1.3, 10.0, 0.0),
            dir: DVec3::NEG_Y,
        };
        assert!(t.ray_hit(&corner).is_some());
        // ... and 1.5 is outside.
        let miss = Ray {
            origin: DVec3::new(1.5, 10.0, 0.0),
            dir: DVec3::NEG_Y,
        };
        assert!(t.ray_hit(&miss).is_none());
    }

    #[test]
    fn face_normal_of_the_face_hit() {
        let t = PartTransform {
            position: DVec3::ZERO,
            rotation: DQuat::from_rotation_x(30f64.to_radians()),
            size: DVec3::new(10.0, 1.0, 10.0),
        };
        let down = Ray {
            origin: DVec3::new(0.0, 10.0, 0.0),
            dir: DVec3::NEG_Y,
        };
        let (_, n) = t.ray_hit_normal(&down).unwrap();
        assert!((n - t.axis(1)).length() < 1e-9, "{n:?}");
        let side = Ray {
            origin: DVec3::new(20.0, 0.0, 0.0),
            dir: DVec3::NEG_X,
        };
        let (d, n) = t.ray_hit_normal(&side).unwrap();
        assert!((d - 15.0).abs() < 1e-9 && (n - DVec3::X).length() < 1e-9);
    }

    #[test]
    fn closest_point_on_axis_line() {
        let ray = Ray {
            origin: DVec3::new(5.0, 10.0, 0.0),
            dir: DVec3::NEG_Y,
        };
        let s = ray.closest_on_line(DVec3::ZERO, DVec3::X).unwrap();
        assert!((s - 5.0).abs() < 1e-9);
    }

    #[test]
    fn snap_rounds_to_increment() {
        assert_eq!(snap(1.26, 0.5), 1.5);
        assert_eq!(snap(-0.74, 0.5), -0.5);
        assert_eq!(snap(1.26, 0.0), 1.26);
    }
}
