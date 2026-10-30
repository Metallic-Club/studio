//! The generated mesh and its GPU vertex layout.
use std::fmt::Write as _;

/// Axis-aligned bounding box in studs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aabb {
    /// Smallest corner.
    pub min: [f32; 3],
    /// Largest corner.
    pub max: [f32; 3],
}

impl Aabb {
    /// `max - min` per axis.
    pub fn size(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| self.max[i] - self.min[i])
    }
}

/// A triangle mesh with separate attribute arrays (one entry per vertex) and a `u32` index list.
/// See the crate docs for the meaning and units of each attribute.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    /// Positions in studs, centred on the origin.
    pub positions: Vec<[f32; 3]>,
    /// Unit outward normals.
    pub normals: Vec<[f32; 3]>,
    /// MikkTSpace tangents: unit `xyz`, `w` = ±1 (bitangent = `w * cross(normal, tangent)`).
    pub tangents: Vec<[f32; 4]>,
    /// World-scale UVs: 1.0 per stud.
    pub uv0: Vec<[f32; 2]>,
    /// 0..1 UVs per chart (face or curved side).
    pub uv1: Vec<[f32; 2]>,
    /// Baked ambient occlusion, 0..=1 (1 = fully open).
    pub ao: Vec<f32>,
    /// Triangle list, counter-clockwise seen from outside.
    pub indices: Vec<u32>,
    /// Bounds of `positions`.
    pub bounds: Aabb,
}

/// One interleaved vertex, 60 bytes, `#[repr(C)]`:
///
/// | field      | offset | format      | wgpu format         |
/// |------------|--------|-------------|---------------------|
/// | `position` | 0      | `[f32; 3]`  | `Float32x3`         |
/// | `normal`   | 12     | `[f32; 3]`  | `Float32x3`         |
/// | `tangent`  | 24     | `[f32; 4]`  | `Float32x4`         |
/// | `uv0`      | 40     | `[f32; 2]`  | `Float32x2`         |
/// | `uv1`      | 48     | `[f32; 2]`  | `Float32x2`         |
/// | `ao`       | 56     | `f32`       | `Float32`           |
///
/// With the `bytemuck` feature it derives `Pod` and `Zeroable`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct Vertex {
    /// Position in studs.
    pub position: [f32; 3],
    /// Unit normal.
    pub normal: [f32; 3],
    /// Tangent with bitangent sign in `w`.
    pub tangent: [f32; 4],
    /// World-scale UV (1.0 per stud).
    pub uv0: [f32; 2],
    /// 0..1 UV per chart.
    pub uv1: [f32; 2],
    /// Ambient occlusion.
    pub ao: f32,
}

impl Vertex {
    /// Size of one vertex in bytes.
    pub const SIZE: usize = std::mem::size_of::<Vertex>();
}

impl MeshData {
    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// The vertices packed into one interleaved array (same order as the attribute arrays).
    pub fn interleaved(&self) -> Vec<Vertex> {
        (0..self.positions.len())
            .map(|i| Vertex {
                position: self.positions[i],
                normal: self.normals[i],
                tangent: self.tangents[i],
                uv0: self.uv0[i],
                uv1: self.uv1[i],
                ao: self.ao[i],
            })
            .collect()
    }

    /// The mesh as Wavefront OBJ text (positions, `uv0`, normals; one object named `name`).
    /// Handy for looking at shapes in Blender or any viewer.
    pub fn to_obj(&self, name: &str) -> String {
        let mut s = String::with_capacity(self.positions.len() * 96 + self.indices.len() * 12);
        let _ = writeln!(s, "# metallic_meshgen {}", crate::MESHGEN_VERSION);
        let _ = writeln!(s, "o {name}");
        for p in &self.positions {
            let _ = writeln!(s, "v {} {} {}", p[0], p[1], p[2]);
        }
        for t in &self.uv0 {
            let _ = writeln!(s, "vt {} {}", t[0], t[1]);
        }
        for n in &self.normals {
            let _ = writeln!(s, "vn {} {} {}", n[0], n[1], n[2]);
        }
        for t in self.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0] + 1, t[1] + 1, t[2] + 1];
            let _ = writeln!(s, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_is_60_bytes() {
        assert_eq!(Vertex::SIZE, 60);
        assert_eq!(std::mem::offset_of!(Vertex, tangent), 24);
        assert_eq!(std::mem::offset_of!(Vertex, uv0), 40);
        assert_eq!(std::mem::offset_of!(Vertex, uv1), 48);
        assert_eq!(std::mem::offset_of!(Vertex, ao), 56);
    }
}
