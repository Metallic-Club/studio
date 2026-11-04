//! The built-in primitive meshes (Block, Ball, Cylinder, Wedge) in one shared vertex and index pool.
//!
//! Every mesh is unit-sized, centred on the origin, and stretched to the part by the instance's affine transform
//! (`Size`, with the Ball and Cylinder rules applied by the scene's `PrimitiveMesh`). All of them live in one
//! vertex buffer and one index buffer, so the draws of every kind share their bindings and only the index range
//! changes; mesh assets join the same pool later (ADR 0008 §4). Indices are absolute (`u32`, base vertex 0), which
//! GLES needs: it has no base vertex.
//!
//! The Ball and the Cylinder have levels of detail ([`crate::lod`]): at full detail they are tessellated finely
//! enough that their silhouettes stay round at the sizes parts are usually seen at (32 segments around, 16 rings for
//! the Ball), and each coarser level halves the segments (and rings), keeping a subset of the finer level's vertices.
//! The pool holds every level of every kind, **level by level**: every kind's full detail first, then every coarser
//! level in turn ([`MeshPool::level_starts`]), so a vertex's index alone says which level it belongs to (the debug
//! tint of [`crate::Renderer::set_lod_tint`]).

use std::f32::consts::{PI, TAU};

use bytemuck::{Pod, Zeroable};
use metallic_core::glam::Vec3;

use crate::lod::{BALL_SEGMENTS, CYLINDER_SEGMENTS, MAX_LOD_LEVELS};
use crate::scene::MeshKind;

/// One vertex of a primitive mesh: position and unit normal in the unit mesh's own space.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct MeshVertex {
    /// Position, inside the unit cube (−0.5 to 0.5 on every axis).
    pub position: [f32; 3],
    /// Unit normal.
    pub normal: [f32; 3],
}

impl MeshVertex {
    /// The vertex buffer layout: position at location 0, normal at location 1.
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    /// The layout of the pool's vertex buffer, stepped per vertex.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// The vertices and triangles of one mesh, wound counter-clockwise seen from outside.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    /// The vertices.
    pub vertices: Vec<MeshVertex>,
    /// Three indices per triangle, into `vertices`.
    pub indices: Vec<u32>,
}

impl MeshData {
    /// The built-in mesh of `kind` at full detail; `None` for kinds that are not built in (mesh assets).
    pub fn primitive(kind: MeshKind) -> Option<Self> {
        Self::primitive_level(kind, 0)
    }

    /// The built-in mesh of `kind` at level of detail `level` (0 is full detail, see [`crate::lod`]); `None` for
    /// kinds that are not built in and for levels the kind does not have.
    pub fn primitive_level(kind: MeshKind, level: usize) -> Option<Self> {
        Some(match (kind, level) {
            (MeshKind::BLOCK, 0) => block(),
            (MeshKind::BALL, _) => {
                let segments = *BALL_SEGMENTS.get(level)?;
                ball(segments, segments / 2)
            }
            (MeshKind::CYLINDER, _) => cylinder(*CYLINDER_SEGMENTS.get(level)?),
            (MeshKind::WEDGE, 0) => wedge(),
            _ => return None,
        })
    }

    /// Adds a flat convex polygon facing `normal`, fanned into triangles.
    fn polygon(&mut self, corners: &[Vec3], normal: Vec3) {
        let base = self.next_index();
        for corner in corners {
            self.vertices.push(MeshVertex {
                position: corner.to_array(),
                normal: normal.to_array(),
            });
        }
        let Some((&first, rest)) = corners.split_first() else {
            return;
        };
        let flip = rest.len() >= 2 && (rest[0] - first).cross(rest[1] - first).dot(normal) < 0.0;
        for i in 1..rest.len() as u32 {
            if flip {
                self.indices.extend([base, base + i + 1, base + i]);
            } else {
                self.indices.extend([base, base + i, base + i + 1]);
            }
        }
    }

    fn next_index(&self) -> u32 {
        u32::try_from(self.vertices.len()).unwrap_or(u32::MAX)
    }
}

/// A unit cube: 24 vertices (flat normals, four per face), 12 triangles.
fn block() -> MeshData {
    let mut mesh = MeshData::default();
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut normal = Vec3::ZERO;
            normal[axis] = sign;
            let u = Vec3::AXES[(axis + 1) % 3] * 0.5;
            let v = Vec3::AXES[(axis + 2) % 3] * 0.5;
            let centre = normal * 0.5;
            mesh.polygon(
                &[centre - u - v, centre + u - v, centre + u + v, centre - u + v],
                normal,
            );
        }
    }
    mesh
}

