//! The material table: the shading parameters of every material row, indexed by the instance's material row.
//!
//! Rows are data, not code (#678, ADR 0012). The scene numbers every material key its parts name, an asset id or a
//! name, from row 1 ([`crate::InstanceScene::material_keys`]); row 0 is the default look. Each row is filled from
//! the parameters given for its key ([`MaterialTable::set_material`]: the engine gives a material asset's), else from
//! the built-in material of that name ([`MaterialParams::builtin`]), else plastic, and from the texture layers given
//! for it ([`MaterialTable::set_layers`], #679). A key's parameters can change at any time, and every part using it
//! changes with them. The table is a uniform buffer (readable on every backend, GLES included) of
//! [`MAX_MATERIAL_ROWS`] rows, 80 bytes each, uploaded only when a row changes.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};

use super::material_textures::MaterialLayers;

/// How many rows the table holds: 80 bytes each, within the 16 KiB of uniform binding WebGPU guarantees. Rows
/// beyond it shade as row 0.
pub const MAX_MATERIAL_ROWS: usize = 192;

/// What a material's textures do past their edges (#679).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaterialWrap {
    /// They repeat.
    #[default]
    Repeat,
    /// They repeat mirrored.
    Mirror,
    /// Their edges stretch on.
    Clamp,
}

/// The shading parameters of one material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialParams {
    /// Microfacet roughness, from 0 (a mirror) to 1 (fully matte).
    pub roughness: f32,
    /// 0 for dielectrics (plastic, wood), 1 for bare metal.
    pub metalness: f32,
    /// Light emitted, as a multiple of the part's colour (Neon glows with 1): `emission` times its strength.
    pub emissive: f32,
    /// Glass: when partly see-through, its reflections stay at full strength while the body fades.
    pub glass: bool,
    /// The base colour's tint, linear, multiplied by the part's colour.
    pub tint: [f32; 3],
    /// The emitted light's tint, linear, multiplied by the part's colour and `emissive`.
    pub emission: [f32; 3],
    /// How strongly the normal map bends the light: 0 flat, 1 as painted (#679).
    pub normal_strength: f32,
    /// How many metres one repeat of the textures covers on a primitive part (#679).
    pub tile_size: f32,
    /// Repeats per tile along U and V (#679).
    pub uv_scale: [f32; 2],
    /// Where the textures start, in tiles (#679).
    pub uv_offset: [f32; 2],
    /// Whether textures are sampled nearest rather than smoothly (#679).
    pub nearest: bool,
    /// What textures do past their edges (#679).
    pub wrap: MaterialWrap,
}

/// `MaterialRow::surface.z`'s flags (`MATERIAL_*` in `parts.wgsl`).
const GLASS: u32 = 1;
const TEXTURED: u32 = 2;
const NEAREST: u32 = 4;
const MIRROR: u32 = 8;
const CLAMP: u32 = 16;

impl MaterialParams {
    /// Plain plastic: the default, and what unknown materials shade as.
    pub const PLASTIC: Self = Self {
        roughness: 0.6,
        metalness: 0.0,
        emissive: 0.0,
        glass: false,
        tint: [1.0; 3],
        emission: [1.0; 3],
        normal_strength: 1.0,
        tile_size: 4.0,
        uv_scale: [1.0; 2],
        uv_offset: [0.0; 2],
        nearest: false,
        wrap: MaterialWrap::Repeat,
    };

