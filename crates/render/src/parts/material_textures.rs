//! Materials' textures (#679, ADR 0012): two texture arrays every part draw binds, beside its base-colour texture
//! (the texture group, [`super::assets`]).
//!
//! - **Colour** (sRGB): a material's base-colour and emission maps.
//! - **Data** (linear): its normal map, and one layer holding ambient occlusion, roughness and metallic in red, green
//!   and blue (packed on load from separate maps or a packed one).
//!
//! Every layer is [`MATERIAL_TEXTURE_SIZE`] square with a full mip chain, made on load (off the frame, by the engine's
//! asset worker). A material uses at most four layers, so it costs at most four samples, and only a material that
//! has textures samples any ([`crate::MaterialTable`]'s textured flag). The arrays start small and double when full:
//! the new arrays are made, the old layers copied in, and every texture group made again.

use std::collections::HashMap;
use std::sync::Arc;

use crate::context::in_heap_turn;

/// The side of every material texture layer, in texels.
pub const MATERIAL_TEXTURE_SIZE: u32 = 512;

/// A layer number that names no layer.
pub const NO_LAYER: u16 = u16::MAX;

/// How many layers each array has at first.
const FIRST_LAYERS: u32 = 4;

/// The most layers an array grows to (the device may allow fewer).
const MOST_LAYERS: u32 = 1024;

/// One square RGBA8 picture with its mip chain, largest first: what a layer holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialImage {
    /// Each level's texels, 4 bytes each, row by row; level `i` is `MATERIAL_TEXTURE_SIZE >> i` square.
    pub mips: Vec<Vec<u8>>,
}

/// A material's textures, as its layers hold them. A channel without one uses the material's constant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MaterialTextureSet {
    /// The base colour (sRGB).
    pub color: Option<MaterialImage>,
    /// The tangent-space normal map (linear).
    pub normal: Option<MaterialImage>,
    /// Ambient occlusion, roughness and metallic in red, green and blue (linear).
    pub orm: Option<MaterialImage>,
    /// The emitted light (sRGB).
    pub emission: Option<MaterialImage>,
}

impl MaterialTextureSet {
    /// Whether it holds no texture.
    pub fn is_empty(&self) -> bool {
        self.color.is_none() && self.normal.is_none() && self.orm.is_none() && self.emission.is_none()
    }
}

/// Where a material's textures are: a layer per channel, [`NO_LAYER`] for none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialLayers {
    /// The base colour's layer in the colour array.
    pub color: u16,
    /// The normal map's layer in the data array.
    pub normal: u16,
    /// The occlusion-roughness-metallic layer in the data array.
    pub orm: u16,
    /// The emission's layer in the colour array.
    pub emission: u16,
}

impl MaterialLayers {
    /// No textures.
    pub const NONE: Self = Self {
        color: NO_LAYER,
        normal: NO_LAYER,
        orm: NO_LAYER,
        emission: NO_LAYER,
    };

    /// Whether any channel has a layer.
    pub fn any(self) -> bool {
        self != Self::NONE
    }
}

impl Default for MaterialLayers {
    fn default() -> Self {
        Self::NONE
    }
}

/// The number of mip levels of a layer.
pub(crate) fn mip_count() -> u32 {
    MATERIAL_TEXTURE_SIZE.ilog2() + 1
}

/// One texture array and which of its layers are taken.
#[derive(Debug)]
struct LayerArray {
    label: &'static str,
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    taken: Vec<bool>,
}

impl LayerArray {
    fn new(device: &wgpu::Device, label: &'static str, format: wgpu::TextureFormat, layers: u32) -> Self {
        let texture = make(device, label, format, layers);
        let view = array_view(&texture);
        Self {
            label,
            format,
            texture,
            view,
            taken: vec![false; layers as usize],
        }
    }

