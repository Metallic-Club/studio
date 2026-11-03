//! The packed record the GPU reads for every renderable instance: 64 bytes (ADR 0008 §2).

use bytemuck::{Pod, Zeroable};
use metallic_core::glam::{Mat3, Vec3};
use metallic_core::{CFrame, Color3};

/// One renderable instance as the GPU reads it, 64 bytes, in a slot of a scene region's instance buffer.
///
/// | Bytes | Field | Contents |
/// |---|---|---|
/// | 0..48 | `rows` | A 3×4 affine transform, row-major: rotation × mesh scale in `xyz`, translation in `w`. The translation is relative to the scene's origin ([`super::InstanceScene::origin`]), not to the world origin |
/// | 48..52 | `color` | sRGB-encoded 8-bit red, green, blue in bytes 0, 1, 2 (`unpack4x8unorm` in WGSL); byte 3 is reserved (0) |
/// | 52..56 | `transparency` | `Transparency`, 0 (solid) to 1 (invisible) |
/// | 56..60 | `mesh_material` | Material row (low 16 bits) and mesh kind (high 16 bits) |
/// | 60..64 | `flags` | [`GpuInstance::LIVE`] and the other flags in the low 8 bits; the rest is reserved (0) |
///
/// A free slot is all zeros, so its `flags` lack [`GpuInstance::LIVE`] and anything that walks the buffer (a
/// GPU culling pass) skips it.
///
/// The colour stays sRGB-encoded: the shader decodes it to linear light, which keeps the precision an 8-bit sRGB
/// texture has. Storing linear 8-bit values instead would band in dark colours.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuInstance {
    /// Rotation × scale in `xyz` and translation (relative to the scene origin) in `w`, one row per world axis.
    pub rows: [[f32; 4]; 3],
    /// sRGB-encoded colour, red in the lowest byte.
    pub color: u32,
    /// How see-through the instance is, 0 to 1.
    pub transparency: f32,
    /// Material row in the low 16 bits, mesh kind in the high 16 bits.
    pub mesh_material: u32,
    /// Flags in the low 8 bits.
    pub flags: u32,
}

const _: () = assert!(size_of::<GpuInstance>() == 64);

impl GpuInstance {
    /// The size of one instance in bytes.
    pub const SIZE: u64 = size_of::<Self>() as u64;

    /// The slot holds an instance. Free slots have no flags at all.
    pub const LIVE: u32 = 1 << 0;
    /// The instance casts shadows (`CastShadow`).
    pub const CAST_SHADOW: u32 = 1 << 1;
    /// Partly see-through (`Transparency` above 0 and below 1): drawn in the transparent pass.
    pub const TRANSLUCENT: u32 = 1 << 2;
    /// Fully see-through (`Transparency` of 1): nothing to draw, though the slot stays live.
    pub const HIDDEN: u32 = 1 << 3;

    /// What a free slot holds.
    pub const FREE: Self = Self {
        rows: [[0.0; 4]; 3],
        color: 0,
        transparency: 0.0,
        mesh_material: 0,
        flags: 0,
    };

    /// The mesh kind.
    pub fn mesh(&self) -> MeshKind {
        MeshKind((self.mesh_material >> 16) as u16)
    }

    /// The row of the material table.
    pub fn material_row(&self) -> u16 {
        (self.mesh_material & 0xFFFF) as u16
    }

    /// The translation, relative to the scene origin.
    pub fn translation(&self) -> Vec3 {
        Vec3::new(self.rows[0][3], self.rows[1][3], self.rows[2][3])
    }

    /// The vertex attributes of the vertex-buffer fallback (GLES, where vertex shaders cannot read storage
    /// buffers), starting at `first_location`: the three rows as `vec4<f32>`, then `color`, `transparency`,
    /// `mesh_material` and `flags`. The buffer's stride is [`GpuInstance::SIZE`], stepped per instance.
    pub const fn vertex_attributes(first_location: u32) -> [wgpu::VertexAttribute; 7] {
        let l = first_location;
        [
            attribute(wgpu::VertexFormat::Float32x4, 0, l),
            attribute(wgpu::VertexFormat::Float32x4, 16, l + 1),
            attribute(wgpu::VertexFormat::Float32x4, 32, l + 2),
            attribute(wgpu::VertexFormat::Uint32, 48, l + 3),
            attribute(wgpu::VertexFormat::Float32, 52, l + 4),
            attribute(wgpu::VertexFormat::Uint32, 56, l + 5),
            attribute(wgpu::VertexFormat::Uint32, 60, l + 6),
        ]
    }
}

const fn attribute(format: wgpu::VertexFormat, offset: u64, shader_location: u32) -> wgpu::VertexAttribute {
    wgpu::VertexAttribute {
        format,
        offset,
        shader_location,
    }
}