    /// The parameters of the built-in material called `name`; `None` for names it does not know. The same set and
    /// values as `metallic_project::material::builtin` (an engine test keeps them together): the renderer's own
    /// fallback for a name nobody gave parameters for.
    pub fn builtin(name: &str) -> Option<Self> {
        let (roughness, metalness, emissive, glass) = match name {
            "Plastic" => return Some(Self::PLASTIC),
            "SmoothPlastic" => (0.3, 0.0, 0.0, false),
            "Metal" => (0.35, 1.0, 0.0, false),
            "DiamondPlate" => (0.4, 1.0, 0.0, false),
            "CorrodedMetal" => (0.8, 0.7, 0.0, false),
            "Foil" => (0.25, 1.0, 0.0, false),
            "Wood" => (0.75, 0.0, 0.0, false),
            "WoodPlanks" => (0.7, 0.0, 0.0, false),
            "Concrete" => (0.9, 0.0, 0.0, false),
            "Brick" => (0.85, 0.0, 0.0, false),
            "Cobblestone" => (0.85, 0.0, 0.0, false),
            "Granite" => (0.7, 0.0, 0.0, false),
            "Marble" => (0.25, 0.0, 0.0, false),
            "Slate" => (0.8, 0.0, 0.0, false),
            "Pebble" => (0.85, 0.0, 0.0, false),
            "Sand" => (0.95, 0.0, 0.0, false),
            "Grass" => (0.95, 0.0, 0.0, false),
            "Fabric" => (0.95, 0.0, 0.0, false),
            "Ice" => (0.1, 0.0, 0.0, false),
            "Glass" => (0.05, 0.0, 0.0, true),
            "Neon" => (1.0, 0.0, 1.0, false),
            _ => return None,
        };
        Some(Self {
            roughness,
            metalness,
            emissive,
            glass,
            ..Self::PLASTIC
        })
    }

    fn row(self, layers: MaterialLayers) -> MaterialRow {
        let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
        let emissive = if self.emissive.is_finite() {
            self.emissive.max(0.0)
        } else {
            0.0
        };
        let mut flags = 0;
        if self.glass {
            flags |= GLASS;
        }
        if layers.any() {
            flags |= TEXTURED;
        }
        if self.nearest {
            flags |= NEAREST;
        }
        flags |= match self.wrap {
            MaterialWrap::Repeat => 0,
            MaterialWrap::Mirror => MIRROR,
            MaterialWrap::Clamp => CLAMP,
        };
        let pair = |low: u16, high: u16| f32::from_bits(u32::from(low) | (u32::from(high) << 16));
        let tile = if self.tile_size.is_finite() && self.tile_size > 0.0 {
            self.tile_size
        } else {
            4.0
        };
        let scale = [finite(self.uv_scale[0], 1.0), finite(self.uv_scale[1], 1.0)];
        let offset = [finite(self.uv_offset[0], 0.0), finite(self.uv_offset[1], 0.0)];
        MaterialRow {
            // NaN stays NaN in roughness and metalness: a broken material should show, not be hidden (the post
            // tests poison a row on purpose).
            surface: [
                self.roughness.clamp(0.0, 1.0),
                self.metalness.clamp(0.0, 1.0),
                f32::from_bits(flags),
                finite(self.normal_strength, 1.0).clamp(0.0, 4.0),
            ],
            tint: [
                unit(self.tint[0]),
                unit(self.tint[1]),
                unit(self.tint[2]),
                pair(layers.color, layers.normal),
            ],
            emission: [
                unit(self.emission[0]) * emissive,
                unit(self.emission[1]) * emissive,
                unit(self.emission[2]) * emissive,
                pair(layers.orm, layers.emission),
            ],
            uv: [scale[0] / tile, scale[1] / tile, offset[0], offset[1]],
            mesh_uv: [scale[0], scale[1], offset[0], offset[1]],
        }
    }
}

impl Default for MaterialParams {
    fn default() -> Self {
        Self::PLASTIC
    }
}

/// One row as the shader reads it (`MaterialRow` in `parts.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct MaterialRow {
    /// Roughness, metalness, flags (bits), normal strength.
    surface: [f32; 4],
    /// The base colour's tint (linear); w: the colour and normal layers (bits).
    tint: [f32; 4],
    /// The emitted light (linear, times its strength); w: the occlusion-roughness-metallic and emission layers.
    emission: [f32; 4],
    /// A primitive's texture mapping: repeats per metre along U and V, then the offset.
    uv: [f32; 4],
    /// A mesh asset's: repeats along its own U and V, then the offset.
    mesh_uv: [f32; 4],
}

