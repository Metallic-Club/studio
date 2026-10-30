//! Unit meshes for the four part shapes. Each is scaled to the part's `Size` by the instance matrix.
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::f32::consts::{PI, TAU};

/// One mesh vertex: position and unit normal in the mesh's own space.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    /// Position inside the unit cube (-0.5..=0.5 on each axis).
    pub position: [f32; 3],
    /// Unit surface normal.
    pub normal: [f32; 3],
}

/// The shapes a part can have, one shared mesh each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshKind {
    /// `Block`: a cube.
    Box,
    /// `Ball`: a sphere.
    Sphere,
    /// `Cylinder`: a cylinder whose axis is the local X axis (as in Roblox).
    Cylinder,
    /// `Wedge`: a right triangular prism sloping down toward -Z.
    Wedge,
}

impl MeshKind {
    /// All shapes, in the order used for buffers.
    pub const ALL: [MeshKind; 4] = [
        MeshKind::Box,
        MeshKind::Sphere,
        MeshKind::Cylinder,
        MeshKind::Wedge,
    ];

    /// Position of this shape in [`MeshKind::ALL`].
    pub fn index(self) -> usize {
        match self {
            MeshKind::Box => 0,
            MeshKind::Sphere => 1,
            MeshKind::Cylinder => 2,
            MeshKind::Wedge => 3,
        }
    }

    /// Maps the `Shape` property string to a mesh; unknown names draw as a block.
    pub fn from_shape(shape: &str) -> MeshKind {
        match shape {
            "Ball" => MeshKind::Sphere,
            "Cylinder" => MeshKind::Cylinder,
            "Wedge" => MeshKind::Wedge,
            _ => MeshKind::Box,
        }
    }

    /// Builds the mesh data for this shape.
    pub fn build(self) -> MeshData {
        match self {
            MeshKind::Box => box_mesh(),
            MeshKind::Sphere => sphere_mesh(24, 16),
            MeshKind::Cylinder => cylinder_mesh(32),
            MeshKind::Wedge => wedge_mesh(),
        }
    }
}

/// Vertices and triangle indices of one mesh.
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    /// Vertex list.
    pub vertices: Vec<Vertex>,
    /// Triangle list, counter-clockwise when seen from outside.
    pub indices: Vec<u16>,
}

impl MeshData {
    /// Adds a flat convex polygon (fan triangulated). The winding is fixed so the face looks along `normal`.
    fn flat_polygon(&mut self, corners: &[Vec3], normal: Vec3) {
        let base = self.vertices.len() as u16;
        for c in corners {
            self.vertices.push(Vertex {
                position: c.to_array(),
                normal: normal.to_array(),
            });
        }
        let geometric = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
        let flip = geometric.dot(normal) < 0.0;
        for i in 1..corners.len() as u16 - 1 {
            if flip {
                self.indices.extend([base, base + i + 1, base + i]);
            } else {
                self.indices.extend([base, base + i, base + i + 1]);
            }
        }
    }
}

/// A unit cube: 24 vertices (4 per face, flat normals) and 36 indices.
pub fn box_mesh() -> MeshData {
    let mut m = MeshData::default();
    for axis in 0..3 {
        for sign in [-1.0_f32, 1.0] {
            let mut n = Vec3::ZERO;
            n[axis] = sign;
            let u = Vec3::AXES[(axis + 1) % 3];
            let v = Vec3::AXES[(axis + 2) % 3];
            let c = n * 0.5;
            m.flat_polygon(
                &[
                    c - u * 0.5 - v * 0.5,
                    c + u * 0.5 - v * 0.5,
                    c + u * 0.5 + v * 0.5,
                    c - u * 0.5 + v * 0.5,
                ],
                n,
            );
        }
    }
    m
}

/// A right triangular prism: floor, vertical back (+Z), slope from the top back edge down to the front bottom edge, two triangular sides.
pub fn wedge_mesh() -> MeshData {
    let mut m = MeshData::default();
    let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    // Floor.
    m.flat_polygon(
        &[
            p(-0.5, -0.5, -0.5),
            p(0.5, -0.5, -0.5),
            p(0.5, -0.5, 0.5),
            p(-0.5, -0.5, 0.5),
        ],
        -Vec3::Y,
    );
    // Back wall.
    m.flat_polygon(
        &[
            p(-0.5, -0.5, 0.5),
            p(0.5, -0.5, 0.5),
            p(0.5, 0.5, 0.5),
            p(-0.5, 0.5, 0.5),
        ],
        Vec3::Z,
    );
    // Slope.
    m.flat_polygon(
        &[
            p(-0.5, -0.5, -0.5),
            p(0.5, -0.5, -0.5),
            p(0.5, 0.5, 0.5),
            p(-0.5, 0.5, 0.5),
        ],
        Vec3::new(0.0, 1.0, -1.0).normalize(),
    );
    // Triangular sides.
    for sign in [-1.0_f32, 1.0] {
        let x = 0.5 * sign;
        m.flat_polygon(
            &[p(x, -0.5, -0.5), p(x, -0.5, 0.5), p(x, 0.5, 0.5)],
            Vec3::new(sign, 0.0, 0.0),
        );
    }
    m
}