/// A ramp: a floor, a vertical back at +Z and a slope from the top of the back down to the front bottom edge
/// (−Z), with two triangular sides.
fn wedge() -> MeshData {
    let mut mesh = MeshData::default();
    let p = Vec3::new;
    mesh.polygon(
        &[
            p(-0.5, -0.5, -0.5),
            p(0.5, -0.5, -0.5),
            p(0.5, -0.5, 0.5),
            p(-0.5, -0.5, 0.5),
        ],
        Vec3::NEG_Y,
    );
    mesh.polygon(
        &[
            p(-0.5, -0.5, 0.5),
            p(0.5, -0.5, 0.5),
            p(0.5, 0.5, 0.5),
            p(-0.5, 0.5, 0.5),
        ],
        Vec3::Z,
    );
    mesh.polygon(
        &[
            p(-0.5, -0.5, -0.5),
            p(0.5, -0.5, -0.5),
            p(0.5, 0.5, 0.5),
            p(-0.5, 0.5, 0.5),
        ],
        Vec3::new(0.0, 1.0, -1.0).normalize(),
    );
    for sign in [-1.0f32, 1.0] {
        let x = 0.5 * sign;
        mesh.polygon(&[p(x, -0.5, -0.5), p(x, -0.5, 0.5), p(x, 0.5, 0.5)], Vec3::X * sign);
    }
    mesh
}

/// A sphere of diameter 1 with smooth normals: `segments` around, `rings` from pole to pole. The degenerate
/// triangles at the poles are left out.
fn ball(segments: u32, rings: u32) -> MeshData {
    let mut mesh = MeshData::default();
    for ring in 0..=rings {
        let phi = PI * ring as f32 / rings as f32;
        for segment in 0..=segments {
            let theta = TAU * segment as f32 / segments as f32;
            let normal = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            mesh.vertices.push(MeshVertex {
                position: (normal * 0.5).to_array(),
                normal: normal.to_array(),
            });
        }
    }
    let stride = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * stride + segment;
            let b = a + stride;
            if ring != 0 {
                mesh.indices.extend([a, a + 1, b]);
            }
            if ring != rings - 1 {
                mesh.indices.extend([a + 1, b + 1, b]);
            }
        }
    }
    orient_outward(&mut mesh);
    mesh
}

/// A cylinder along X, length 1 and diameter 1: smooth sides and flat caps.
fn cylinder(segments: u32) -> MeshData {
    let mut mesh = MeshData::default();
    let around = |segment: u32| {
        let angle = TAU * segment as f32 / segments as f32;
        (angle.cos(), angle.sin())
    };
    for segment in 0..=segments {
        let (c, s) = around(segment);
        for x in [-0.5f32, 0.5] {
            mesh.vertices.push(MeshVertex {
                position: [x, c * 0.5, s * 0.5],
                normal: [0.0, c, s],
            });
        }
    }
    for segment in 0..segments {
        let i = segment * 2;
        mesh.indices.extend([i, i + 2, i + 1, i + 1, i + 2, i + 3]);
    }
    for sign in [-1.0f32, 1.0] {
        let normal = (Vec3::X * sign).to_array();
        let centre = mesh.next_index();
        mesh.vertices.push(MeshVertex {
            position: [0.5 * sign, 0.0, 0.0],
            normal,
        });
        for segment in 0..segments {
            let (c, s) = around(segment);
            mesh.vertices.push(MeshVertex {
                position: [0.5 * sign, c * 0.5, s * 0.5],
                normal,
            });
        }
        for segment in 0..segments {
            let next = (segment + 1) % segments;
            mesh.indices.extend([centre, centre + 1 + segment, centre + 1 + next]);
        }
    }
    orient_outward(&mut mesh);
    mesh
}

/// Flips every triangle whose winding disagrees with its first vertex's normal.
fn orient_outward(mesh: &mut MeshData) {
    let vertices = &mesh.vertices;
    for triangle in mesh.indices.as_chunks_mut::<3>().0 {
        let position = |i: u32| vertices.get(i as usize).map_or(Vec3::ZERO, |v| Vec3::from(v.position));
        let normal = vertices
            .get(triangle[0] as usize)
            .map_or(Vec3::ZERO, |v| Vec3::from(v.normal));
        let winding =
            (position(triangle[1]) - position(triangle[0])).cross(position(triangle[2]) - position(triangle[0]));
        if winding.dot(normal) < 0.0 {
            triangle.swap(1, 2);
        }
    }
}

/// Where one mesh's triangles are in the pool's index buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshRange {
    /// The first index.
    pub first_index: u32,
    /// How many indices (three per triangle).
    pub index_count: u32,
}

/// Every built-in mesh, at every level of detail, in one vertex buffer and one index buffer.
#[derive(Debug)]
pub struct MeshPool {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// Indexed by mesh kind, then level; an empty range for kinds and levels the pool does not have.
    ranges: Vec<[MeshRange; MAX_LOD_LEVELS]>,
    /// The first vertex of each level.
    level_starts: [u32; MAX_LOD_LEVELS],
}

