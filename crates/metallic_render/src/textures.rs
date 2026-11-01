//! Textured Part materials (#146): albedo detail, normal and ORM maps in texture arrays,
//! projected triplanar in the part's own studs.
//!
//! A material's images live in one layer of three arrays (detail, normal, ORM) of one *size
//! class* ([`SIZE_CLASSES`]): 32 px for pixel sets (filtered nearest), 256 px for the built-in
//! placeholders and 1024 px for the CC0 asset library. Every array has a full mip chain. The
//! material table row of each material says which layer and class it uses
//! ([`crate::shading::TextureRef`]); a row without one keeps the shader's procedural detail, so
//! Plastic, SmoothPlastic and Glass look exactly as they did before textures.
//!
//! Images are stored *normalised*, so the Part's `Color` sets the hue as in Roblox:
//! - detail: linear albedo divided by the image's mean colour, times 0.5 (1.0 = the mean);
//! - normal: tangent-space x and y (OpenGL, +Y up), times 0.5 plus 0.5;
//! - ORM: R ambient occlusion, G roughness divided by its mean times 0.5 (the material table's
//!   roughness scales it), B metalness (multiplied by the table's).
//!
//! Sources, in order: the asset library cache from `tools/assets/fetch.py` (CC0, licences in
//! `LICENSES/ASSETS.md`), then [`MaterialImages::placeholders`], which are drawn by code in
//! this file (own work, licensed with the engine). The repository holds no texture files.
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::materials::MATERIAL_TABLE;
use crate::shading::TextureRef;

/// Texture sizes of the three size classes: pixel sets, placeholders, the asset library.
pub const SIZE_CLASSES: [u32; 3] = [32, 256, 1024];

/// Materials in the registry.
pub const MATERIAL_COUNT: usize = MATERIAL_TABLE.len();

/// Which textures parts use. A Look picks one (`docs/design/look-presets.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum TextureSet {
    /// No textures: the shader's procedural detail (the look before #146).
    Procedural,
    /// Photographic sets (asset library `primary`, or the placeholders).
    #[default]
    Realistic,
    /// Softer, posterised sets with weak normals (asset library `stylized`).
    Stylized,
    /// 32 px sets filtered nearest (asset library `pixel`).
    Pixel,
}

impl TextureSet {
    /// The asset library `role` of this set's textures.
    pub fn role(self) -> Option<&'static str> {
        match self {
            TextureSet::Procedural => None,
            TextureSet::Realistic => Some("primary"),
            TextureSet::Stylized => Some("stylized"),
            TextureSet::Pixel => Some("pixel"),
        }
    }
}

/// One material's images, normalised as described in the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialImage {
    /// Registry index of the material.
    pub material: usize,
    /// Width and height in pixels (one of [`SIZE_CLASSES`]).
    pub size: u32,
    /// RGBA8, linear: rgb = albedo / mean x 0.5.
    pub detail: Vec<u8>,
    /// RG8: tangent-space normal x, y.
    pub normal: Vec<u8>,
    /// RGBA8: ambient occlusion, roughness / mean x 0.5, metalness, 255.
    pub orm: Vec<u8>,
    /// Studs one repeat covers.
    pub tile_studs: f32,
    /// Filter nearest (pixel sets).
    pub nearest: bool,
    /// Normal map strength, 0..1.
    pub normal_strength: f32,
    /// Where the images came from (asset id and licence, or "placeholder").
    pub source: String,
}

/// The images of one texture set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialImages {
    /// At most one image per material.
    pub images: Vec<MaterialImage>,
}

impl MaterialImages {
    /// The built-in placeholders of `set` (empty for [`TextureSet::Procedural`]). Drawn once per
    /// process and shared.
    pub fn placeholders(set: TextureSet) -> Arc<MaterialImages> {
        static SETS: [OnceLock<Arc<MaterialImages>>; 4] =
            [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
        let i = set as usize;
        SETS[i]
            .get_or_init(|| Arc::new(placeholder::set(set)))
            .clone()
    }

    /// The image of a material, if the set has one.
    pub fn get(&self, material: usize) -> Option<&MaterialImage> {
        self.images.iter().find(|i| i.material == material)
    }

    /// `self` with every material it lacks taken from `fallback`.
    pub fn or(mut self, fallback: &MaterialImages) -> MaterialImages {
        for img in &fallback.images {
            if self.get(img.material).is_none() {
                self.images.push(img.clone());
            }
        }
        self.images.sort_by_key(|i| i.material);
        self
    }

    /// Reads the converted textures of `set` from an asset library cache (the folder
    /// `tools/assets/fetch.py` fills; see [`default_asset_cache`]). Each material takes the first
    /// set (by asset id) whose `material.json` names it with the set's role, at 1024 px or the
    /// largest size below. Unreadable sets are skipped with a warning; an empty result means the
    /// cache has none.
    pub fn from_cache(dir: &Path, set: TextureSet) -> MaterialImages {
        let mut out = MaterialImages::default();
        let Some(role) = set.role() else {
            return out;
        };
        let Ok(entries) = std::fs::read_dir(dir.join("converted")) else {
            return out;
        };
        let mut ids: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        ids.sort();
        for asset in ids {
            match cache::read(&asset, role, set) {
                Ok(Some(img)) if out.get(img.material).is_none() => out.images.push(img),
                Ok(_) => {}
                Err(e) => log::warn!("asset cache {}: {e}", asset.display()),
            }
        }
        out.images.sort_by_key(|i| i.material);
        out
    }
}

/// The asset library cache: `METALLIC_ASSET_CACHE`, else `%LOCALAPPDATA%/Metallic/asset-cache`
/// (Windows) or `$XDG_CACHE_HOME`/`~/.cache` + `metallic/asset-cache` elsewhere. `None` when the
/// folder does not exist.
pub fn default_asset_cache() -> Option<PathBuf> {
    let dir = if let Some(d) = std::env::var_os("METALLIC_ASSET_CACHE") {
        PathBuf::from(d)
    } else if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(d).join("Metallic").join("asset-cache")
    } else if let Some(d) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(d).join("metallic").join("asset-cache")
    } else {
        PathBuf::from(std::env::var_os("HOME")?)
            .join(".cache")
            .join("metallic")
            .join("asset-cache")
    };
    dir.is_dir().then_some(dir)
}

