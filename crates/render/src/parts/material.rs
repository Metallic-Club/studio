//! The material table: the shading parameters of every material row, indexed by the instance's material row.
//!
//! Rows are data, not code (#678, ADR 0012). The scene numbers every material key its parts name, an asset id or a
//! name, from row 1 ([`crate::InstanceScene::material_keys`]); row 0 is the default look. Each row is filled from
//! the parameters given for its key ([`MaterialTable::set_material`]: the engine gives a material asset's), else from
//! the built-in material of that name ([`MaterialParams::builtin`]), else plastic. A key's parameters can change at
//! any time, and every part using it changes with them. The table is a uniform buffer (readable on every backend,
//! GLES included) of [`MAX_MATERIAL_ROWS`] rows, 64 bytes each, uploaded only when a row changes.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};

/// How many rows the table holds. Rows beyond it shade as row 0. The instance format has room for 65,536.
pub const MAX_MATERIAL_ROWS: usize = 256;

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
}

impl MaterialParams {
    /// Plain plastic: the default, and what unknown materials shade as.
    pub const PLASTIC: Self = Self {
        roughness: 0.6,
        metalness: 0.0,
        emissive: 0.0,
        glass: false,
        tint: [1.0; 3],
        emission: [1.0; 3],
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

    fn row(self) -> MaterialRow {
        let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let emissive = if self.emissive.is_finite() { self.emissive.max(0.0) } else { 0.0 };
        MaterialRow {
            // NaN stays NaN in roughness and metalness: a broken material should show, not be hidden (the post
            // tests poison a row on purpose).
            surface: [
                self.roughness.clamp(0.0, 1.0),
                self.metalness.clamp(0.0, 1.0),
                if self.glass { 1.0 } else { 0.0 },
                1.0,
            ],
            tint: [unit(self.tint[0]), unit(self.tint[1]), unit(self.tint[2]), 0.0],
            emission: [
                unit(self.emission[0]) * emissive,
                unit(self.emission[1]) * emissive,
                unit(self.emission[2]) * emissive,
                0.0,
            ],
            uv: [1.0, 1.0, 0.0, 0.0],
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
    /// Roughness, metalness, glass flag, normal strength.
    surface: [f32; 4],
    /// The base colour's tint (linear); w is for the texture layers (#679).
    tint: [f32; 4],
    /// The emitted light (linear, times its strength); w is for the texture layers (#679).
    emission: [f32; 4],
    /// Texture tiling: scale per metre along U and V, then the offset (#679).
    uv: [f32; 4],
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
            rows: vec![MaterialParams::PLASTIC.row(); MAX_MATERIAL_ROWS],
            buffer,
            given: HashMap::new(),
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

    /// Forgets the parameters given for `key`: its rows go back to the built-in material of that name, or plastic.
    pub fn forget_material(&mut self, key: &str) {
        if self.given.remove(key).is_some() {
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

    /// Fills the rows from the scene's material keys (row `i + 1` from `keys[i]`) when the scene has new keys
    /// (`revision`) or a key's parameters changed. Rows changed with [`MaterialTable::set`] since are overwritten
    /// only then.
    pub fn fill_from_keys(&mut self, keys: &[Arc<str>], revision: u64) {
        if !self.stale && self.filled == Some((revision, keys.len())) {
            return;
        }
        self.filled = Some((revision, keys.len()));
        self.stale = false;
        for (row, key) in self.rows.iter_mut().skip(1).zip(keys) {
            let params = self
                .given
                .get(key)
                .copied()
                .or_else(|| MaterialParams::builtin(key))
                .unwrap_or_default();
            *row = params.row();
        }
        self.dirty = true;
    }

    /// Replaces row `row`. Returns false (and changes nothing) when the table has no such row.
    pub fn set(&mut self, row: usize, params: MaterialParams) -> bool {
        let Some(target) = self.rows.get_mut(row) else {
            return false;
        };
        *target = params.row();
        self.dirty = true;
        true
    }

    /// Row `row`'s parameters, as the shader reads them.
    pub fn get(&self, row: usize) -> Option<MaterialParams> {
        self.rows.get(row).map(|r| {
            let emissive = r.emission[0].max(r.emission[1]).max(r.emission[2]);
            let emission = if emissive > 0.0 {
                [r.emission[0] / emissive, r.emission[1] / emissive, r.emission[2] / emissive]
            } else {
                [1.0; 3]
            };
            MaterialParams {
                roughness: r.surface[0],
                metalness: r.surface[1],
                emissive,
                glass: r.surface[2] > 0.5,
                tint: [r.tint[0], r.tint[1], r.tint[2]],
                emission,
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
        for name in ["Plastic", "SmoothPlastic", "Metal", "Wood", "Glass", "Neon", "Concrete", "Grass", "Marble"] {
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
    fn a_row_is_64_bytes_and_the_table_fits_a_uniform_binding() {
        assert_eq!(size_of::<MaterialRow>(), 64);
        // WebGPU guarantees 16 KiB of uniform binding, GLES included.
        const { assert!(MATERIAL_TABLE_SIZE <= 16 * 1024) };
        let row = MaterialParams::builtin("Glass").unwrap().row();
        assert_eq!(row.surface, [0.05, 0.0, 1.0, 1.0]);
        let neon = MaterialParams::builtin("Neon").unwrap().row();
        assert_eq!(neon.emission, [1.0, 1.0, 1.0, 0.0]);
    }
}
