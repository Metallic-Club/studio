//! Animation clips, poses and their evaluation on the CPU.
//!
//! A [`Clip`] holds keyframe [`Track`]s, one per animated joint channel. [`Clip::sample`] writes
//! the clip's value at a time into a [`Pose`] (local transforms, one per joint);
//! [`Pose::blend`] mixes two poses; [`Pose::skin_matrices`] produces the joint matrices for
//! linear-blend skinning.
use crate::skeleton::Skeleton;
use glam::{Mat4, Quat, Vec3, Vec4};

/// A joint's local transform: scale, then rotation, then translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Translation relative to the parent joint.
    pub translation: Vec3,
    /// Rotation (unit quaternion).
    pub rotation: Quat,
    /// Scale along the joint's axes.
    pub scale: Vec3,
}

impl Transform {
    /// No translation, rotation or scale.
    pub const IDENTITY: Transform = Transform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// The transform as a matrix.
    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Interpolates from `self` (at `t = 0`) to `other` (at `t = 1`): translation and scale
    /// linearly, rotation along the shorter arc (normalised linear interpolation, the usual choice
    /// for blending poses).
    pub fn lerp(&self, other: &Transform, t: f32) -> Transform {
        let b = if self.rotation.dot(other.rotation) < 0.0 {
            -other.rotation
        } else {
            other.rotation
        };
        Transform {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.lerp(b, t).normalize(),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

/// Local transforms of every joint of a skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    /// One local transform per joint, in skeleton order.
    pub locals: Vec<Transform>,
}

impl Pose {
    /// The skeleton's rest pose.
    pub fn rest(skeleton: &Skeleton) -> Pose {
        Pose {
            locals: skeleton.rest.clone(),
        }
    }

    /// Moves this pose toward `other` by `weight` (0 keeps this pose, 1 gives `other`), joint by
    /// joint. Joints missing from either pose are left unchanged.
    pub fn blend(&mut self, other: &Pose, weight: f32) {
        let w = weight.clamp(0.0, 1.0);
        if w <= 0.0 {
            return;
        }
        for (a, b) in self.locals.iter_mut().zip(&other.locals) {
            *a = if w >= 1.0 { *b } else { a.lerp(b, w) };
        }
    }

    /// Fills `out` with one skinning matrix per joint:
    /// `model * joint model-space matrix * inverse bind matrix`. A bind-pose vertex multiplied by
    /// the weighted sum of its joints' matrices lands where `model` puts the posed mesh.
    pub fn skin_matrices(&self, skeleton: &Skeleton, model: Mat4, out: &mut Vec<Mat4>) {
        let n = skeleton.len().min(self.locals.len());
        out.clear();
        // First the model-space matrix of each joint (parents come first), then the bind.
        for j in 0..n {
            let local = self.locals[j].to_mat4();
            let parent = match skeleton.parents[j] {
                Some(p) if p < j => out[p],
                _ => model * skeleton.base,
            };
            out.push(parent * local);
        }
        for (m, ibm) in out.iter_mut().zip(&skeleton.inverse_bind) {
            *m *= *ibm;
        }
    }
}

/// How a track's value changes between keyframes (glTF sampler interpolation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    /// Straight line between keys (spherical for rotations).
    Linear,
    /// Holds each key's value until the next key.
    Step,
    /// Cubic Hermite spline with in and out tangents stored beside each value.
    CubicSpline,
}

/// Which part of a joint's transform a track drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Translation, 3 values per key.
    Translation,
    /// Rotation quaternion `x, y, z, w`, 4 values per key.
    Rotation,
    /// Scale, 3 values per key.
    Scale,
}

impl Channel {
    /// Values per key (3 or 4).
    pub fn width(self) -> usize {
        match self {
            Channel::Rotation => 4,
            Channel::Translation | Channel::Scale => 3,
        }
    }
}

/// Keyframes of one channel of one joint.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Joint index (skeleton order).
    pub joint: usize,
    /// The driven channel.
    pub channel: Channel,
    /// Interpolation between keys.
    pub interpolation: Interpolation,
    /// Key times in seconds; at least one, non-negative and non-decreasing.
    pub times: Vec<f32>,
    /// Key values, `width` floats per key; for [`Interpolation::CubicSpline`] each key holds an
    /// in-tangent, the value and an out-tangent (`3 * width` floats).
    pub values: Vec<f32>,
}