/// Which mesh an instance is drawn with: one of the built-in primitives, or (later) a mesh asset.
///
/// Draws are grouped by mesh kind (ADR 0008 §4), so the number of kinds, not of parts, sets the draw count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshKind(pub u16);

impl MeshKind {
    /// A unit cube, stretched to the part's size.
    pub const BLOCK: Self = Self(0);
    /// A unit sphere; its diameter is the part's smallest side.
    pub const BALL: Self = Self(1);
    /// A unit cylinder along X; its length is the part's X size and its diameter the smaller of Y and Z.
    pub const CYLINDER: Self = Self(2);
    /// A unit ramp that slopes down towards -Z, stretched to the part's size.
    pub const WEDGE: Self = Self(3);
    /// How many built-in kinds there are; mesh assets get kinds from here up.
    pub const BUILTIN_COUNT: u16 = 4;
}

/// The 3×4 affine of `cframe` with the unit mesh stretched by `scale`, translated so `origin` is at zero.
#[inline]
pub fn affine_rows(cframe: CFrame, scale: Vec3, origin: Vec3) -> [[f32; 4]; 3] {
    let m = Mat3::from_quat(cframe.rotation());
    let (x, y, z) = (m.x_axis * scale.x, m.y_axis * scale.y, m.z_axis * scale.z);
    let t = cframe.position() - origin;
    [[x.x, y.x, z.x, t.x], [x.y, y.y, z.y, t.y], [x.z, y.z, z.z, t.z]]
}

/// `color` as four 8-bit sRGB channels, red in the lowest byte, the fourth byte 0.
#[inline]
pub fn pack_color(color: Color3) -> u32 {
    let [r, g, b] = color.to_rgb_u8();
    u32::from_le_bytes([r, g, b, 0])
}

/// The flag bits `Transparency` sets.
#[inline]
pub(crate) fn transparency_flags(transparency: f32) -> u32 {
    if transparency >= 1.0 {
        GpuInstance::HIDDEN
    } else if transparency > 0.0 {
        GpuInstance::TRANSLUCENT
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use metallic_core::glam::Quat;

    use super::*;

    #[test]
    fn the_layout_is_64_bytes_and_matches_the_attributes() {
        assert_eq!(GpuInstance::SIZE, 64);
        let attributes = GpuInstance::vertex_attributes(3);
        let end = attributes.iter().map(|a| a.offset + a.format.size()).max();
        assert_eq!(end, Some(GpuInstance::SIZE));
        assert_eq!(attributes[0].shader_location, 3);
        assert_eq!(attributes[6].shader_location, 9);
        assert_eq!(bytemuck::bytes_of(&GpuInstance::FREE), &[0u8; 64]);
    }

    #[test]
    fn rows_hold_rotation_times_scale_and_relative_translation() {
        let turn = CFrame::from_parts(
            Vec3::new(10.0, 20.0, 30.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        );
        let rows = affine_rows(turn, Vec3::new(2.0, 3.0, 4.0), Vec3::new(8.0, 0.0, 0.0));
        let apply = |p: Vec3| {
            Vec3::new(
                rows[0][0] * p.x + rows[0][1] * p.y + rows[0][2] * p.z + rows[0][3],
                rows[1][0] * p.x + rows[1][1] * p.y + rows[1][2] * p.z + rows[1][3],
                rows[2][0] * p.x + rows[2][1] * p.y + rows[2][2] * p.z + rows[2][3],
            )
        };
        // The unit mesh's +X corner, stretched to 2, turned a quarter about Y (to -Z), moved, minus the origin.
        let p = apply(Vec3::new(0.5, 0.0, 0.0));
        assert!(p.abs_diff_eq(Vec3::new(2.0, 20.0, 29.0), 1e-5), "{p}");
        let q = apply(Vec3::new(0.0, 0.5, 0.0));
        assert!(q.abs_diff_eq(Vec3::new(2.0, 21.5, 30.0), 1e-5), "{q}");
    }

    #[test]
    fn colour_and_kind_pack_into_their_bits() {
        assert_eq!(pack_color(Color3::from_rgb_u8(1, 2, 3)), 0x0003_0201);
        let instance = GpuInstance {
            mesh_material: (u32::from(MeshKind::WEDGE.0) << 16) | 7,
            ..GpuInstance::default()
        };
        assert_eq!(instance.mesh(), MeshKind::WEDGE);
        assert_eq!(instance.material_row(), 7);
        assert_eq!(transparency_flags(0.0), 0);
        assert_eq!(transparency_flags(0.5), GpuInstance::TRANSLUCENT);
        assert_eq!(transparency_flags(1.0), GpuInstance::HIDDEN);
    }
}