/// Normalises raw maps into a [`MaterialImage`]: `albedo` sRGB RGBA8, `normal` RGB8/RGBA8
/// (OpenGL), `orm` RGB8/RGBA8 (glTF order), all `size` x `size`; `channels` is 3 or 4.
#[allow(clippy::too_many_arguments)]
pub fn normalise(
    material: usize,
    size: u32,
    albedo_srgb: &[u8],
    albedo_channels: usize,
    normal: &[u8],
    normal_channels: usize,
    orm: &[u8],
    orm_channels: usize,
) -> MaterialImage {
    let n = (size * size) as usize;
    let lin: Vec<[f32; 3]> = (0..n)
        .map(|i| {
            let p = &albedo_srgb[i * albedo_channels..];
            [p[0], p[1], p[2]].map(|c| crate::scene::srgb_to_linear(f32::from(c) / 255.0))
        })
        .collect();
    let rough: Vec<f32> = (0..n)
        .map(|i| f32::from(orm[i * orm_channels + 1]) / 255.0)
        .collect();
    let mut out = MaterialImage {
        material,
        size,
        detail: Vec::with_capacity(n * 4),
        normal: Vec::with_capacity(n * 2),
        orm: Vec::with_capacity(n * 4),
        tile_studs: MATERIAL_TABLE
            .get(material)
            .map_or(4.0, |m| m.tile_studs.max(1.0)),
        nearest: false,
        normal_strength: 1.0,
        source: String::new(),
    };
    pack_detail(&lin, &mut out.detail);
    for i in 0..n {
        let p = &normal[i * normal_channels..];
        out.normal.extend_from_slice(&[p[0], p[1]]);
    }
    let mean_r = mean(&rough).max(0.02);
    for i in 0..n {
        let o = &orm[i * orm_channels..];
        out.orm
            .extend_from_slice(&[o[0], unit8(rough[i] / mean_r * 0.5), o[2], 255]);
    }
    out
}

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len().max(1) as f32
}

fn unit8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Writes `albedo / mean x 0.5` as RGBA8.
fn pack_detail(albedo: &[[f32; 3]], out: &mut Vec<u8>) {
    let mut m = [0.0f32; 3];
    for a in albedo {
        for c in 0..3 {
            m[c] += a[c];
        }
    }
    let m = m.map(|v| (v / albedo.len().max(1) as f32).max(0.02));
    for a in albedo {
        out.extend_from_slice(&[
            unit8(a[0] / m[0] * 0.5),
            unit8(a[1] / m[1] * 0.5),
            unit8(a[2] / m[2] * 0.5),
            255,
        ]);
    }
}

