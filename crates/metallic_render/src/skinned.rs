//! Skinned meshes: GPU data for [`metallic_asset::SkinnedModel`]s, per-instance joint buffers and
//! level-of-detail selection.
//!
//! Skinning is linear blend in the vertex shader (`shaders/skinned.wgsl`): each vertex is moved
//! by the weighted sum of up to four joint matrices read from a storage buffer that belongs to
//! the instance. The skinned pass writes the same outputs as the part pass, so characters get the
//! same lighting, shadows, fog and procedural material detail as parts.
use crate::camera::Camera;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use metallic_asset::{MAX_MATERIALS, SkinnedModel};
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Below this projected height in pixels an instance draws LOD1 instead of LOD0.
pub const LOD1_BELOW_PX: f32 = 220.0;
/// Below this projected height in pixels an instance draws LOD2.
pub const LOD2_BELOW_PX: f32 = 80.0;

/// The level of detail for an instance that covers `height_px` pixels on screen, given the
/// model's `lod_count` levels (LOD0 is the most detailed).
pub fn select_lod(height_px: f32, lod_count: usize) -> usize {
    let wanted = if height_px >= LOD1_BELOW_PX {
        0
    } else if height_px >= LOD2_BELOW_PX {
        1
    } else {
        2
    };
    wanted.min(lod_count.saturating_sub(1))
}

/// Height in pixels of an upright object `height` studs tall whose middle is at `center`, seen by
/// `camera` in a viewport `viewport_height` pixels tall.
pub fn projected_height(camera: &Camera, viewport_height: u32, center: Vec3, height: f32) -> f32 {
    let distance = camera.position.distance(center).max(1e-3);
    let focal = viewport_height as f32 / (2.0 * (camera.fov_deg.to_radians() * 0.5).tan());
    height * focal / distance
}

/// What to draw for one skinned instance this frame (see `Renderer::set_skinned`).
#[derive(Debug, Clone, Copy)]
pub struct SkinnedInstance<'a> {
    /// The model. It is uploaded to the GPU the first time it is drawn and kept, keyed by the
    /// `Arc`, for as long as the renderer lives.
    pub model: &'a Arc<SkinnedModel>,
    /// One skinning matrix per joint, model transform included (see
    /// `metallic_asset::Pose::skin_matrices`): a bind-pose vertex times its weighted joint
    /// matrices is its world position in studs.
    pub joints: &'a [Mat4],
    /// Linear RGB colour of each material slot; slots past the end use the model's own colour.
    pub slot_colors: &'a [[f32; 3]],
    /// World position of the middle of the figure (for level-of-detail selection).
    pub center: Vec3,
    /// World height of the figure in studs (for level-of-detail selection).
    pub height: f32,
    /// Studs per model unit; scales the procedural surface detail so it matches parts.
    pub detail_scale: f32,
}

/// One vertex as the skinned pipeline reads it (52 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct GpuSkinnedVertex {
    position: [f32; 3],
    normal: [f32; 3],
    joints: [u16; 4],
    weights: [f32; 4],
    slot: u32,
}

pub(crate) const SKINNED_ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x3, 2 => Uint16x4, 3 => Float32x4, 4 => Uint32
];

pub(crate) fn skinned_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuSkinnedVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &SKINNED_ATTRS,
    }
}

/// Header of the per-instance storage buffer; mirrors `Skin` in `skin_common.wgsl` (the joint
/// matrices follow it).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkinHeader {
    /// x: studs per model unit (procedural detail scale).
    params: [f32; 4],
    /// Linear RGB and alpha per material slot.
    slot_color: [[f32; 4]; MAX_MATERIALS],
    /// Roughness, metalness, material id, unused.
    slot_material: [[f32; 4]; MAX_MATERIALS],
}

/// Shader material id used for the character's surface detail (`SmoothPlastic`: nearly flat, so
/// the planes of the mannequin read cleanly).
const SKIN_DETAIL_MATERIAL: f32 = 1.0;

pub(crate) struct GpuLod {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub index_count: u32,
}

/// A model on the GPU.
pub(crate) struct GpuSkinnedModel {
    pub model: Arc<SkinnedModel>,
    pub lods: Vec<GpuLod>,
}