    /// A free layer, growing the array when it is full (copying what it holds). `None` past the device's limit.
    /// Returns whether the array was made again.
    fn take(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, most: u32) -> Option<(u16, bool)> {
        if let Some(free) = self.taken.iter().position(|taken| !taken) {
            self.taken[free] = true;
            return Some((u16::try_from(free).ok()?, false));
        }
        let old = self.taken.len() as u32;
        let new = (old * 2).min(most);
        if new <= old {
            return None;
        }
        let texture = make(device, self.label, self.format, new);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic material textures grow"),
        });
        for level in 0..mip_count() {
            let side = (MATERIAL_TEXTURE_SIZE >> level).max(1);
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: old,
                },
            );
        }
        queue.submit([encoder.finish()]);
        self.view = array_view(&texture);
        self.texture = texture;
        self.taken.resize(new as usize, false);
        self.taken[old as usize] = true;
        Some((u16::try_from(old).ok()?, true))
    }

    fn release(&mut self, layer: u16) {
        if let Some(taken) = self.taken.get_mut(usize::from(layer)) {
            *taken = false;
        }
    }

    fn write(&self, queue: &wgpu::Queue, layer: u16, image: &MaterialImage) {
        for (level, texels) in image.mips.iter().enumerate().take(mip_count() as usize) {
            let side = (MATERIAL_TEXTURE_SIZE >> level).max(1);
            if texels.len() != (side * side * 4) as usize {
                continue;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: u32::from(layer),
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * side),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

fn make(device: &wgpu::Device, label: &'static str, format: wgpu::TextureFormat, layers: u32) -> wgpu::Texture {
    in_heap_turn(|| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: MATERIAL_TEXTURE_SIZE,
                height: MATERIAL_TEXTURE_SIZE,
                depth_or_array_layers: layers,
            },
            mip_level_count: mip_count(),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    })
}

fn array_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..wgpu::TextureViewDescriptor::default()
    })
}

/// The materials' texture arrays, their samplers, and which layers each material has.
#[derive(Debug)]
pub(crate) struct MaterialTextures {
    color: LayerArray,
    data: LayerArray,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    by_key: HashMap<Arc<str>, MaterialLayers>,
    most: u32,
}

impl MaterialTextures {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let sampler = |label, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: match filter {
                    wgpu::FilterMode::Linear => wgpu::MipmapFilterMode::Linear,
                    wgpu::FilterMode::Nearest => wgpu::MipmapFilterMode::Nearest,
                },
                ..wgpu::SamplerDescriptor::default()
            })
        };
        Self {
            color: LayerArray::new(
                device,
                "metallic material colour textures",
                wgpu::TextureFormat::Rgba8UnormSrgb,
                FIRST_LAYERS,
            ),
            data: LayerArray::new(
                device,
                "metallic material data textures",
                wgpu::TextureFormat::Rgba8Unorm,
                FIRST_LAYERS,
            ),
            linear: sampler("metallic material textures", wgpu::FilterMode::Linear),
            nearest: sampler("metallic material textures (nearest)", wgpu::FilterMode::Nearest),
            by_key: HashMap::new(),
            most: device.limits().max_texture_array_layers.min(MOST_LAYERS),
        }
    }

    /// The colour array's view, the data array's, and the linear and nearest samplers, for a texture group.
    pub(crate) fn bindings(&self) -> (&wgpu::TextureView, &wgpu::TextureView, &wgpu::Sampler, &wgpu::Sampler) {
        (&self.color.view, &self.data.view, &self.linear, &self.nearest)
    }

    /// Puts `set` in layers for the material `key` (its old layers freed first) and returns them, with whether an
    /// array was made again (so every texture group must be). A channel that finds no free layer, past the device's
    /// limit, goes without.
    pub(crate) fn add(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: &str,
        set: &MaterialTextureSet,
    ) -> (MaterialLayers, bool) {
        self.remove(key);
        let most = self.most;
        let mut grew = false;
        let mut put = |array: &mut LayerArray, image: &Option<MaterialImage>| -> u16 {
            let Some(image) = image else {
                return NO_LAYER;
            };
            match array.take(device, queue, most) {
                Some((layer, made)) => {
                    array.write(queue, layer, image);
                    grew |= made;
                    layer
                }
                None => NO_LAYER,
            }
        };
        let layers = MaterialLayers {
            color: put(&mut self.color, &set.color),
            emission: put(&mut self.color, &set.emission),
            normal: put(&mut self.data, &set.normal),
            orm: put(&mut self.data, &set.orm),
        };
        self.by_key.insert(Arc::from(key), layers);
        (layers, grew)
    }

    /// Frees the layers of the material `key`.
    pub(crate) fn remove(&mut self, key: &str) {
        if let Some(old) = self.by_key.remove(key) {
            for layer in [old.color, old.emission] {
                if layer != NO_LAYER {
                    self.color.release(layer);
                }
            }
            for layer in [old.normal, old.orm] {
                if layer != NO_LAYER {
                    self.data.release(layer);
                }
            }
        }
    }
}