/// The next mip level of a `size` x `size` image with `channels` 8-bit channels (2x2 box).
pub fn downsample(src: &[u8], size: u32, channels: usize) -> Vec<u8> {
    let half = (size / 2).max(1);
    let s = size as usize;
    let mut out = Vec::with_capacity((half * half) as usize * channels);
    for y in 0..half as usize {
        for x in 0..half as usize {
            for c in 0..channels {
                let at = |dx: usize, dy: usize| {
                    let (sx, sy) = ((2 * x + dx).min(s - 1), (2 * y + dy).min(s - 1));
                    u32::from(src[(sy * s + sx) * channels + c])
                };
                out.push(((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8);
            }
        }
    }
    out
}

/// Number of mip levels of a `size` texture.
pub fn mip_count(size: u32) -> u32 {
    32 - size.max(1).leading_zeros()
}

/// The size class an image of `size` pixels goes into (the largest class not above it).
pub fn class_of(size: u32) -> usize {
    SIZE_CLASSES
        .iter()
        .rposition(|&c| c <= size)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// GPU arrays

/// Formats of the three maps.
const FORMATS: [wgpu::TextureFormat; 3] = [
    wgpu::TextureFormat::Rgba8Unorm,
    wgpu::TextureFormat::Rg8Unorm,
    wgpu::TextureFormat::Rgba8Unorm,
];

/// The texture arrays the scene shader samples (bindings 4 to 14 of group 0).
pub(crate) struct TextureArrays {
    /// Per size class: detail, normal, ORM views.
    views: Vec<[wgpu::TextureView; 3]>,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    /// GPU bytes of all mips (for diagnostics).
    pub(crate) bytes: u64,
}

impl TextureArrays {
    /// One 1 x 1 layer per array: what an untextured scene binds.
    pub(crate) fn empty(device: &wgpu::Device, queue: &wgpu::Queue) -> TextureArrays {
        Self::upload(device, queue, &MaterialImages::default()).0
    }

    /// Uploads `images` (each into the size class matching its size, resized down if it lies
    /// between classes) and returns the arrays and each material's [`TextureRef`].
    pub(crate) fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &MaterialImages,
    ) -> (TextureArrays, [Option<TextureRef>; MATERIAL_COUNT]) {
        let mut refs = [None; MATERIAL_COUNT];
        let mut views = Vec::new();
        let mut bytes = 0u64;
        for (class, &size) in SIZE_CLASSES.iter().enumerate() {
            let members: Vec<&MaterialImage> = images
                .images
                .iter()
                .filter(|i| i.material < MATERIAL_COUNT && class_of(i.size) == class)
                .collect();
            let (side, layers) = if members.is_empty() {
                (1, 1)
            } else {
                (size, members.len() as u32)
            };
            let mips = mip_count(side);
            let make = |format: wgpu::TextureFormat, label: &str| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: layers,
                    },
                    mip_level_count: mips,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            };
            let textures = [
                make(FORMATS[0], "metallic material detail"),
                make(FORMATS[1], "metallic material normal"),
                make(FORMATS[2], "metallic material orm"),
            ];
            if members.is_empty() {
                // Neutral texel: detail 0.5 (x1), flat normal, AO 1, roughness x1, metal 1.
                let texels: [&[u8]; 3] = [&[128, 128, 128, 255], &[128, 128], &[255, 128, 255, 255]];
                for (t, data) in textures.iter().zip(texels) {
                    write_layer(queue, t, 0, 0, 1, data);
                }
            }
            for (layer, img) in members.iter().enumerate() {
                let (d, n, o) = fit(img, size);
                for (t, (data, ch)) in textures.iter().zip([(d, 4), (n, 2), (o, 4)]) {
                    let mut level = data;
                    let mut s = size;
                    for mip in 0..mips {
                        write_layer(queue, t, layer as u32, mip, s, &level);
                        bytes += level.len() as u64;
                        if mip + 1 < mips {
                            level = downsample(&level, s, ch);
                            s = (s / 2).max(1);
                        }
                    }
                }
                refs[img.material] = Some(TextureRef {
                    layer: layer as u32,
                    class: class as u32,
                    nearest: img.nearest || class == 0,
                    tile_studs: img.tile_studs,
                    normal_strength: img.normal_strength,
                });
            }
            views.push(textures.map(|t| {
                t.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                })
            }));
        }
        let sampler = |filter: wgpu::FilterMode, aniso: u16, label: &str| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::Repeat,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: match filter {
                    wgpu::FilterMode::Linear => wgpu::MipmapFilterMode::Linear,
                    wgpu::FilterMode::Nearest => wgpu::MipmapFilterMode::Nearest,
                },
                anisotropy_clamp: aniso,
                ..Default::default()
            })
        };
        let arrays = TextureArrays {
            views,
            linear: sampler(wgpu::FilterMode::Linear, 8, "metallic material linear"),
            nearest: sampler(wgpu::FilterMode::Nearest, 1, "metallic material nearest"),
            bytes,
        };
        (arrays, refs)
    }

    /// Bind group entries 4 to 14: per class detail, normal, ORM; then the linear and the
    /// nearest sampler.
    pub(crate) fn entries(&self) -> Vec<wgpu::BindGroupEntry<'_>> {
        let mut out = Vec::with_capacity(11);
        for (class, views) in self.views.iter().enumerate() {
            for (map, view) in views.iter().enumerate() {
                out.push(wgpu::BindGroupEntry {
                    binding: 4 + (class * 3 + map) as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
        }
        out.push(wgpu::BindGroupEntry {
            binding: 13,
            resource: wgpu::BindingResource::Sampler(&self.linear),
        });
        out.push(wgpu::BindGroupEntry {
            binding: 14,
            resource: wgpu::BindingResource::Sampler(&self.nearest),
        });
        out
    }

    /// The matching layout entries (fragment stage).
    pub(crate) fn layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
        let mut out = Vec::with_capacity(11);
        for binding in 4..13 {
            out.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            });
        }
        for binding in [13, 14] {
            out.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        out
    }
}

/// `img`'s maps at exactly `size` (box-downsampled when larger; never larger than the image).
fn fit(img: &MaterialImage, size: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut maps = (img.detail.clone(), img.normal.clone(), img.orm.clone());
    let mut s = img.size;
    while s > size {
        maps = (
            downsample(&maps.0, s, 4),
            downsample(&maps.1, s, 2),
            downsample(&maps.2, s, 4),
        );
        s /= 2;
    }
    maps
}

fn write_layer(queue: &wgpu::Queue, texture: &wgpu::Texture, layer: u32, mip: u32, size: u32, data: &[u8]) {
    let bpp = data.len() as u32 / (size * size).max(1);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * bpp),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
}