/// A unit-diameter UV sphere with smooth normals.
pub fn sphere_mesh(segments: u16, rings: u16) -> MeshData {
    let mut m = MeshData::default();
    for r in 0..=rings {
        let phi = PI * f32::from(r) / f32::from(rings);
        for s in 0..=segments {
            let theta = TAU * f32::from(s) / f32::from(segments);
            let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            m.vertices.push(Vertex {
                position: (n * 0.5).to_array(),
                normal: n.to_array(),
            });
        }
    }
    let stride = segments + 1;
    for r in 0..rings {
        for s in 0..segments {
            let a = r * stride + s;
            let b = a + stride;
            m.indices.extend([a, a + 1, b]);
            m.indices.extend([a + 1, b + 1, b]);
        }
    }
    orient_outward(&mut m);
    m
}

/// A unit cylinder along the X axis (length 1, diameter 1) with smooth sides and flat caps.
pub fn cylinder_mesh(segments: u16) -> MeshData {
    let mut m = MeshData::default();
    let ring = |s: u16| {
        let a = TAU * f32::from(s) / f32::from(segments);
        (a.cos(), a.sin())
    };
    // Side: two vertices per step, radial normals.
    for s in 0..=segments {
        let (c, sn) = ring(s);
        let n = [0.0, c, sn];
        for x in [-0.5_f32, 0.5] {
            m.vertices.push(Vertex {
                position: [x, c * 0.5, sn * 0.5],
                normal: n,
            });
        }
    }
    for s in 0..segments {
        let i = s * 2;
        m.indices.extend([i, i + 2, i + 1]);
        m.indices.extend([i + 1, i + 2, i + 3]);
    }
    // Caps.
    for sign in [-1.0_f32, 1.0] {
        let n = Vec3::new(sign, 0.0, 0.0);
        let centre = m.vertices.len() as u16;
        m.vertices.push(Vertex {
            position: [0.5 * sign, 0.0, 0.0],
            normal: n.to_array(),
        });
        for s in 0..=segments {
            let (c, sn) = ring(s);
            m.vertices.push(Vertex {
                position: [0.5 * sign, c * 0.5, sn * 0.5],
                normal: n.to_array(),
            });
        }
        for s in 0..segments {
            m.indices.extend([centre, centre + 1 + s, centre + 2 + s]);
        }
    }
    orient_outward(&mut m);
    m
}

/// Flips any triangle whose geometric normal disagrees with its vertex normals.
fn orient_outward(m: &mut MeshData) {
    let vertices = m.vertices.clone();
    for t in m.indices.as_chunks_mut::<3>().0 {
        let p = |i: u16| Vec3::from(vertices[usize::from(i)].position);
        let n = Vec3::from(vertices[usize::from(t[0])].normal);
        let g = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
        if g.dot(n) < 0.0 {
            t.swap(1, 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_mesh_has_24_vertices_36_indices() {
        let m = box_mesh();
        assert_eq!(m.vertices.len(), 24);
        assert_eq!(m.indices.len(), 36);
    }

    #[test]
    fn wedge_mesh_normals_unit_length() {
        let m = wedge_mesh();
        assert!(!m.vertices.is_empty());
        for v in &m.vertices {
            let len = Vec3::from(v.normal).length();
            assert!((len - 1.0).abs() < 1e-5, "normal length {len}");
        }
    }

    #[test]
    fn every_mesh_is_valid_and_wound_outward() {
        for kind in MeshKind::ALL {
            let m = kind.build();
            assert_eq!(m.indices.len() % 3, 0);
            for &i in &m.indices {
                assert!(usize::from(i) < m.vertices.len());
            }
            for v in &m.vertices {
                assert!((Vec3::from(v.normal).length() - 1.0).abs() < 1e-4);
                assert!(v.position.iter().all(|c| c.abs() <= 0.5 + 1e-5));
            }
            for t in m.indices.as_chunks::<3>().0 {
                let p = |i: u16| Vec3::from(m.vertices[usize::from(i)].position);
                let g = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
                if g.length() < 1e-9 {
                    continue; // degenerate pole triangle of the sphere
                }
                let n = Vec3::from(m.vertices[usize::from(t[0])].normal);
                assert!(g.dot(n) > 0.0, "{kind:?} triangle faces inward");
            }
        }
    }

    #[test]
    fn shape_names_map_to_meshes() {
        assert_eq!(MeshKind::from_shape("Block"), MeshKind::Box);
        assert_eq!(MeshKind::from_shape("Ball"), MeshKind::Sphere);
        assert_eq!(MeshKind::from_shape("Cylinder"), MeshKind::Cylinder);
        assert_eq!(MeshKind::from_shape("Wedge"), MeshKind::Wedge);
        for (i, k) in MeshKind::ALL.iter().enumerate() {
            assert_eq!(k.index(), i);
        }
    }
}