/// Size of the table in bytes.
pub(crate) const MATERIAL_TABLE_SIZE: u64 = (MAX_MATERIAL_ROWS * size_of::<MaterialRow>()) as u64;

/// Every material row's parameters, and the uniform buffer the shaders read them from.
#[derive(Debug)]
pub struct MaterialTable {
    rows: Vec<MaterialRow>,
    buffer: wgpu::Buffer,
    /// Parameters given for keys ([`MaterialTable::set_material`]).
    given: HashMap<Arc<str>, MaterialParams>,
    /// Texture layers given for keys ([`MaterialTable::set_layers`]).
    layers: HashMap<Arc<str>, MaterialLayers>,
    /// The scene revision the rows were last filled for, and how many keys it had.
    filled: Option<(u64, usize)>,
    /// Whether a key's parameters changed since the rows were filled.
    stale: bool,
    dirty: bool,
}

impl MaterialTable {
    /// A table of [`MAX_MATERIAL_ROWS`] rows of plastic.
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic materials"),
            size: MATERIAL_TABLE_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            rows: vec![MaterialParams::PLASTIC.row(MaterialLayers::NONE); MAX_MATERIAL_ROWS],
            buffer,
            given: HashMap::new(),
            layers: HashMap::new(),
            filled: None,
            stale: false,
            dirty: true,
        }
    }

    /// Gives the material named by `key` (an asset id or a name, as parts name it) its parameters: every row of that
    /// key shades with them from the next frame. Replaces what was given before.
    pub fn set_material(&mut self, key: &str, params: MaterialParams) {
        if self.given.get(key) != Some(&params) {
            self.given.insert(Arc::from(key), params);
            self.stale = true;
        }
    }

    /// Gives the material named by `key` its texture layers (#679, made by
    /// [`crate::Renderer::add_material_textures`]).
    pub fn set_layers(&mut self, key: &str, layers: MaterialLayers) {
        if self.layers.get(key) != Some(&layers) {
            self.layers.insert(Arc::from(key), layers);
            self.stale = true;
        }
    }

    /// Forgets the parameters and layers given for `key`: its rows go back to the built-in material of that name, or
    /// plastic.
    pub fn forget_material(&mut self, key: &str) {
        if self.given.remove(key).is_some() | self.layers.remove(key).is_some() {
            self.stale = true;
        }
    }

    /// The parameters a row with `key` shades with: those given, the built-in material of that name, or plastic.
    pub fn resolve(&self, key: &str) -> MaterialParams {
        self.given
            .get(key)
            .copied()
            .or_else(|| MaterialParams::builtin(key))
            .unwrap_or_default()
    }

    /// The texture layers of `key`'s rows.
    pub fn layers(&self, key: &str) -> MaterialLayers {
        self.layers.get(key).copied().unwrap_or_default()
    }

    /// Fills the rows from the scene's material keys (row `i + 1` from `keys[i]`) when the scene has new keys
    /// (`revision`) or a key's parameters changed. Rows changed with [`MaterialTable::set`] since are overwritten
    /// only then.
    pub fn fill_from_keys(&mut self, keys: &[Arc<str>], revision: u64) {
        if !self.stale && self.filled == Some((revision, keys.len())) {
            return;
        }
        self.filled = Some((revision, keys.len()));
        self.stale = false;
        let rows: Vec<MaterialRow> = keys
            .iter()
            .take(MAX_MATERIAL_ROWS - 1)
            .map(|key| self.resolve(key).row(self.layers(key)))
            .collect();
        for (row, made) in self.rows.iter_mut().skip(1).zip(rows) {
            *row = made;
        }
        self.dirty = true;
    }

    /// Replaces row `row`. Returns false (and changes nothing) when the table has no such row.
    pub fn set(&mut self, row: usize, params: MaterialParams) -> bool {
        let Some(target) = self.rows.get_mut(row) else {
            return false;
        };
        *target = params.row(MaterialLayers::NONE);
        self.dirty = true;
        true
    }

    /// Row `row`'s parameters, as the shader reads them (without its texture mapping).
    pub fn get(&self, row: usize) -> Option<MaterialParams> {
        self.rows.get(row).map(|r| {
            let emissive = r.emission[0].max(r.emission[1]).max(r.emission[2]);
            let emission = if emissive > 0.0 {
                [
                    r.emission[0] / emissive,
                    r.emission[1] / emissive,
                    r.emission[2] / emissive,
                ]
            } else {
                [1.0; 3]
            };
            MaterialParams {
                roughness: r.surface[0],
                metalness: r.surface[1],
                emissive,
                glass: r.surface[2].to_bits() & GLASS != 0,
                tint: [r.tint[0], r.tint[1], r.tint[2]],
                emission,
                normal_strength: r.surface[3],
                ..MaterialParams::PLASTIC
            }
        })
    }

    /// The uniform buffer.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Uploads the table if a row changed since the last upload.
    pub(crate) fn upload(&mut self, queue: &wgpu::Queue) {
        if self.dirty {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.rows));
            self.dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_material_has_parameters_in_range() {
        for name in [
            "Plastic",
            "SmoothPlastic",
            "Metal",
            "Wood",
            "Glass",
            "Neon",
            "Concrete",
            "Grass",
            "Marble",
        ] {
            let params = MaterialParams::builtin(name).unwrap();
            assert!((0.0..=1.0).contains(&params.roughness), "{name}");
            assert!((0.0..=1.0).contains(&params.metalness), "{name}");
            assert!(params.emissive >= 0.0, "{name}");
        }
        assert!(MaterialParams::builtin("Glass").unwrap().glass);
        assert!(MaterialParams::builtin("Neon").unwrap().emissive > 0.0);
        assert_eq!(MaterialParams::builtin("Metal").unwrap().metalness, 1.0);
        assert_eq!(MaterialParams::builtin("Unobtainium"), None);
    }

    #[test]
    fn a_row_is_80_bytes_and_the_table_fits_a_uniform_binding() {
        assert_eq!(size_of::<MaterialRow>(), 80);
        // WebGPU guarantees 16 KiB of uniform binding, GLES included.
        const { assert!(MATERIAL_TABLE_SIZE <= 16 * 1024) };
        let glass = MaterialParams::builtin("Glass").unwrap().row(MaterialLayers::NONE);
        assert_eq!(glass.surface[2].to_bits(), GLASS);
        let neon = MaterialParams::builtin("Neon").unwrap().row(MaterialLayers::NONE);
        assert_eq!(&neon.emission[..3], &[1.0, 1.0, 1.0]);
    }

    #[test]
    fn layers_and_tiling_reach_the_row() {
        let params = MaterialParams {
            tile_size: 2.0,
            uv_scale: [2.0, 1.0],
            uv_offset: [0.5, 0.0],
            nearest: true,
            wrap: MaterialWrap::Mirror,
            ..MaterialParams::PLASTIC
        };
        let layers = MaterialLayers {
            color: 3,
            normal: MaterialLayers::NONE.normal,
            orm: 1,
            emission: 7,
        };
        let row = params.row(layers);
        assert_eq!(row.surface[2].to_bits(), TEXTURED | NEAREST | MIRROR);
        assert_eq!(row.tint[3].to_bits(), 3 | (0xffff << 16));
        assert_eq!(row.emission[3].to_bits(), 1 | (7 << 16));
        assert_eq!(row.uv, [1.0, 0.5, 0.5, 0.0], "per metre");
        assert_eq!(row.mesh_uv, [2.0, 1.0, 0.5, 0.0]);
        let plain = MaterialParams::PLASTIC.row(MaterialLayers::NONE);
        assert_eq!(plain.surface[2].to_bits(), 0, "nothing to sample");
    }
}