/// The pool's contents: every built-in mesh level by level, the ranges and where each level's vertices start.
type PoolData = (
    Vec<MeshVertex>,
    Vec<u32>,
    Vec<[MeshRange; MAX_LOD_LEVELS]>,
    [u32; MAX_LOD_LEVELS],
);

fn pool_data() -> PoolData {
    let mut vertices: Vec<MeshVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut ranges = vec![[MeshRange::default(); MAX_LOD_LEVELS]; usize::from(MeshKind::BUILTIN_COUNT)];
    let mut level_starts = [0u32; MAX_LOD_LEVELS];
    for (level, start) in level_starts.iter_mut().enumerate() {
        *start = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
        for (kind, kind_ranges) in (0..MeshKind::BUILTIN_COUNT).zip(ranges.iter_mut()) {
            let Some(mesh) = MeshData::primitive_level(MeshKind(kind), level) else {
                continue;
            };
            let base = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
            let first_index = u32::try_from(indices.len()).unwrap_or(u32::MAX);
            vertices.extend_from_slice(&mesh.vertices);
            indices.extend(mesh.indices.iter().map(|&i| base + i));
            kind_ranges[level] = MeshRange {
                first_index,
                index_count: u32::try_from(mesh.indices.len()).unwrap_or(u32::MAX),
            };
        }
    }
    (vertices, indices, ranges, level_starts)
}