// ---------------------------------------------------------------------------------------------
// Asset library cache

mod cache {
    use super::*;

    /// Reads one converted asset folder if it is a texture of `role`.
    pub(super) fn read(asset: &Path, role: &str, set: TextureSet) -> Result<Option<MaterialImage>, String> {
        let Ok(text) = std::fs::read_to_string(asset.join("material.json")) else {
            return Ok(None);
        };
        let info: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if info["role"].as_str() != Some(role) {
            return Ok(None);
        }
        let Some(material) = info["material"]
            .as_str()
            .and_then(|m| MATERIAL_TABLE.iter().position(|p| p.name == m))
        else {
            return Ok(None);
        };
        if MATERIAL_TABLE[material].tile_studs <= 0.0 {
            return Ok(None);
        }
        let nearest = info["filter"].as_str() == Some("nearest") || set == TextureSet::Pixel;
        // The largest converted size up to 1024.
        let mut sizes: Vec<u32> = info["sizes"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as u32).collect())
            .unwrap_or_default();
        sizes.sort_unstable();
        let Some(&size) = sizes.iter().rev().find(|&&s| s <= 1024 && s.is_power_of_two()) else {
            return Err("no power-of-two size up to 1024".into());
        };
        let dir = asset.join(size.to_string());
        let (albedo, ac, w) = read_png(&dir.join("albedo.png"))?;
        if w != size {
            return Err(format!("albedo is {w} px, expected {size}"));
        }
        let (normal, nc) = match read_png(&dir.join("normal.png")) {
            Ok((n, c, nw)) if nw == size => (n, c),
            _ => (vec![128, 128, 255].repeat((size * size) as usize), 3),
        };
        let (orm, oc, ow) = read_png(&dir.join("orm.png"))?;
        if ow != size {
            return Err(format!("orm is {ow} px, expected {size}"));
        }
        let mut img = normalise(material, size, &albedo, ac, &normal, nc, &orm, oc);
        if let Some(m) = info["tile_size_m"].as_f64() {
            // 0.28 m per stud.
            img.tile_studs = (m / 0.28).max(1.0) as f32;
        }
        img.nearest = nearest;
        img.normal_strength = if set == TextureSet::Stylized { 0.5 } else { 1.0 };
        img.source = format!(
            "{} (asset library, licence in LICENSES/ASSETS.md)",
            info["id"].as_str().unwrap_or("?")
        );
        Ok(Some(img))
    }

    /// 8-bit RGB or RGBA PNG: pixels, channels, width (square images only).
    fn read_png(path: &Path) -> Result<(Vec<u8>, usize, u32), String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("image too large")?];
        let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        if frame.width != frame.height || frame.width > 4096 {
            return Err(format!("{}: {}x{} is not a supported square", path.display(), frame.width, frame.height));
        }
        let channels = match frame.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Grayscale => {
                buf.truncate(frame.buffer_size());
                let rgb = buf.iter().flat_map(|&v| [v, v, v]).collect();
                return Ok((rgb, 3, frame.width));
            }
            other => return Err(format!("{}: unsupported colour type {other:?}", path.display())),
        };
        buf.truncate(frame.buffer_size());
        Ok((buf, channels, frame.width))
    }
}

// ---------------------------------------------------------------------------------------------
// Placeholders

mod placeholder {
    //! Tileable textures drawn by code: periodic value noise, Voronoi cells and patterns per
    //! material. Own work (no source images), licensed with the engine.
    use super::*;

    /// One texel before normalisation.
    #[derive(Clone, Copy)]
    struct Texel {
        albedo: [f32; 3],
        height: f32,
        rough: f32,
        metal: f32,
        ao: f32,
    }

    fn texel(albedo: [f32; 3], height: f32, rough: f32) -> Texel {
        Texel {
            albedo,
            height,
            rough,
            metal: 0.0,
            ao: 1.0,
        }
    }

    fn grey(v: f32) -> [f32; 3] {
        [v, v, v]
    }

    fn hash(x: i32, y: i32, seed: u32) -> f32 {
        let mut h = (x as u32)
            .wrapping_mul(0x8da6_b343)
            ^ (y as u32).wrapping_mul(0xd816_3841)
            ^ seed.wrapping_mul(0xcb1a_b31f);
        h ^= h >> 13;
        h = h.wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        (h & 0x00ff_ffff) as f32 / 16_777_215.0
    }

    /// Value noise of period `p` cells; (u, v) in 0..1 covers the period once.
    fn noise(u: f32, v: f32, p: i32, seed: u32) -> f32 {
        let (x, y) = (u * p as f32, v * p as f32);
        let (ix, iy) = (x.floor() as i32, y.floor() as i32);
        let (fx, fy) = (x - ix as f32, y - iy as f32);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let w = |a: i32| a.rem_euclid(p);
        let h = |dx: i32, dy: i32| hash(w(ix + dx), w(iy + dy), seed);
        let a = h(0, 0) + (h(1, 0) - h(0, 0)) * sx;
        let b = h(0, 1) + (h(1, 1) - h(0, 1)) * sx;
        a + (b - a) * sy
    }