/// `rgba` (`width` × `height`, 4 bytes a texel) fitted to a layer, with its mip chain: resized to
/// [`MATERIAL_TEXTURE_SIZE`] square by bilinear filtering, then halved by 2 × 2 boxes. Colour (sRGB) pictures should be
/// averaged in linear light; the box filter on stored values is close enough for mips and costs nothing extra.
pub fn material_image(width: u32, height: u32, rgba: &[u8]) -> Option<MaterialImage> {
    if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
        return None;
    }
    let side = MATERIAL_TEXTURE_SIZE;
    let mut base = vec![0u8; (side * side * 4) as usize];
    let at = |x: u32, y: u32, c: usize| f32::from(rgba[((y * width + x) * 4) as usize + c]);
    for y in 0..side {
        let fy = ((y as f32 + 0.5) * height as f32 / side as f32 - 0.5).max(0.0);
        let y0 = (fy as u32).min(height - 1);
        let y1 = (y0 + 1).min(height - 1);
        let ty = fy - y0 as f32;
        for x in 0..side {
            let fx = ((x as f32 + 0.5) * width as f32 / side as f32 - 0.5).max(0.0);
            let x0 = (fx as u32).min(width - 1);
            let x1 = (x0 + 1).min(width - 1);
            let tx = fx - x0 as f32;
            for c in 0..4 {
                let top = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
                let bottom = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
                base[((y * side + x) * 4) as usize + c] = (top * (1.0 - ty) + bottom * ty).round() as u8;
            }
        }
    }
    let mut mips = vec![base];
    let mut level_side = side;
    while level_side > 1 {
        let half = level_side / 2;
        let above = mips.last()?;
        let mut next = vec![0u8; (half * half * 4) as usize];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let texel = |dx: u32, dy: u32| {
                        u32::from(above[(((2 * y + dy) * level_side + 2 * x + dx) * 4) as usize + c])
                    };
                    next[((y * half + x) * 4) as usize + c] =
                        ((texel(0, 0) + texel(1, 0) + texel(0, 1) + texel(1, 1) + 2) / 4) as u8;
                }
            }
        }
        mips.push(next);
        level_side = half;
    }
    Some(MaterialImage { mips })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_fitted_to_a_layer_with_every_mip() {
        let image = material_image(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        assert_eq!(image.mips.len() as u32, mip_count());
        assert_eq!(
            image.mips[0].len() as u32,
            MATERIAL_TEXTURE_SIZE * MATERIAL_TEXTURE_SIZE * 4
        );
        assert_eq!(image.mips.last().unwrap().len(), 4);
        // The left edge is red, the right blue; the last mip is their average.
        assert_eq!(&image.mips[0][..4], &[255, 0, 0, 255]);
        let last = image.mips.last().unwrap();
        assert!(
            (120..=135).contains(&last[0]) && (120..=135).contains(&last[2]),
            "{last:?}"
        );
        assert_eq!(material_image(0, 4, &[]), None);
        assert_eq!(material_image(2, 2, &[0; 4]), None, "too few texels");
    }

    #[test]
    fn layers_name_none_until_given() {
        assert!(!MaterialLayers::NONE.any());
        assert!(
            MaterialLayers {
                orm: 3,
                ..MaterialLayers::NONE
            }
            .any()
        );
        assert!(MaterialTextureSet::default().is_empty());
    }
}
