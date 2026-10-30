//! Starter shapes to model from. Each is closed (except the plane and grid), wound outwards and box-UV mapped.
//!
//! Later these seeds should come from `metallic_meshgen` (#35) primitives once it is merged, converted to polygons
//! (quads kept), so a Part and its editable mesh match exactly.

use std::collections::BTreeSet;

use glam::Vec3;

use crate::error::{ModelError, ModelResult};
use crate::ids::{FaceId, VertId};
use crate::mesh::Mesh;
use crate::ops::normals::recalculate_outside;
use crate::uv::box_project;

/// A starter shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Primitive {
    /// A cube with edge length `size`, centred on the origin.
    Cube {
        /// Edge length in studs.
        size: f32,
    },
    /// A single quad in the XZ plane facing +Y.
    Plane {
        /// Edge length in studs.
        size: f32,
    },
    /// A quad grid in the XZ plane facing +Y.
    Grid {
        /// Quads along X.
        x: u32,
        /// Quads along Z.
        z: u32,
        /// Edge length in studs.
        size: f32,
    },
    /// A capped cylinder along Y with n-gon caps.
    Cylinder {
        /// Sides around.
        segments: u32,
        /// Radius in studs.
        radius: f32,
        /// Height in studs.
        height: f32,
    },
    /// A UV sphere: quads with triangle fans at the poles.
    UvSphere {
        /// Segments around.
        segments: u32,
        /// Rings from pole to pole.
        rings: u32,
        /// Radius in studs.
        radius: f32,
    },
}

impl Primitive {
    /// Builds the shape as a new mesh centred on `center`.
    ///
    /// # Errors
    /// [`ModelError::InvalidParameter`] for non-positive sizes or too few segments.
    pub fn build(&self, center: Vec3) -> ModelResult<Mesh> {
        let mut mesh = Mesh::new();
        self.add_to(&mut mesh, center)?;
        Ok(mesh)
    }

    /// Adds the shape to an existing mesh and returns its new faces.
    ///
    /// # Errors
    /// As [`Primitive::build`].
    pub fn add_to(&self, mesh: &mut Mesh, center: Vec3) -> ModelResult<BTreeSet<FaceId>> {
        let bad = |what: &str| Err(ModelError::InvalidParameter(format!("primitive {what}")));
        let mut faces: BTreeSet<FaceId> = BTreeSet::new();
        let mut add = |mesh: &mut Mesh, vs: &[VertId]| -> ModelResult<()> {
            faces.insert(mesh.add_face(vs)?);
            Ok(())
        };
        match *self {
            Primitive::Cube { size } => {
                if !(size.is_finite() && size > 0.0) {
                    return bad("size");
                }
                let h = size * 0.5;
                let v: Vec<VertId> = (0..8)
                    .map(|i| {
                        let p = Vec3::new(
                            if i & 1 == 0 { -h } else { h },
                            if i & 2 == 0 { -h } else { h },
                            if i & 4 == 0 { -h } else { h },
                        );
                        mesh.add_vert(center + p)
                    })
                    .collect();
                // Corner index bits: 1 = +X, 2 = +Y, 4 = +Z.
                for q in [
                    [4, 5, 7, 6],
                    [1, 0, 2, 3],
                    [5, 1, 3, 7],
                    [0, 4, 6, 2],
                    [6, 7, 3, 2],
                    [0, 1, 5, 4],
                ] {
                    add(mesh, &q.map(|i| v[i]))?;
                }
            }
            Primitive::Plane { size } => {
                return Primitive::Grid { x: 1, z: 1, size }.add_to(mesh, center);
            }
            Primitive::Grid { x, z, size } => {
                if x == 0 || z == 0 || x > 1024 || z > 1024 || !(size.is_finite() && size > 0.0) {
                    return bad("grid dimensions");
                }
                let (nx, nz) = (x as usize, z as usize);
                let mut ids = vec![vec![VertId(0); nz + 1]; nx + 1];
                for (i, col) in ids.iter_mut().enumerate() {
                    for (j, id) in col.iter_mut().enumerate() {
                        let p = Vec3::new(
                            -size * 0.5 + size * i as f32 / nx as f32,
                            0.0,
                            -size * 0.5 + size * j as f32 / nz as f32,
                        );
                        *id = mesh.add_vert(center + p);
                    }
                }
                for i in 0..nx {
                    for j in 0..nz {
                        add(
                            mesh,
                            &[ids[i][j], ids[i][j + 1], ids[i + 1][j + 1], ids[i + 1][j]],
                        )?;
                    }
                }
            }
            Primitive::Cylinder {
                segments,
                radius,
                height,
            } => {
                if !(3..=512).contains(&segments) || !(radius > 0.0 && height > 0.0) {
                    return bad("cylinder parameters");
                }
                let n = segments as usize;
                let ring = |mesh: &mut Mesh, y: f32| -> Vec<VertId> {
                    (0..n)
                        .map(|k| {
                            let a = std::f32::consts::TAU * k as f32 / n as f32;
                            mesh.add_vert(center + Vec3::new(radius * a.cos(), y, radius * a.sin()))
                        })
                        .collect()
                };
                let bottom = ring(mesh, -height * 0.5);
                let top = ring(mesh, height * 0.5);
                for k in 0..n {
                    let k1 = (k + 1) % n;
                    add(mesh, &[bottom[k], top[k], top[k1], bottom[k1]])?;
                }
                add(mesh, &bottom)?;
                let rev: Vec<VertId> = top.iter().rev().copied().collect();
                add(mesh, &rev)?;
            }
            Primitive::UvSphere {
                segments,
                rings,
                radius,
            } => {
                if !(3..=512).contains(&segments) || !(2..=512).contains(&rings) || radius <= 0.0 {
                    return bad("sphere parameters");
                }
                let (n, r) = (segments as usize, rings as usize);
                let north = mesh.add_vert(center + Vec3::Y * radius);
                let south = mesh.add_vert(center - Vec3::Y * radius);
                let mut lat: Vec<Vec<VertId>> = Vec::new();
                for i in 1..r {
                    let phi = std::f32::consts::PI * i as f32 / r as f32;
                    let row = (0..n)
                        .map(|k| {
                            let a = std::f32::consts::TAU * k as f32 / n as f32;
                            let p = Vec3::new(phi.sin() * a.cos(), phi.cos(), phi.sin() * a.sin())
                                * radius;
                            mesh.add_vert(center + p)
                        })
                        .collect();
                    lat.push(row);
                }
                for k in 0..n {
                    let k1 = (k + 1) % n;
                    add(mesh, &[north, lat[0][k1], lat[0][k]])?;
                    for i in 0..r - 2 {
                        add(
                            mesh,
                            &[lat[i][k], lat[i][k1], lat[i + 1][k1], lat[i + 1][k]],
                        )?;
                    }
                    add(mesh, &[south, lat[r - 2][k], lat[r - 2][k1]])?;
                }
            }
        }
        if !matches!(self, Primitive::Grid { .. } | Primitive::Plane { .. }) {
            recalculate_outside(mesh, &faces)?;
        }
        box_project(mesh, &faces, 1.0)?;
        Ok(faces)
    }
}
