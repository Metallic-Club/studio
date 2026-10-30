//! Translate, rotate and scale selected vertices, with optional mirror-symmetric editing.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Quat, Vec3};

use crate::error::{ModelError, ModelResult};
use crate::ids::VertId;
use crate::mesh::Mesh;

/// A coordinate axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Axis {
    /// The X axis (mirror plane YZ).
    #[default]
    X,
    /// The Y axis (mirror plane XZ).
    Y,
    /// The Z axis (mirror plane XY).
    Z,
}

impl Axis {
    /// Index 0, 1 or 2.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }

    /// Unit vector along the axis.
    #[must_use]
    pub fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    /// `p` mirrored across the plane through the origin perpendicular to this axis.
    #[must_use]
    pub fn mirror(self, p: Vec3) -> Vec3 {
        let mut a = p.to_array();
        a[self.index()] = -a[self.index()];
        Vec3::from_array(a)
    }
}

/// Mirror-symmetric editing: moving a vertex also moves its mirror partner across `axis` (matched by position within
/// `tolerance`), and vertices on the mirror plane stay on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Symmetry {
    /// The axis whose plane mirrors edits.
    pub axis: Axis,
    /// Distance under which two positions count as mirror partners or as "on the plane".
    pub tolerance: f32,
}

impl Default for Symmetry {
    fn default() -> Self {
        Self { axis: Axis::X, tolerance: 1e-4 }
    }
}

/// An affine edit of selected vertices: `p' = pivot + rotation * (scale * (p - pivot)) + translation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Movement in model space.
    pub translation: Vec3,
    /// Rotation about the pivot.
    pub rotation: Quat,
    /// Per-axis scale about the pivot.
    pub scale: Vec3,
    /// Centre of rotation and scale.
    pub pivot: Vec3,
    /// Mirror-symmetric editing, if on.
    pub symmetry: Option<Symmetry>,
}

impl Default for Transform {
    fn default() -> Self {
        Self { translation: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE, pivot: Vec3::ZERO, symmetry: None }
    }
}

impl Transform {
    /// A pure move.
    #[must_use]
    pub fn translate(offset: Vec3) -> Self {
        Self { translation: offset, ..Self::default() }
    }

    /// A rotation about `pivot`.
    #[must_use]
    pub fn rotate(rotation: Quat, pivot: Vec3) -> Self {
        Self { rotation, pivot, ..Self::default() }
    }

    /// A scale about `pivot`.
    #[must_use]
    pub fn scale(scale: Vec3, pivot: Vec3) -> Self {
        Self { scale, pivot, ..Self::default() }
    }

    /// Turns on symmetric editing.
    #[must_use]
    pub fn with_symmetry(mut self, symmetry: Symmetry) -> Self {
        self.symmetry = Some(symmetry);
        self
    }

    /// Where `p` goes.
    #[must_use]
    pub fn apply(&self, p: Vec3) -> Vec3 {
        self.pivot + self.rotation * (self.scale * (p - self.pivot)) + self.translation
    }
}

/// Moves `verts` by `xf`. With symmetry on, unselected mirror partners follow with the mirrored transform and
/// vertices on the mirror plane are kept on it.
///
/// # Errors
/// [`ModelError::NothingSelected`] for an empty set, [`ModelError::InvalidParameter`] for a non-finite transform.
pub fn transform(mesh: &mut Mesh, verts: &BTreeSet<VertId>, xf: &Transform) -> ModelResult<()> {
    if verts.is_empty() {
        return Err(ModelError::NothingSelected("vertices to transform"));
    }
    if !(xf.translation.is_finite() && xf.rotation.is_finite() && xf.scale.is_finite() && xf.pivot.is_finite()) {
        return Err(ModelError::InvalidParameter("transform has a non-finite component".into()));
    }
    let mut targets: BTreeMap<VertId, Vec3> = BTreeMap::new();
    for &v in verts {
        targets.insert(v, xf.apply(mesh.position(v)?));
    }
    if let Some(sym) = xf.symmetry {
        let partners = mirror_partners(mesh, sym);
        let axis = sym.axis.index();
        for &v in verts {
            let p = mesh.position(v)?;
            let on_plane = p.to_array()[axis].abs() <= sym.tolerance;
            if on_plane {
                if let Some(t) = targets.get_mut(&v) {
                    let mut a = t.to_array();
                    a[axis] = 0.0;
                    *t = Vec3::from_array(a);
                }
                continue;
            }
            if let Some(&m) = partners.get(&v) {
                if !verts.contains(&m) {
                    let moved = targets.get(&v).copied().unwrap_or(p);
                    targets.insert(m, sym.axis.mirror(moved));
                }
            }
        }
    }
    for (v, p) in targets {
        mesh.set_position(v, p)?;
    }
    Ok(())
}

/// For every vertex off the mirror plane, the vertex at its mirrored position (within tolerance), if any.
#[must_use]
pub fn mirror_partners(mesh: &Mesh, sym: Symmetry) -> BTreeMap<VertId, VertId> {
    let cell = sym.tolerance.max(1e-6) * 4.0;
    let quant = |p: Vec3| ((p.x / cell).round() as i64, (p.y / cell).round() as i64, (p.z / cell).round() as i64);
    let mut grid: BTreeMap<(i64, i64, i64), Vec<(VertId, Vec3)>> = BTreeMap::new();
    for (id, v) in mesh.verts() {
        grid.entry(quant(v.position)).or_default().push((id, v.position));
    }
    let mut out = BTreeMap::new();
    for (id, v) in mesh.verts() {
        let target = sym.axis.mirror(v.position);
        if target.distance(v.position) <= sym.tolerance {
            continue;
        }
        let (cx, cy, cz) = quant(target);
        let mut best: Option<(f32, VertId)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &(other, q) in grid.get(&(cx + dx, cy + dy, cz + dz)).map_or(&[][..], Vec::as_slice) {
                        let d = q.distance(target);
                        if other != id && d <= sym.tolerance && best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, other));
                        }
                    }
                }
            }
        }
        if let Some((_, m)) = best {
            out.insert(id, m);
        }
    }
    out
}