    fn fbm(u: f32, v: f32, p: i32, seed: u32, octaves: u32) -> f32 {
        let (mut sum, mut amp, mut norm, mut period) = (0.0, 0.5, 0.0, p);
        for o in 0..octaves {
            sum += amp * noise(u, v, period, seed + o * 7);
            norm += amp;
            amp *= 0.5;
            period *= 2;
        }
        sum / norm
    }

    /// Voronoi of period `p` cells: distance to the nearest and second nearest point, and the
    /// nearest cell's random value.
    fn cells(u: f32, v: f32, p: i32, seed: u32) -> (f32, f32, f32) {
        let (x, y) = (u * p as f32, v * p as f32);
        let (ix, iy) = (x.floor() as i32, y.floor() as i32);
        let (mut d1, mut d2, mut id) = (9.0f32, 9.0f32, 0.0);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy) = ((ix + dx).rem_euclid(p), (iy + dy).rem_euclid(p));
                let px = (ix + dx) as f32 + hash(cx, cy, seed);
                let py = (iy + dy) as f32 + hash(cx, cy, seed + 1);
                let d = ((px - x).powi(2) + (py - y).powi(2)).sqrt();
                if d < d1 {
                    d2 = d1;
                    d1 = d;
                    id = hash(cx, cy, seed + 2);
                } else if d < d2 {
                    d2 = d;
                }
            }
        }
        (d1, d2, id)
    }

    fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
        [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
    }

    /// Masonry: rows of `rows` bricks, `cols` per row, offset by half every other row. Returns
    /// (distance to the joint in brick-height units, brick random value).
    fn bond(u: f32, v: f32, cols: f32, rows: f32, seed: u32) -> (f32, f32) {
        let r = (v * rows).floor();
        let off = if (r as i32) % 2 == 1 { 0.5 } else { 0.0 };
        let x = u * cols + off;
        let (bx, by) = (x.fract(), (v * rows).fract());
        let aspect = rows / cols;
        let d = (bx.min(1.0 - bx) / aspect).min(by.min(1.0 - by));
        let id = hash(x.floor() as i32 % cols as i32, r as i32, seed);
        (d, id)
    }

    /// The texel of `material` at (u, v) in one repeat.
    fn sample(material: &str, u: f32, v: f32) -> Texel {
        match material {
            "Metal" => {
                let s = noise(u, v * 0.02, 64, 1) * 0.6 + noise(u, v * 0.03, 256, 2) * 0.4;
                let scratch = smooth(0.92, 0.97, noise(u * 0.3 + v, v, 48, 3));
                Texel {
                    metal: 1.0,
                    ..texel(grey(0.82 + 0.16 * s + 0.08 * scratch), 0.2 * s, 0.85 + 0.3 * s)
                }
            }
            "Wood" => {
                let warp = fbm(u, v, 4, 4, 3) * 2.5;
                let ring = (v * 12.0 + warp).fract();
                let band = smooth(0.0, 0.6, ring) * (1.0 - smooth(0.6, 1.0, ring));
                let pores = noise(u * 0.1, v, 256, 5);
                let a = mix3([0.55, 0.36, 0.2], [0.72, 0.5, 0.3], band);
                texel(a.map(|c| c * (0.92 + 0.12 * pores)), 0.3 * band + 0.1 * pores, 0.9 + 0.2 * pores)
            }
            "WoodPlanks" => {
                let (d, id) = bond(u, v, 2.0, 8.0, 6);
                let seam = smooth(0.0, 0.05, d);
                let warp = fbm(u, v, 4, 7, 3) * 2.0;
                let grain = ((v * 64.0 + warp + id * 9.0).fract() - 0.5).abs() * 2.0;
                let tone = 0.75 + 0.4 * id;
                let a = mix3([0.5, 0.33, 0.18], [0.7, 0.5, 0.3], grain).map(|c| c * tone * (0.35 + 0.65 * seam));
                Texel {
                    ao: 0.6 + 0.4 * seam,
                    ..texel(a, seam * (0.9 + 0.1 * grain), 1.0 + 0.3 * (1.0 - seam))
                }
            }
            "Concrete" => {
                let s = fbm(u, v, 8, 8, 5);
                let pits = smooth(0.8, 0.9, noise(u, v, 96, 9));
                texel(grey(0.55 + 0.35 * s - 0.15 * pits), s * 0.6 - pits * 0.3, 0.95 + 0.1 * s)
            }
            "Grass" => {
                let patches = fbm(u, v, 4, 10, 3);
                let blades = noise(u, v * 0.25, 256, 11) * 0.6 + noise(u, v, 128, 12) * 0.4;
                let a = mix3([0.18, 0.32, 0.08], [0.36, 0.5, 0.14], patches);
                texel(a.map(|c| c * (0.7 + 0.5 * blades)), blades, 1.0)
            }
            "Brick" => {
                let (d, id) = bond(u, v, 4.0, 8.0, 13);
                let mortar = 1.0 - smooth(0.04, 0.1, d);
                let grain = noise(u, v, 128, 14);
                let brick = mix3([0.5, 0.2, 0.14], [0.66, 0.3, 0.2], id).map(|c| c * (0.9 + 0.2 * grain));
                Texel {
                    ao: 1.0 - 0.4 * mortar,
                    ..texel(mix3(brick, [0.62, 0.6, 0.56], mortar), (1.0 - mortar) * (0.8 + 0.2 * grain), 1.0 + 0.15 * mortar)
                }
            }
            "Ice" => {
                let s = fbm(u, v, 4, 15, 4);
                let crack = 1.0 - smooth(0.0, 0.02, (s - 0.5).abs());
                texel(mix3([0.7, 0.85, 0.95], [0.92, 0.97, 1.0], s + 0.5 * crack), 0.3 * s - 0.4 * crack, 1.0 + 1.5 * crack)
            }
            "Sand" => {
                let grains = noise(u, v, 256, 16);
                let ripple = noise(u * 0.3, v, 12, 17);
                texel(grey(0.8 + 0.12 * grains + 0.1 * ripple).map(|c| c * 0.95), 0.6 * ripple + 0.2 * grains, 1.0)
            }
            "Slate" => {
                let (d1, d2, id) = cells(u * 0.6, v, 6, 18);
                let edge = smooth(0.0, 0.08, d2 - d1);
                let layers = noise(u, v * 0.1, 64, 19);
                texel(grey((0.32 + 0.18 * id + 0.08 * layers) * (0.5 + 0.5 * edge)), edge * (0.6 + 0.3 * id) + 0.1 * layers, 0.9 + 0.2 * id)
            }
            "Cobblestone" => {
                let (d1, d2, id) = cells(u, v, 6, 20);
                let gap = smooth(0.02, 0.12, d2 - d1);
                let dome = (1.0 - d1).max(0.0);
                let grain = noise(u, v, 128, 21);
                let a = grey((0.38 + 0.25 * id) * (0.85 + 0.2 * grain) * (0.35 + 0.65 * gap));
                Texel {
                    ao: 0.5 + 0.5 * gap,
                    ..texel(a, gap * (0.5 + 0.5 * dome), 1.0 + 0.2 * (1.0 - gap))
                }
            }
            "Marble" => {
                let w = fbm(u, v, 4, 22, 5);
                let vein = 1.0 - smooth(0.0, 0.06, ((u + v + w * 2.0) * 6.0).sin().abs());
                let cloud = fbm(u, v, 8, 23, 3);
                texel(grey(0.88 + 0.08 * cloud - 0.4 * vein), 0.1 * cloud, 0.9 + 0.4 * vein)
            }
            "Granite" => {
                let speck = noise(u, v, 256, 24);
                let big = noise(u, v, 64, 25);
                let a = if speck > 0.8 {
                    grey(0.15)
                } else if big > 0.7 {
                    [0.75, 0.6, 0.55]
                } else {
                    grey(0.55 + 0.2 * speck)
                };
                texel(a, 0.2 * speck, 0.9 + 0.2 * speck)
            }
            "Pebble" => {
                let (d1, d2, id) = cells(u, v, 12, 26);
                let stone = smooth(0.02, 0.1, d2 - d1);
                let dome = (1.0 - 1.4 * d1).max(0.0).sqrt();
                let a = mix3([0.4, 0.37, 0.33], [0.72, 0.68, 0.6], id).map(|c| c * (0.4 + 0.6 * stone));
                Texel {
                    ao: 0.5 + 0.5 * stone,
                    ..texel(a, stone * dome, 0.9 + 0.2 * (1.0 - stone))
                }
            }
            "Fabric" => {
                let n = 64.0;
                let (x, y) = ((u * n).fract(), (v * n).fract());
                let warp = ((u * n).floor() as i32 + (v * n).floor() as i32) % 2 == 0;
                let thread = if warp {
                    (x * std::f32::consts::PI).sin()
                } else {
                    (y * std::f32::consts::PI).sin()
                };
                let fuzz = noise(u, v, 256, 27);
                texel(grey(0.7 + 0.25 * thread + 0.08 * fuzz), 0.6 * thread, 1.0)
            }
            "DiamondPlate" => {
                let n = 8.0;
                let (x, y) = ((u * n).fract() - 0.5, (v * n * 2.0).fract() - 0.5);
                // Two crossed lozenges per cell, alternating.
                let odd = ((u * n).floor() as i32 + (v * n * 2.0).floor() as i32) % 2 == 0;
                let (a, b) = if odd { (x + y, x - y) } else { (x - y, x + y) };
                let bump = (1.0 - smooth(0.06, 0.1, a.abs())) * (1.0 - smooth(0.3, 0.38, b.abs()));
                let s = noise(u, v * 0.05, 128, 28);
                Texel {
                    metal: 1.0,
                    ..texel(grey(0.78 + 0.12 * s + 0.1 * bump), bump, 0.9 + 0.2 * s)
                }
            }
            "CorrodedMetal" => {
                let rust = smooth(0.45, 0.6, fbm(u, v, 6, 29, 5));
                let pits = noise(u, v, 128, 30);
                let a = mix3(grey(0.62 + 0.1 * pits), [0.5, 0.26, 0.12], rust);
                Texel {
                    metal: 1.0 - rust,
                    ..texel(a, 0.3 * rust + 0.15 * pits, 0.7 + 0.6 * rust)
                }
            }
            "Snow" => {
                let s = fbm(u, v, 4, 31, 4);
                let sparkle = smooth(0.96, 0.99, noise(u, v, 256, 32));
                texel(grey(0.9 + 0.06 * s + 0.04 * sparkle), s, 1.0 - 0.5 * sparkle)
            }
            "Rock" => {
                let ridged = 1.0 - (fbm(u, v, 6, 33, 5) * 2.0 - 1.0).abs();
                let (d1, d2, _) = cells(u, v, 4, 34);
                let crack = 1.0 - smooth(0.0, 0.04, d2 - d1);
                texel(grey((0.42 + 0.25 * ridged) * (1.0 - 0.5 * crack)), ridged - 0.6 * crack, 1.0)
            }
            "Asphalt" => {
                let fine = noise(u, v, 256, 35);
                let stone = smooth(0.75, 0.85, noise(u, v, 128, 36));
                texel(grey(0.22 + 0.08 * fine + 0.12 * stone), 0.3 * stone + 0.1 * fine, 1.0 - 0.2 * stone)
            }
            _ => texel(grey(0.8), 0.0, 1.0),
        }
    }

    /// Draws one material at `size` px.
    fn draw(material: usize, size: u32) -> MaterialImage {
        let m = &MATERIAL_TABLE[material];
        let n = (size * size) as usize;
        let mut texels = Vec::with_capacity(n);
        for y in 0..size {
            for x in 0..size {
                let (u, v) = ((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32);
                texels.push(sample(m.name, u, v));
            }
        }
        let albedo: Vec<[f32; 3]> = texels.iter().map(|t| t.albedo).collect();
        let mut img = MaterialImage {
            material,
            size,
            detail: Vec::with_capacity(n * 4),
            normal: Vec::with_capacity(n * 2),
            orm: Vec::with_capacity(n * 4),
            tile_studs: m.tile_studs,
            nearest: false,
            normal_strength: 1.0,
            source: "placeholder (drawn by metallic_render::textures, own work)".into(),
        };
        pack_detail(&albedo, &mut img.detail);
        // Normals from height differences: a height step of 1 over one repeat-pixel is steep,
        // so scale to a 4-pixel bevel.
        let s = size as i32;
        let h = |x: i32, y: i32| texels[(y.rem_euclid(s) * s + x.rem_euclid(s)) as usize].height;
        let k = size as f32 / 64.0;
        for y in 0..s {
            for x in 0..s {
                let dx = (h(x + 1, y) - h(x - 1, y)) * k;
                let dy = (h(x, y - 1) - h(x, y + 1)) * k;
                let len = (dx * dx + dy * dy + 1.0).sqrt();
                img.normal
                    .extend_from_slice(&[unit8(-dx / len * 0.5 + 0.5), unit8(-dy / len * 0.5 + 0.5)]);
            }
        }
        let mean_r = mean(&texels.iter().map(|t| t.rough).collect::<Vec<_>>()).max(0.02);
        for t in &texels {
            img.orm
                .extend_from_slice(&[unit8(t.ao), unit8(t.rough / mean_r * 0.5), unit8(t.metal), 255]);
        }
        img
    }

    /// Posterises detail to `levels` steps around the mean and scales its contrast.
    fn posterise(img: &mut MaterialImage, levels: f32, contrast: f32) {
        for px in img.detail.chunks_exact_mut(4) {
            for c in &mut px[..3] {
                let v = f32::from(*c) / 127.5; // 1.0 = the mean
                let v = 1.0 + (v - 1.0) * contrast;
                let q = (v * levels).round() / levels;
                *c = unit8(q * 0.5);
            }
        }
    }

    pub(super) fn set(set: TextureSet) -> MaterialImages {
        let mut images = Vec::new();
        for (i, m) in MATERIAL_TABLE.iter().enumerate() {
            if m.tile_studs <= 0.0 {
                continue;
            }
            let img = match set {
                TextureSet::Procedural => continue,
                TextureSet::Realistic => draw(i, SIZE_CLASSES[1]),
                TextureSet::Stylized => {
                    let mut img = draw(i, SIZE_CLASSES[1]);
                    posterise(&mut img, 4.0, 0.6);
                    img.normal_strength = 0.4;
                    img
                }
                TextureSet::Pixel => {
                    let big = draw(i, SIZE_CLASSES[1]);
                    let (mut d, mut n, mut o) = (big.detail, big.normal, big.orm);
                    let mut s = big.size;
                    while s > SIZE_CLASSES[0] {
                        d = downsample(&d, s, 4);
                        n = downsample(&n, s, 2);
                        o = downsample(&o, s, 4);
                        s /= 2;
                    }
                    let mut img = MaterialImage {
                        size: s,
                        detail: d,
                        normal: n,
                        orm: o,
                        nearest: true,
                        normal_strength: 0.6,
                        ..big
                    };
                    posterise(&mut img, 6.0, 1.2);
                    img
                }
            };
            images.push(img);
        }
        MaterialImages { images }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_cover_textured_materials_and_are_normalised() {
        let set = MaterialImages::placeholders(TextureSet::Realistic);
        for (i, m) in MATERIAL_TABLE.iter().enumerate() {
            assert_eq!(set.get(i).is_some(), m.tile_studs > 0.0, "{}", m.name);
        }
        for img in &set.images {
            let n = (img.size * img.size) as usize;
            assert_eq!((img.detail.len(), img.normal.len(), img.orm.len()), (n * 4, n * 2, n * 4));
            // The detail mean is 0.5 (x1): the part colour keeps its brightness.
            let mean_g = img.detail.iter().skip(1).step_by(4).map(|&v| f32::from(v)).sum::<f32>() / n as f32;
            assert!((mean_g - 127.5).abs() < 6.0, "{}: {mean_g}", MATERIAL_TABLE[img.material].name);
        }
        assert!(Arc::ptr_eq(&set, &MaterialImages::placeholders(TextureSet::Realistic)), "drawn once");
        assert!(MaterialImages::placeholders(TextureSet::Procedural).images.is_empty());
    }

    #[test]
    fn placeholders_tile_without_seams() {
        let set = MaterialImages::placeholders(TextureSet::Realistic);
        for img in &set.images {
            let s = img.size as usize;
            let px = |x: usize, y: usize| i32::from(img.detail[(y * s + x) * 4 + 1]);
            // Opposite edges differ about as much as neighbouring columns do.
            let edge: i32 = (0..s).map(|y| (px(0, y) - px(s - 1, y)).abs()).sum();
            let inner: i32 = (0..s).map(|y| (px(s / 2, y) - px(s / 2 - 1, y)).abs()).sum();
            assert!(edge <= inner * 3 + s as i32 * 4, "{}: {edge} vs {inner}", MATERIAL_TABLE[img.material].name);
        }
    }

    #[test]
    fn pixel_and_stylized_sets() {
        let px = MaterialImages::placeholders(TextureSet::Pixel);
        assert!(px.images.iter().all(|i| i.size == 32 && i.nearest));
        let st = MaterialImages::placeholders(TextureSet::Stylized);
        let real = MaterialImages::placeholders(TextureSet::Realistic);
        let distinct = |img: &MaterialImage| {
            let mut v: Vec<u8> = img.detail.iter().skip(1).step_by(4).copied().collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        let brick = crate::materials::material_index("Brick");
        assert!(distinct(st.get(brick).unwrap()) < distinct(real.get(brick).unwrap()));
    }

    #[test]
    fn mips_and_classes() {
        assert_eq!(mip_count(1), 1);
        assert_eq!(mip_count(256), 9);
        assert_eq!(class_of(16), 0);
        assert_eq!(class_of(32), 0);
        assert_eq!(class_of(512), 1);
        assert_eq!(class_of(2048), 2);
        let img = [0u8, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150];
        assert_eq!(downsample(&img, 4, 1), vec![25, 45, 105, 125]);
    }

    #[test]
    fn the_asset_cache_is_read_by_role_and_material() {
        let dir = std::env::temp_dir().join(format!("metallic-tex-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |id: &str, material: &str, role: &str| {
            let d = dir.join("converted").join(id).join("64");
            std::fs::create_dir_all(&d).unwrap();
            let json = format!(
                r#"{{"id":"{id}","material":"{material}","role":"{role}","sizes":[64],"tile_size_m":1.12,"filter":"linear"}}"#
            );
            std::fs::write(dir.join("converted").join(id).join("material.json"), json).unwrap();
            for (name, ty, px) in [
                ("albedo.png", png::ColorType::Rgb, vec![200u8, 100, 50]),
                ("normal.png", png::ColorType::Rgb, vec![128, 128, 255]),
                ("orm.png", png::ColorType::Rgb, vec![255, 200, 0]),
            ] {
                let f = std::fs::File::create(d.join(name)).unwrap();
                let mut enc = png::Encoder::new(f, 64, 64);
                enc.set_color(ty);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header().unwrap().write_image_data(&px.repeat(64 * 64)).unwrap();
            }
        };
        write("tex-brick-a", "Brick", "primary");
        write("tex-brick-b", "Brick", "alternate");
        write("tex-plastic", "Plastic", "primary");
        write("tex-wood-s", "Wood", "stylized");
        let real = MaterialImages::from_cache(&dir, TextureSet::Realistic);
        assert_eq!(real.images.len(), 1, "Plastic stays untextured; alternates are not used");
        let brick = &real.images[0];
        assert_eq!(MATERIAL_TABLE[brick.material].name, "Brick");
        assert_eq!(brick.size, 64);
        assert_eq!(brick.tile_studs, 4.0, "1.12 m at 0.28 m per stud");
        assert_eq!(&brick.detail[..4], &[128, 128, 128, 255], "flat colour normalises to x1");
        assert_eq!(brick.orm[1], 128, "roughness normalised to its mean");
        assert!(brick.source.contains("tex-brick-a"));
        let styl = MaterialImages::from_cache(&dir, TextureSet::Stylized);
        assert_eq!(styl.images.len(), 1);
        assert_eq!(MATERIAL_TABLE[styl.images[0].material].name, "Wood");
        // Missing materials come from the placeholders.
        let full = real.or(&MaterialImages::placeholders(TextureSet::Realistic));
        assert_eq!(full.get(crate::materials::material_index("Brick")).unwrap().size, 64);
        assert_eq!(full.get(crate::materials::material_index("Wood")).unwrap().size, 256);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