impl GpuSkinnedModel {
    pub fn upload(device: &wgpu::Device, model: &Arc<SkinnedModel>) -> GpuSkinnedModel {
        let lods = model
            .lods
            .iter()
            .map(|lod| {
                let vertices: Vec<GpuSkinnedVertex> = lod
                    .vertices
                    .iter()
                    .map(|v| GpuSkinnedVertex {
                        position: v.position,
                        normal: v.normal,
                        joints: v.joints,
                        weights: v.weights,
                        slot: u32::from(v.material),
                    })
                    .collect();
                GpuLod {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("metallic skinned vertices"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("metallic skinned indices"),
                        contents: bytemuck::cast_slice(&lod.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    index_count: lod.indices.len() as u32,
                }
            })
            .collect();
        GpuSkinnedModel {
            model: model.clone(),
            lods,
        }
    }
}

/// The storage buffer and bind group of one instance slot; grows to fit more joints.
pub(crate) struct SkinSlot {
    pub buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    joints: usize,
}

impl SkinSlot {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, joints: usize) -> SkinSlot {
        let size = std::mem::size_of::<SkinHeader>() + joints.max(1) * 64;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic skin instance"),
            size: size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("metallic skin instance group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        SkinSlot {
            buffer,
            bind_group,
            joints: joints.max(1),
        }
    }

    /// Writes one instance into slot `index` of `slots`, creating or growing it first.
    pub fn write(
        slots: &mut Vec<SkinSlot>,
        index: usize,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        inst: &SkinnedInstance<'_>,
        scratch: &mut Vec<u8>,
    ) {
        let joints = inst.joints.len();
        if index >= slots.len() {
            slots.push(SkinSlot::new(device, layout, joints.next_power_of_two()));
        } else if slots[index].joints < joints {
            slots[index] = SkinSlot::new(device, layout, joints.next_power_of_two());
        }
        let mut header = SkinHeader::zeroed();
        header.params[0] = inst.detail_scale;
        for (i, slot) in inst.model.materials.iter().enumerate().take(MAX_MATERIALS) {
            let rgb = inst.slot_colors.get(i).copied().unwrap_or([
                slot.base_color[0],
                slot.base_color[1],
                slot.base_color[2],
            ]);
            header.slot_color[i] = [rgb[0], rgb[1], rgb[2], 1.0];
            header.slot_material[i] = [slot.roughness, slot.metallic, SKIN_DETAIL_MATERIAL, 0.0];
        }
        scratch.clear();
        scratch.extend_from_slice(bytemuck::bytes_of(&header));
        for m in inst.joints {
            scratch.extend_from_slice(bytemuck::bytes_of(&m.to_cols_array()));
        }
        queue.write_buffer(&slots[index].buffer, 0, scratch);
    }
}

/// One skinned draw prepared by `Renderer::set_skinned`.
pub(crate) struct SkinnedDraw {
    pub model: usize,
    pub slot: usize,
    pub center: Vec3,
    pub height: f32,
    /// Level of detail picked for the current view.
    pub lod: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_follows_screen_height() {
        assert_eq!(select_lod(800.0, 3), 0);
        assert_eq!(select_lod(LOD1_BELOW_PX, 3), 0);
        assert_eq!(select_lod(LOD1_BELOW_PX - 1.0, 3), 1);
        assert_eq!(select_lod(LOD2_BELOW_PX - 1.0, 3), 2);
        assert_eq!(select_lod(5.0, 2), 1, "clamped to the levels the model has");
        assert_eq!(select_lod(5.0, 1), 0);
    }

    #[test]
    fn projected_height_halves_with_twice_the_distance() {
        let cam = |z: f32| Camera {
            position: Vec3::new(0.0, 2.75, z),
            look_at: Vec3::new(0.0, 2.75, 0.0),
            fov_deg: 70.0,
        };
        let near = projected_height(&cam(12.0), 1080, Vec3::new(0.0, 2.75, 0.0), 5.5);
        let far = projected_height(&cam(24.0), 1080, Vec3::new(0.0, 2.75, 0.0), 5.5);
        assert!((near / far - 2.0).abs() < 1e-4);
        // The default third-person view at 1080p draws the full model.
        assert_eq!(select_lod(near, 3), 0, "{near} px");
    }

    #[test]
    fn header_matches_the_shader_layout() {
        // params + 8 colours + 8 material rows, 16 bytes each.
        assert_eq!(
            std::mem::size_of::<SkinHeader>(),
            16 * (1 + 2 * MAX_MATERIALS)
        );
        assert_eq!(std::mem::size_of::<GpuSkinnedVertex>(), 52);
    }
}
