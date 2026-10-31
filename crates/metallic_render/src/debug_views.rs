//! Debug views (#23): see how the renderer and the physics see the scene.
//!
//! [`DebugView`] switches the whole frame ([`crate::Renderer::set_debug_view`]): wireframe,
//! normals, overdraw heat, shadow cascade colours, or the exposure false-colour light meter.
//! [`DebugShape`]s are wireframe overlays drawn over any view
//! ([`crate::Renderer::set_debug_shapes`]); the runtime fills them from the physics debug draw
//! (colliders, contacts, velocities, touch volumes). With the view off and no shapes the frame
//! costs what it did before: one uniform value is read per pixel, nothing else runs.
use crate::meshes::{MeshData, MeshKind, cylinder_mesh, sphere_mesh};
use crate::scene::InstanceRaw;
use glam::{Mat4, Quat, Vec3};

/// A whole-frame debug view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum DebugView {
    /// The normal image.
    #[default]
    Off,
    /// Part edges only, over a dark background (characters stay shaded).
    Wireframe,
    /// World-space normals as colours (x red, y green, z blue).
    Normals,
    /// How many surfaces cover each pixel: brighter and warmer is more.
    Overdraw,
    /// Which shadow cascade covers each surface (violet, blue, amber; grey past the last).
    ShadowCascades,
    /// Light meter: the exposed scene in false colour, one band per stop around middle grey.
    Exposure,
}

impl DebugView {
    /// Every view, in menu order.
    pub const ALL: [DebugView; 6] = [
        DebugView::Off,
        DebugView::Wireframe,
        DebugView::Normals,
        DebugView::Overdraw,
        DebugView::ShadowCascades,
        DebugView::Exposure,
    ];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            DebugView::Off => "Off",
            DebugView::Wireframe => "Wireframe",
            DebugView::Normals => "Normals",
            DebugView::Overdraw => "Overdraw",
            DebugView::ShadowCascades => "Shadow cascades",
            DebugView::Exposure => "Exposure (light meter)",
        }
    }

    /// Command-line name (`wireframe`, `normals`, `overdraw`, `cascades`, `exposure`, `off`).
    pub fn name(self) -> &'static str {
        match self {
            DebugView::Off => "off",
            DebugView::Wireframe => "wireframe",
            DebugView::Normals => "normals",
            DebugView::Overdraw => "overdraw",
            DebugView::ShadowCascades => "cascades",
            DebugView::Exposure => "exposure",
        }
    }

    /// The view called `name` (see [`DebugView::name`]).
    pub fn from_name(name: &str) -> Option<DebugView> {
        DebugView::ALL.into_iter().find(|v| v.name() == name)
    }

    /// The next view, wrapping to `Off` (a key that cycles through them).
    pub fn next(self) -> DebugView {
        let i = DebugView::ALL.iter().position(|v| *v == self).unwrap_or(0);
        DebugView::ALL[(i + 1) % DebugView::ALL.len()]
    }

    /// `g.debug.x` in the shaders (`common.wgsl`, `debug.wgsl`).
    pub(crate) fn shader_mode(self) -> f32 {
        match self {
            DebugView::Off => 0.0,
            DebugView::Wireframe => 1.0,
            DebugView::Normals => 2.0,
            DebugView::Overdraw => 3.0,
            DebugView::ShadowCascades => 4.0,
            DebugView::Exposure => 5.0,
        }
    }

    /// Whether auto exposure keeps metering in this view. The views that replace the scene's
    /// light hold the exposure they started with, so the meter never chases its own output.
    pub(crate) fn meters(self) -> bool {
        matches!(self, DebugView::Off | DebugView::Exposure)
    }
}

/// One wireframe overlay: a unit mesh placed by `transform` (scale included), in `colour`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugShape {
    /// Which unit mesh (the same ones parts use: a box, sphere, cylinder along X, wedge).
    pub kind: MeshKind,
    /// Translation * rotation * scale.
    pub transform: Mat4,
    /// Display colour (linear RGB, 0..1); drawn as is, whatever the exposure.
    pub colour: [f32; 3],
}

impl DebugShape {
    /// A mesh of `size` at `position`, turned by `rotation`.
    pub fn oriented(
        kind: MeshKind,
        position: Vec3,
        rotation: Quat,
        size: Vec3,
        colour: [f32; 3],
    ) -> DebugShape {
        DebugShape {
            kind,
            transform: Mat4::from_scale_rotation_translation(size, rotation, position),
            colour,
        }
    }

