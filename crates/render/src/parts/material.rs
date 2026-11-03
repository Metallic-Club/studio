//! The material table: the shading parameters of every material row, indexed by the instance's material row.
//!
//! Rows are data, not code: the built-in `Material` enum items get their parameters by name
//! ([`MaterialParams::builtin`]), in the order the scene numbers them ([`crate::InstanceScene::material_names`]),
//! and any row can be replaced ([`MaterialTable::set`]), so custom materials only add rows. The table is a uniform
//! buffer (readable on every backend, GLES included) of [`MAX_MATERIAL_ROWS`] rows, 32 bytes each, uploaded only
//! when a row changes.

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
    /// Light emitted, as a multiple of the part's colour (Neon glows with 1).
    pub emissive: f32,
    /// Glass: when partly see-through, its reflections stay at full strength while the body fades.
    pub glass: bool,
}

impl MaterialParams {
    /// Plain plastic: the default, and what unknown materials shade as.
    pub const PLASTIC: Self = Self {
        roughness: 0.6,
        metalness: 0.0,
        emissive: 0.0,
        glass: false,
    };

    /// The parameters of a built-in `Material` item, by name; `None` for names it does not know.
    pub fn builtin(name: &str) -> Option<Self> {
        let (roughness, metalness, emissive, glass) = match name {
            "Plastic" => return Some(Self::PLASTIC),
            "SmoothPlastic" => (0.3, 0.0, 0.0, false),
            "Metal" => (0.35, 1.0, 0.0, false),
            "Wood" => (0.75, 0.0, 0.0, false),
            "Glass" => (0.05, 0.0, 0.0, true),
            "Neon" => (1.0, 0.0, 1.0, false),
            "Concrete" => (0.9, 0.0, 0.0, false),
            "Grass" => (0.95, 0.0, 0.0, false),
            _ => return None,
        };
        Some(Self {
            roughness,
            metalness,
            emissive,
            glass,
        })
    }

    fn row(self) -> MaterialRow {
        MaterialRow {
            surface: [
                self.roughness.clamp(0.0, 1.0),
                self.metalness.clamp(0.0, 1.0),
                self.emissive.max(0.0),
                if self.glass { 1.0 } else { 0.0 },
            ],
            reserved: [0.0; 4],
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
    /// Roughness, metalness, emissive, glass flag.
    surface: [f32; 4],
    /// Room for custom materials' parameters.
    reserved: [f32; 4],
}

/// Size of the table in bytes.
pub(crate) const MATERIAL_TABLE_SIZE: u64 = (MAX_MATERIAL_ROWS * size_of::<MaterialRow>()) as u64;

/// Every material row's parameters, and the uniform buffer the shaders read them from.
#[derive(Debug)]
pub struct MaterialTable {
    rows: Vec<MaterialRow>,
    buffer: wgpu::Buffer,
    /// The names the rows were last filled from, to notice a scene with another `Material` enum.
    names: &'static [&'static str],
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
            names: &[],
            dirty: true,
        }
    }

    /// Fills the rows from material names, row `i` from `names[i]` ([`MaterialParams::builtin`], plastic when
    /// unknown), unless the table was already filled from these names: rows changed with [`MaterialTable::set`]
    /// stay as they are.
    pub fn fill_from_names(&mut self, names: &'static [&'static str]) {
        if std::ptr::eq(self.names, names) {
            return;
        }
        self.names = names;
        for (row, name) in self.rows.iter_mut().zip(names) {
            *row = MaterialParams::builtin(name).unwrap_or_default().row();
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
        self.rows.get(row).map(|r| MaterialParams {
            roughness: r.surface[0],
            metalness: r.surface[1],
            emissive: r.surface[2],
            glass: r.surface[3] > 0.5,
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

    const MATERIALS: [&str; 8] = [
        "Plastic",
        "SmoothPlastic",
        "Metal",
        "Wood",
        "Glass",
        "Neon",
        "Concrete",
        "Grass",
    ];

    #[test]
    fn every_builtin_material_has_parameters_in_range() {
        for name in MATERIALS {
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
    fn a_row_is_32_bytes_and_the_table_fits_a_uniform_binding() {
        assert_eq!(size_of::<MaterialRow>(), 32);
        // WebGPU guarantees 16 KiB of uniform binding, GLES included.
        const { assert!(MATERIAL_TABLE_SIZE <= 16 * 1024) };
        let row = MaterialParams::builtin("Glass").unwrap().row();
        assert_eq!(row.surface, [0.05, 0.0, 0.0, 1.0]);
    }
}