impl MeshPool {
    /// Builds the primitives and uploads them.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let (vertices, indices, ranges, level_starts) = pool_data();
        let buffer = |label, contents: &[u8], usage| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: contents.len() as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, contents);
            buffer
        };
        Self {
            vertices: buffer(
                "metallic mesh vertices",
                bytemuck::cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            indices: buffer(
                "metallic mesh indices",
                bytemuck::cast_slice(&indices),
                wgpu::BufferUsages::INDEX,
            ),
            ranges,
            level_starts,
        }
    }

    /// The vertex buffer ([`MeshVertex`]).
    pub fn vertices(&self) -> &wgpu::Buffer {
        &self.vertices
    }

    /// The index buffer (`u32`, absolute).
    pub fn indices(&self) -> &wgpu::Buffer {
        &self.indices
    }

    /// The index range of `kind`'s mesh at level `level` (0: full detail); empty when the pool does not have it.
    pub fn range(&self, kind: MeshKind, level: usize) -> MeshRange {
        self.ranges
            .get(usize::from(kind.0))
            .and_then(|levels| levels.get(level))
            .copied()
            .unwrap_or_default()
    }

    /// The first vertex of each level in the vertex buffer: level `i`'s meshes are the vertices from
    /// `level_starts()[i]` to the next level's start.
    pub fn level_starts(&self) -> [u32; MAX_LOD_LEVELS] {
        self.level_starts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lod;

    /// Every built-in kind at every level it has.
    fn kinds() -> impl Iterator<Item = (MeshKind, MeshData)> {
        (0..MeshKind::BUILTIN_COUNT).flat_map(|k| {
            (0..lod::levels(MeshKind(k)))
                .map(move |level| (MeshKind(k), MeshData::primitive_level(MeshKind(k), level).unwrap()))
        })
    }

    #[test]
    fn levels_halve_the_triangles_of_round_kinds() {
        let triangles = |kind: MeshKind| -> Vec<usize> {
            (0..MAX_LOD_LEVELS + 1)
                .map_while(|level| MeshData::primitive_level(kind, level))
                .map(|mesh| mesh.indices.len() / 3)
                .collect()
        };
        assert_eq!(triangles(MeshKind::BALL), [960, 224, 48, 8]);
        assert_eq!(triangles(MeshKind::CYLINDER), [128, 64, 32]);
        assert_eq!(triangles(MeshKind::BLOCK), [12]);
        assert_eq!(triangles(MeshKind::WEDGE), [8]);
        for kind in 0..MeshKind::BUILTIN_COUNT {
            assert_eq!(triangles(MeshKind(kind)).len(), lod::levels(MeshKind(kind)));
        }
    }

    #[test]
    fn every_level_is_inscribed_in_the_finer_ones() {
        // Every vertex of a coarser level is a vertex of the finer level: the coarse surface is inside the fine one,
        // which is what lets a shadow be coarser than the surface receiving it (crate::lod).
        for kind in [MeshKind::BALL, MeshKind::CYLINDER] {
            for level in 1..lod::levels(kind) {
                let fine = MeshData::primitive_level(kind, level - 1).unwrap();
                let coarse = MeshData::primitive_level(kind, level).unwrap();
                for v in &coarse.vertices {
                    let p = Vec3::from(v.position);
                    assert!(
                        fine.vertices.iter().any(|f| Vec3::from(f.position).distance(p) < 1e-6),
                        "{kind:?} level {level}: {p} is not a vertex of level {}",
                        level - 1
                    );
                }
            }
        }
    }

    #[test]
    fn the_pool_lays_the_levels_out_one_after_another() {
        let (vertices, indices, ranges, starts) = pool_data();
        assert_eq!(starts[0], 0);
        assert!(starts.windows(2).all(|w| w[0] < w[1]), "{starts:?}");
        for (kind, levels) in ranges.iter().enumerate() {
            let kind = MeshKind(kind as u16);
            for (level, range) in levels.iter().enumerate() {
                if level >= lod::levels(kind) {
                    assert_eq!(*range, MeshRange::default());
                    continue;
                }
                let mesh = MeshData::primitive_level(kind, level).unwrap();
                assert_eq!(range.index_count as usize, mesh.indices.len());
                // Every vertex the range uses is in its level's span of the pool.
                let end = starts.get(level + 1).copied().unwrap_or(vertices.len() as u32);
                let used = &indices[range.first_index as usize..(range.first_index + range.index_count) as usize];
                assert!(used.iter().all(|&i| i >= starts[level] && i < end), "{kind:?} {level}");
            }
        }
    }

    #[test]
    fn every_primitive_is_unit_sized_and_valid() {
        for (kind, mesh) in kinds() {
            assert_eq!(mesh.indices.len() % 3, 0, "{kind:?}");
            assert!(
                mesh.indices.iter().all(|&i| (i as usize) < mesh.vertices.len()),
                "{kind:?}"
            );
            let mut max = Vec3::ZERO;
            for v in &mesh.vertices {
                assert!(
                    (Vec3::from(v.normal).length() - 1.0).abs() < 1e-5,
                    "{kind:?}: {:?}",
                    v.normal
                );
                let p = Vec3::from(v.position);
                assert!(p.abs().max_element() <= 0.5 + 1e-6, "{kind:?}: {p}");
                max = max.max(p.abs());
            }
            // Every primitive touches the unit cube's faces along each axis.
            assert!(max.abs_diff_eq(Vec3::splat(0.5), 1e-5), "{kind:?}: {max}");
        }
    }

    #[test]
    fn every_triangle_faces_outward() {
        for (kind, mesh) in kinds() {
            // Every primitive is convex, so the mean of its vertices is inside it.
            let inside =
                mesh.vertices.iter().map(|v| Vec3::from(v.position)).sum::<Vec3>() / mesh.vertices.len() as f32;
            for t in mesh.indices.as_chunks::<3>().0 {
                let p = |i: u32| Vec3::from(mesh.vertices[i as usize].position);
                let winding = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
                assert!(winding.length() > 1e-7, "{kind:?}: degenerate triangle {t:?}");
                let centroid = (p(t[0]) + p(t[1]) + p(t[2])) / 3.0;
                assert!(
                    winding.dot(centroid - inside) > 0.0,
                    "{kind:?}: triangle {t:?} faces inward"
                );
                let normal = Vec3::from(mesh.vertices[t[0] as usize].normal);
                assert!(
                    winding.dot(normal) > 0.0,
                    "{kind:?}: triangle {t:?} disagrees with its normal"
                );
            }
        }
    }

    #[test]
    fn closed_meshes_have_the_expected_area() {
        let area = |mesh: &MeshData| -> f32 {
            mesh.indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| {
                    let p = |i: u32| Vec3::from(mesh.vertices[i as usize].position);
                    (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).length() / 2.0
                })
                .sum()
        };
        let block = MeshData::primitive(MeshKind::BLOCK).unwrap();
        assert!((area(&block) - 6.0).abs() < 1e-5);
        let wedge = MeshData::primitive(MeshKind::WEDGE).unwrap();
        assert!((area(&wedge) - (2.0 + 2.0f32.sqrt() + 1.0)).abs() < 1e-5);
        let ball = MeshData::primitive(MeshKind::BALL).unwrap();
        assert!((area(&ball) - PI).abs() < 0.05 * PI, "{}", area(&ball));
        let cylinder = MeshData::primitive(MeshKind::CYLINDER).unwrap();
        let expected = PI + 2.0 * PI * 0.25;
        assert!(
            (area(&cylinder) - expected).abs() < 0.02 * expected,
            "{}",
            area(&cylinder)
        );
    }

    #[test]
    fn the_wedge_slopes_down_towards_minus_z() {
        let wedge = MeshData::primitive(MeshKind::WEDGE).unwrap();
        // The top edge is at the back (+Z); nothing is high at the front.
        for v in &wedge.vertices {
            if v.position[1] > 0.0 {
                assert!(v.position[2] > 0.0, "{:?}", v.position);
            }
        }
        assert!(MeshData::primitive(MeshKind(MeshKind::BUILTIN_COUNT)).is_none());
    }
}
