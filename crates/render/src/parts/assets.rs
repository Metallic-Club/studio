//! Mesh assets' base-colour textures (#189): one bind group per texture, bound per draw.
//!
//! Every part pipeline samples a base-colour texture (`base_texture` in the instance files). A mesh asset's draw binds
//! its own texture; every other draw (the primitives, the transparent pass, an asset without a texture or whose
//! texture is not here yet) binds a 1 × 1 white one, sampled nearest so it multiplies the colour by exactly 1. The
//! group is the storage path's group 2 and the vertex-buffer path's group 1 ([`texture_group`]).
//!
//! Textures are 8-bit sRGB RGBA, one level (no mips yet), sampled linearly and repeating, as glTF's default sampler
//! does. A large one is made off the frame like the shadow maps (#152, [`crate::textures`]): its texture arrives in a
//! later frame, which clears it, so its pixels are written in the frame after that; the draws use white until then.

use std::collections::HashMap;
use std::sync::Arc;

use crate::context::in_heap_turn;
use crate::scene::InstanceBinding;
use crate::textures::{Slot, TextureMaker, TextureSpec};

use super::material_textures::{MaterialLayers, MaterialTextureSet, MaterialTextures};

/// The bind group index of the base-colour texture on the instance path `binding`.
pub(crate) const fn texture_group(binding: InstanceBinding) -> u32 {
    match binding {
        InstanceBinding::Storage => 2,
        InstanceBinding::Vertex => 1,
    }
}

/// The index of the white texture's group in [`AssetTextures::groups`].
pub(crate) const WHITE: u32 = 0;

/// A texture asked for that is not drawn yet: being made, or made and cleared this frame.
#[derive(Debug)]
struct Pending {
    key: Arc<str>,
    slot: Slot,
    /// The frame's clear of a texture made off the frame comes after this frame's writes, so it is written in the
    /// next one.
    arrived: bool,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// The textures of the mesh assets, by key, and the bind group of each.
#[derive(Debug)]
pub(crate) struct AssetTextures {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The white texture's sampler (nearest).
    nearest: wgpu::Sampler,
    /// The white texture's group first ([`WHITE`]), then one per texture.
    groups: Vec<wgpu::BindGroup>,
    /// Each group's base-colour texture, to make the group again when the materials' arrays grow.
    views: Vec<wgpu::TextureView>,
    by_key: HashMap<Arc<str>, u32>,
    pending: Vec<Pending>,
    /// The materials' textures (#679), in every group.
    materials: MaterialTextures,
    /// Changes whenever a texture's group appears.
    revision: u64,
}

impl AssetTextures {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let array = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = in_heap_turn(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("metallic parts texture"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    sampler_entry(1),
                    // The materials' colour and data arrays and their sampler (#679).
                    array(2),
                    array(3),
                    sampler_entry(4),
                ],
            })
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("metallic parts texture"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let nearest = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("metallic parts white texture"),
            ..wgpu::SamplerDescriptor::default()
        });
        let white = TextureSpec::new(
            "metallic parts white texture",
            (1, 1, 1),
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        )
        .make_now(device);
        write(queue, &white, 1, 1, &[255; 4]);
        let materials = MaterialTextures::new(device);
        let white = white.create_view(&wgpu::TextureViewDescriptor::default());
        let white_group = group(device, &layout, &white, &nearest, &materials);
        Self {
            layout,
            sampler,
            nearest,
            groups: vec![white_group],
            views: vec![white],
            by_key: HashMap::new(),
            pending: Vec::new(),
            materials,
            revision: 0,
        }
    }

    /// Puts the material `key`'s textures in layers (#679) and returns them. When an array grew, every group is made
    /// again.
    pub(crate) fn add_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: &str,
        set: &MaterialTextureSet,
    ) -> MaterialLayers {
        let (layers, grew) = self.materials.add(device, queue, key, set);
        if grew {
            self.groups = self
                .views
                .iter()
                .enumerate()
                .map(|(index, view)| {
                    let sampler = if index == WHITE as usize {
                        &self.nearest
                    } else {
                        &self.sampler
                    };
                    group(device, &self.layout, view, sampler, &self.materials)
                })
                .collect();
            self.revision = self.revision.wrapping_add(1);
        }
        layers
    }

    /// Frees the material `key`'s layers.
    pub(crate) fn remove_material(&mut self, key: &str) {
        self.materials.remove(key);
    }

    /// The layout of a texture's group.
    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// The bind group at `index` ([`AssetTextures::index`]), white when there is none.
    pub(crate) fn group(&self, index: u32) -> &wgpu::BindGroup {
        self.groups.get(index as usize).unwrap_or(&self.groups[WHITE as usize])
    }

    /// The index of the group of texture `key`, [`WHITE`] until it is drawable.
    pub(crate) fn index(&self, key: &str) -> u32 {
        self.by_key.get(key).copied().unwrap_or(WHITE)
    }

    /// Whether texture `key` was added (drawable or not yet).
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.by_key.contains_key(key) || self.pending.iter().any(|p| &*p.key == key)
    }

    /// Changes whenever a texture becomes drawable.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Adds texture `key`, `width` × `height` sRGB RGBA pixels, made through `maker` (off the frame when it is large).
    /// The caller checked the size against the device's limit.
    pub(crate) fn add(
        &mut self,
        (device, queue, maker): (&wgpu::Device, &wgpu::Queue, &TextureMaker),
        key: &str,
        (width, height): (u32, u32),
        rgba: Vec<u8>,
    ) {
        let spec = TextureSpec::new(
            "metallic mesh asset texture",
            (width, height, 1),
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        self.pending.retain(|p| &*p.key != key);
        self.pending.push(Pending {
            key: Arc::from(key),
            slot: Slot::new(maker.make(spec)),
            arrived: false,
            width,
            height,
            rgba,
        });
        self.poll(device, queue, maker);
    }

    /// Writes the textures that are here and makes their groups. Allocates nothing when none is pending.
    pub(crate) fn poll(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, maker: &TextureMaker) {
        if self.pending.is_empty() {
            return;
        }
        let mut i = 0;
        while i < self.pending.len() {
            let pending = &mut self.pending[i];
            let waiting = matches!(pending.slot, Slot::Waiting(_));
            if !pending.slot.poll(maker) {
                i += 1;
                continue;
            }
            if waiting && !pending.arrived {
                // Cleared by this frame's commands, after any write made now.
                pending.arrived = true;
                i += 1;
                continue;
            }
            let pending = self.pending.swap_remove(i);
            let Some(texture) = pending.slot.into_texture() else {
                continue;
            };
            write(queue, &texture, pending.width, pending.height, &pending.rgba);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let group = group(device, &self.layout, &view, &self.sampler, &self.materials);
            match self.by_key.get(&pending.key) {
                Some(&index) => {
                    self.groups[index as usize] = group;
                    self.views[index as usize] = view;
                }
                None => {
                    self.by_key.insert(pending.key, self.groups.len() as u32);
                    self.groups.push(group);
                    self.views.push(view);
                }
            }
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

/// Writes all of `texture` (`width` × `height`, 4 bytes a texel) from `rgba`.
fn write(queue: &wgpu::Queue, texture: &wgpu::Texture, width: u32, height: u32, rgba: &[u8]) {
    queue.write_texture(
        texture.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

fn group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    materials: &MaterialTextures,
) -> wgpu::BindGroup {
    let (color, data, linear) = materials.bindings();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("metallic parts texture"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(color),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(data),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(linear),
            },
        ],
    })
}