impl Track {
    /// The value at `time` (clamped to the first and last key) as a 4-vector (`w` unused for
    /// translation and scale).
    pub fn sample(&self, time: f32) -> Vec4 {
        let width = self.channel.width();
        let keys = self.times.len();
        let (stride, offset) = match self.interpolation {
            Interpolation::CubicSpline => (3 * width, width),
            _ => (width, 0),
        };
        let value = |k: usize| -> Vec4 { self.vec(k * stride + offset, width) };
        if keys == 0 {
            return Vec4::ZERO;
        }
        // Index of the last key at or before `time`.
        let next = self.times.partition_point(|&t| t <= time);
        if next == 0 {
            return value(0);
        }
        if next >= keys {
            return value(keys - 1);
        }
        let k = next - 1;
        let (t0, t1) = (self.times[k], self.times[next]);
        let span = t1 - t0;
        let u = if span > 0.0 {
            ((time - t0) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let rotation = self.channel == Channel::Rotation;
        match self.interpolation {
            Interpolation::Step => value(k),
            Interpolation::Linear if rotation => {
                let a = Quat::from_vec4(value(k));
                let b = Quat::from_vec4(value(next));
                Vec4::from(a.slerp(b, u))
            }
            Interpolation::Linear => value(k).lerp(value(next), u),
            Interpolation::CubicSpline => {
                let out_tangent = self.vec(k * stride + 2 * width, width);
                let in_tangent = self.vec(next * stride, width);
                let (u2, u3) = (u * u, u * u * u);
                let v = value(k) * (2.0 * u3 - 3.0 * u2 + 1.0)
                    + out_tangent * (span * (u3 - 2.0 * u2 + u))
                    + value(next) * (-2.0 * u3 + 3.0 * u2)
                    + in_tangent * (span * (u3 - u2));
                if rotation { v.normalize_or(Vec4::W) } else { v }
            }
        }
    }

    /// `width` floats starting at `at`, padded to a 4-vector.
    fn vec(&self, at: usize, width: usize) -> Vec4 {
        let mut v = [0.0; 4];
        if let Some(src) = self.values.get(at..at + width) {
            v[..width].copy_from_slice(src);
        }
        Vec4::from_array(v)
    }
}

/// A named animation: tracks for some joints' channels.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    /// The glTF animation name (`idle`, `walk`, ...).
    pub name: String,
    /// Time of the last key of any track, in seconds.
    pub duration: f32,
    /// Keyframe tracks.
    pub tracks: Vec<Track>,
}

impl Clip {
    /// Writes the clip's value at `time` (seconds, clamped to `0..=duration`) into `pose`. Only
    /// the channels the clip animates are written; reset the pose first (for example to
    /// [`Pose::rest`]) when the clip does not animate every joint. A looping caller passes
    /// `time.rem_euclid(duration)`.
    pub fn sample(&self, time: f32, pose: &mut Pose) {
        let time = if time.is_finite() {
            time.clamp(0.0, self.duration)
        } else {
            0.0
        };
        for track in &self.tracks {
            let Some(local) = pose.locals.get_mut(track.joint) else {
                continue;
            };
            let v = track.sample(time);
            match track.channel {
                Channel::Translation => local.translation = v.truncate(),
                Channel::Scale => local.scale = v.truncate(),
                Channel::Rotation => local.rotation = Quat::from_vec4(v).normalize(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(
        channel: Channel,
        interpolation: Interpolation,
        times: &[f32],
        values: &[f32],
    ) -> Track {
        Track {
            joint: 0,
            channel,
            interpolation,
            times: times.to_vec(),
            values: values.to_vec(),
        }
    }

    #[test]
    fn linear_translation_interpolates_and_clamps() {
        let t = track(
            Channel::Translation,
            Interpolation::Linear,
            &[0.0, 1.0, 3.0],
            &[0.0, 0.0, 0.0, 2.0, 4.0, 0.0, 2.0, 0.0, 0.0],
        );
        assert_eq!(t.sample(0.5).truncate(), Vec3::new(1.0, 2.0, 0.0));
        assert_eq!(t.sample(2.0).truncate(), Vec3::new(2.0, 2.0, 0.0));
        assert_eq!(t.sample(-1.0).truncate(), Vec3::ZERO);
        assert_eq!(t.sample(10.0).truncate(), Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(t.sample(1.0).truncate(), Vec3::new(2.0, 4.0, 0.0));
    }

    #[test]
    fn step_holds_the_previous_key() {
        let t = track(
            Channel::Scale,
            Interpolation::Step,
            &[0.0, 1.0],
            &[1.0, 1.0, 1.0, 2.0, 2.0, 2.0],
        );
        assert_eq!(t.sample(0.99).truncate(), Vec3::ONE);
        assert_eq!(t.sample(1.0).truncate(), Vec3::splat(2.0));
    }

    #[test]
    fn linear_rotation_is_spherical() {
        let a = Quat::IDENTITY;
        let b = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let mut values = a.to_array().to_vec();
        values.extend(b.to_array());
        let t = track(
            Channel::Rotation,
            Interpolation::Linear,
            &[0.0, 1.0],
            &values,
        );
        let q = Quat::from_vec4(t.sample(0.5));
        let want = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        assert!(q.angle_between(want) < 1e-5, "{q:?}");
    }

    #[test]
    fn cubic_spline_follows_the_hermite_curve() {
        // Value 0 -> 1 over 2 s with zero tangents: smoothstep, so 0.5 at the middle and
        // 0.15625 a quarter in (3u^2 - 2u^3 at u = 0.25).
        let key = |v: f32| [0.0, 0.0, 0.0, v, 0.0, 0.0, 0.0, 0.0, 0.0];
        let mut values = key(0.0).to_vec();
        values.extend(key(1.0));
        let t = track(
            Channel::Translation,
            Interpolation::CubicSpline,
            &[0.0, 2.0],
            &values,
        );
        assert!((t.sample(1.0).x - 0.5).abs() < 1e-6);
        assert!((t.sample(0.5).x - 0.15625).abs() < 1e-6);
        // A slope of 1 unit/s out of the first key and into the second, from 0 to 2: a line.
        let values = [
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0,
            0.0,
        ];
        let t = track(
            Channel::Translation,
            Interpolation::CubicSpline,
            &[0.0, 2.0],
            &values,
        );
        assert!((t.sample(0.5).x - 0.5).abs() < 1e-6);
    }

    #[test]
    fn blend_mixes_translation_and_takes_the_short_rotation_arc() {
        let a = Pose {
            locals: vec![Transform::IDENTITY],
        };
        let mut mixed = a.clone();
        let b = Pose {
            locals: vec![Transform {
                translation: Vec3::new(4.0, 0.0, 0.0),
                // The same 90 degree turn written as the negated quaternion.
                rotation: -Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                scale: Vec3::splat(3.0),
            }],
        };
        mixed.blend(&b, 0.25);
        let m = mixed.locals[0];
        assert_eq!(m.translation, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(m.scale, Vec3::splat(1.5));
        let angle = m.rotation.angle_between(Quat::IDENTITY);
        assert!(
            (angle - 0.25 * std::f32::consts::FRAC_PI_2).abs() < 0.02,
            "{angle}"
        );
        let mut full = a.clone();
        full.blend(&b, 1.0);
        assert_eq!(full, b);
        let mut none = a.clone();
        none.blend(&b, 0.0);
        assert_eq!(none, a);
    }

    #[test]
    fn skin_matrices_are_identity_in_the_bind_pose() {
        let rest = vec![
            Transform {
                translation: Vec3::new(0.0, 1.0, 0.0),
                ..Transform::IDENTITY
            },
            Transform {
                translation: Vec3::new(0.0, 0.5, 0.0),
                rotation: Quat::from_rotation_x(0.3),
                ..Transform::IDENTITY
            },
        ];
        let globals = [rest[0].to_mat4(), rest[0].to_mat4() * rest[1].to_mat4()];
        let skeleton = Skeleton {
            names: vec!["a".into(), "b".into()],
            parents: vec![None, Some(0)],
            rest,
            inverse_bind: globals.iter().map(|m| m.inverse()).collect(),
            base: Mat4::IDENTITY,
        };
        let mut out = Vec::new();
        Pose::rest(&skeleton).skin_matrices(&skeleton, Mat4::IDENTITY, &mut out);
        for m in &out {
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-5), "{m:?}");
        }
        let model = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
        Pose::rest(&skeleton).skin_matrices(&skeleton, model, &mut out);
        assert!(out[1].abs_diff_eq(model, 1e-5));
    }
}