    /// A thin box from `a` to `b` (a line `width` studs thick); `None` when they coincide.
    pub fn segment(a: Vec3, b: Vec3, width: f32, colour: [f32; 3]) -> Option<DebugShape> {
        let d = b - a;
        let len = d.length();
        if len.is_nan() || len <= 1e-4 {
            return None;
        }
        let rotation = Quat::from_rotation_arc(Vec3::X, d / len);
        Some(DebugShape::oriented(
            MeshKind::Box,
            (a + b) * 0.5,
            rotation,
            Vec3::new(len, width, width),
            colour,
        ))
    }

    /// A capsule from `a` to `b` of `radius`: a cylinder and two spheres, added to `out`.
    pub fn capsule(a: Vec3, b: Vec3, radius: f32, colour: [f32; 3], out: &mut Vec<DebugShape>) {
        let d = 2.0 * radius;
        let len = (b - a).length();
        if len > 1e-4 {
            let rotation = Quat::from_rotation_arc(Vec3::X, (b - a) / len);
            out.push(DebugShape::oriented(
                MeshKind::Cylinder,
                (a + b) * 0.5,
                rotation,
                Vec3::new(len, d, d),
                colour,
            ));
        }
        for end in [a, b] {
            out.push(DebugShape::oriented(
                MeshKind::Sphere,
                end,
                Quat::IDENTITY,
                Vec3::splat(d),
                colour,
            ));
        }
    }

    pub(crate) fn instance(&self) -> InstanceRaw {
        InstanceRaw {
            model: self.transform.to_cols_array_2d(),
            color: [self.colour[0], self.colour[1], self.colour[2], 1.0],
            material: [0.0; 4],
        }
    }
}

/// The unit mesh debug shapes of `kind` are drawn with: round shapes with fewer segments than
/// parts use, so a wireframe stays readable.
pub(crate) fn coarse_mesh(kind: MeshKind) -> MeshData {
    match kind {
        MeshKind::Sphere => sphere_mesh(12, 6),
        MeshKind::Cylinder => cylinder_mesh(12),
        other => other.build(),
    }
}

/// The edges of `mesh` as a line list: every triangle edge once, except the edges between two
/// triangles of one flat face (a box shows its 12 edges, not the diagonals of its sides).
pub fn edge_indices(mesh: &MeshData) -> Vec<u16> {
    use std::collections::HashMap;
    let normal = |t: &[u16]| {
        let p = |i: u16| Vec3::from(mesh.vertices[i as usize].position);
        (p(t[1]) - p(t[0]))
            .cross(p(t[2]) - p(t[0]))
            .normalize_or_zero()
    };
    // Edge (low, high) -> the normal of the first triangle that had it, and whether a second
    // triangle with a different normal shares it (or none does).
    let mut edges: HashMap<(u16, u16), (Vec3, bool)> = HashMap::new();
    let mut order = Vec::new();
    for t in mesh.indices.as_chunks::<3>().0 {
        let n = normal(t);
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let key = (a.min(b), a.max(b));
            match edges.get_mut(&key) {
                Some((first, keep)) => *keep = first.dot(n) < 0.999,
                None => {
                    edges.insert(key, (n, true));
                    order.push(key);
                }
            }
        }
    }
    order
        .into_iter()
        .filter(|k| edges[k].1)
        .flat_map(|(a, b)| [a, b])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_faces_keep_their_outline_but_not_their_diagonals() {
        // Box faces have their own vertices, so each cube edge appears once per side; the
        // diagonals inside a side are dropped.
        let edges = |k: MeshKind| edge_indices(&k.build()).len() / 2;
        assert_eq!(edges(MeshKind::Box), 24);
        assert_eq!(edges(MeshKind::Wedge), 18);
        assert!(edges(MeshKind::Sphere) > 100);
    }

    #[test]
    fn views_round_trip_by_name_and_cycle() {
        for v in DebugView::ALL {
            assert_eq!(DebugView::from_name(v.name()), Some(v));
        }
        let mut v = DebugView::Off;
        for _ in 0..DebugView::ALL.len() {
            v = v.next();
        }
        assert_eq!(v, DebugView::Off);
        assert!(DebugView::Off.meters() && !DebugView::Overdraw.meters());
    }

    #[test]
    fn a_segment_spans_its_ends() {
        let s = DebugShape::segment(Vec3::ZERO, Vec3::new(0.0, 4.0, 0.0), 0.1, [1.0; 3]).unwrap();
        let top = s.transform.transform_point3(Vec3::new(0.5, 0.0, 0.0));
        assert!((top - Vec3::new(0.0, 4.0, 0.0)).length() < 1e-5);
        assert!(DebugShape::segment(Vec3::ONE, Vec3::ONE, 0.1, [1.0; 3]).is_none());
        let mut out = Vec::new();
        DebugShape::capsule(Vec3::ZERO, Vec3::Y, 0.5, [1.0; 3], &mut out);
        assert_eq!(out.len(), 3);
    }
}
