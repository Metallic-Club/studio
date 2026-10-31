//! Shading models and the GPU material table (#63).
//!
//! Every part is drawn by the same surface shader (`scene.wgsl`), which switches on the shading
//! model stored in its material table row. A scene that mixes Lit, Toon, Unlit and Flat parts
//! therefore needs no extra pipelines.
//!
//! Rows are keyed by (style slot, row kind): each style slot has [`ROWS_PER_STYLE`] rows, one per
//! `Material` (registry order) and then the class rows ([`CHARACTER_ROW`]). Slot 0 is the place
//! Look; per-part styles (#223) add slots. Each instance carries its row index, so restyling
//! a part changes one `u32`, not its pipeline (building-tools spec 12.4, lead ruling R14).
use crate::materials::{MATERIAL_TABLE, MaterialParams};
use bytemuck::{Pod, Zeroable};

/// Styles the table holds at once (the place Look plus per-part styles).
pub const MAX_STYLES: usize = 8;
/// Rows per style: the 22 materials, then the Character class row and one reserved row.
pub const ROWS_PER_STYLE: usize = 24;
/// All rows (192 x 80 B = 15 KB, one uniform buffer under the 16 KB every device allows).
pub const MAX_ROWS: usize = MAX_STYLES * ROWS_PER_STYLE;
/// Offset of the Character class row inside a style's rows (skinned characters drawn by the
/// standard surface shader use it).
pub const CHARACTER_ROW: usize = MATERIAL_TABLE.len();
// A new material takes the reserved row; one more needs a larger ROWS_PER_STYLE.
const _: () = assert!(CHARACTER_ROW < ROWS_PER_STYLE);

/// The lighting function a row uses. The discriminant is the id the shader switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShadingModel {
    /// GGX specular and Lambert diffuse with sky light and reflections (physically based).
    #[default]
    Lit = 0,
    /// Banded sun light with a tinted shade side, flattened ambient, a highlight blob and an
    /// optional rim.
    Toon = 1,
    /// Albedo shown as its own colour: no sun, no shade.
    Unlit = 2,
    /// One normal per triangle from screen derivatives, wrapped Lambert and sky light.
    Flat = 3,
}

impl ShadingModel {
    /// The id the shader switches on.
    pub fn id(self) -> u32 {
        self as u32
    }
}

/// Parameters of one shading model (look spec `docs/design/look-presets.md` 3.1). Fields a
/// model does not use are ignored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelParams {
    /// Which lighting function.
    pub model: ShadingModel,
    /// Lit and Flat: wrapped diffuse, 0..0.5 (bends the terminator). Lit takes the larger of
    /// this and `LightingStyle.Soft`'s wrap.
    pub wrap: f32,
    /// Lit: every material's roughness is raised to at least this.
    pub roughness_floor: f32,
    /// Toon: lit steps, 1..8.
    pub bands: f32,
    /// Toon: band edge width in N.L units.
    pub softness: f32,
    /// Toon: multiplier on the shade side, sRGB 0..255.
    pub shadow_tint: [f32; 3],
    /// Toon: 0..1, how much ambient light is mixed toward the sky and ground average.
    pub flatten: f32,
    /// Toon: highlight size (0 turns it off) and strength.
    pub spec_size: f32,
    /// Toon: highlight strength.
    pub spec_strength: f32,
    /// Toon: rim width, 0 turns the rim off.
    pub rim_width: f32,
    /// Toon: rim colour, sRGB 0..255.
    pub rim_color: [f32; 3],
    /// Toon: quantise incoming light in steps of this many EV (0 = off; Pixel 3D uses 1).
    pub light_step_ev: f32,
}

impl ModelParams {
    /// Realistic: Lit, no wrap, no roughness floor (the renderer's original shading).
    pub const LIT: ModelParams = ModelParams {
        model: ShadingModel::Lit,
        wrap: 0.0,
        roughness_floor: 0.0,
        bands: 1.0,
        softness: 0.0,
        shadow_tint: [255.0; 3],
        flatten: 0.0,
        spec_size: 0.0,
        spec_strength: 0.0,
        rim_width: 0.0,
        rim_color: [255.0; 3],
        light_step_ev: 0.0,
    };

    /// The Toon Look's default model: bands 2, softness 0.03, shadow tint (230, 220, 255),
    /// flatten 0.85, highlight 0.08 / 0.5.
    pub const TOON: ModelParams = ModelParams {
        model: ShadingModel::Toon,
        bands: 2.0,
        softness: 0.03,
        shadow_tint: [230.0, 220.0, 255.0],
        flatten: 0.85,
        spec_size: 0.08,
        spec_strength: 0.5,
        ..ModelParams::LIT
    };

    /// Low-poly's Flat model (wrap 0.1).
    pub const FLAT: ModelParams = ModelParams {
        model: ShadingModel::Flat,
        wrap: 0.1,
        ..ModelParams::LIT
    };

    /// Unlit.
    pub const UNLIT: ModelParams = ModelParams {
        model: ShadingModel::Unlit,
        ..ModelParams::LIT
    };
}

impl Default for ModelParams {
    fn default() -> ModelParams {
        ModelParams::LIT
    }
}

/// How one style shades: a default model, per-material overrides and the Character class row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shading {
    /// Model of every material without an override.
    pub default: ModelParams,
    /// Overrides by material name (`"Metal"`, `"Glass"` ...); unknown names are ignored.
    pub materials: Vec<(String, ModelParams)>,
    /// Skinned characters drawn with the standard surface shader; `None` uses `default`.
    pub character: Option<ModelParams>,
}

impl Shading {
    /// One model for everything.
    pub fn uniform(model: ModelParams) -> Shading {
        Shading {
            default: model,
            ..Shading::default()
        }
    }

    fn for_material(&self, name: &str) -> ModelParams {
        self.materials
            .iter()
            .find(|(m, _)| m == name)
            .map_or(self.default, |(_, p)| *p)
    }
}

/// Where a material's textures are (#146): a layer of the texture arrays of one size class.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureRef {
    /// Layer in the size class's detail, normal and ORM arrays.
    pub layer: u32,
    /// Size class (index into [`crate::textures::SIZE_CLASSES`]).
    pub class: u32,
    /// Nearest filtering (pixel sets).
    pub nearest: bool,
    /// Studs one repeat covers.
    pub tile_studs: f32,
    /// Normal map strength, 0..1.
    pub normal_strength: f32,
}

impl TextureRef {
    /// The row's `texture` field: layer, repeats per stud, normal strength, class + 4 x nearest.
    /// Layer -1 means untextured (procedural detail).
    fn pack(this: Option<&TextureRef>) -> [f32; 4] {
        match this {
            Some(t) => [
                t.layer as f32,
                1.0 / t.tile_studs.max(0.01),
                t.normal_strength.clamp(0.0, 1.0),
                (t.class + if t.nearest { 4 } else { 0 }) as f32,
            ],
            None => [-1.0, 0.0, 0.0, 0.0],
        }
    }
}

/// One row of the GPU material table (80 bytes, `MaterialRow` in `scene.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct MaterialRow {
    /// Model id, roughness (after the floor), metalness, wrap.
    pub surface: [f32; 4],
    /// Toon: bands, softness, flatten, light step in EV.
    pub toon: [f32; 4],
    /// Toon: shadow tint (linear), highlight size.
    pub tint: [f32; 4],
    /// Toon: highlight strength, rim width, rim colour packed as 0xRRGGBB (exact in an f32);
    /// all models: the roughness floor (skinned slots, which bring their own roughness).
    pub extra: [f32; 4],
    /// Textures (#146): layer (-1 = procedural detail), repeats per stud, normal strength,
    /// size class + 4 when filtered nearest.
    pub texture: [f32; 4],
}

fn srgb8_to_linear(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| crate::scene::srgb_to_linear(v / 255.0))
}

impl MaterialRow {
    /// The row for a material shaded with `p`.
    pub fn new(p: &ModelParams, material: &MaterialParams) -> MaterialRow {
        let tint = srgb8_to_linear(p.shadow_tint);
        let rgb = p.rim_color.map(|v| v.clamp(0.0, 255.0).round() as u32);
        let packed = (rgb[0] << 16) | (rgb[1] << 8) | rgb[2];
        MaterialRow {
            surface: [
                p.model.id() as f32,
                material.roughness.max(p.roughness_floor),
                material.metalness,
                p.wrap.clamp(0.0, 0.5),
            ],
            toon: [
                p.bands.clamp(1.0, 8.0),
                p.softness.clamp(0.0, 0.5),
                p.flatten.clamp(0.0, 1.0),
                p.light_step_ev.max(0.0),
            ],
            tint: [tint[0], tint[1], tint[2], p.spec_size.clamp(0.0, 1.0)],
            extra: [
                p.spec_strength.max(0.0),
                p.rim_width.clamp(0.0, 1.0),
                packed as f32,
                p.roughness_floor.clamp(0.0, 1.0),
            ],
            texture: TextureRef::pack(None),
        }
    }
}

/// The CPU copy of the GPU material table.
#[derive(Debug, Clone)]
pub struct MaterialTable {
    rows: Box<[MaterialRow; MAX_ROWS]>,
    styles: [Option<Shading>; MAX_STYLES],
    textures: [Option<TextureRef>; MATERIAL_TABLE.len()],
    dirty: bool,
}

impl Default for MaterialTable {
    fn default() -> MaterialTable {
        MaterialTable::new(&Shading::default())
    }
}

impl MaterialTable {
    /// A table whose slot 0 (the place Look) is `place`.
    pub fn new(place: &Shading) -> MaterialTable {
        let mut t = MaterialTable {
            rows: Box::new([MaterialRow::zeroed(); MAX_ROWS]),
            styles: Default::default(),
            textures: [None; MATERIAL_TABLE.len()],
            dirty: true,
        };
        t.set_style(0, place);
        t
    }

    /// Sets (or replaces) the shading of a style slot. Rows change only when the shading does.
    /// A slot of [`MAX_STYLES`] or more is ignored with a warning (it has no rows), and the
    /// call returns false; otherwise it returns true.
    pub fn set_style(&mut self, slot: usize, shading: &Shading) -> bool {
        if slot >= MAX_STYLES {
            log::warn!("material table: style slot {slot} ignored (slots are 0..{MAX_STYLES})");
            return false;
        }
        if self.styles[slot].as_ref() == Some(shading) {
            return true;
        }
        self.write_style(slot, shading);
        self.styles[slot] = Some(shading.clone());
        self.dirty = true;
        true
    }

    fn write_style(&mut self, slot: usize, shading: &Shading) {
        let base = slot * ROWS_PER_STYLE;
        for (i, m) in MATERIAL_TABLE.iter().enumerate() {
            let mut row = MaterialRow::new(&shading.for_material(m.name), m);
            row.texture = TextureRef::pack(self.textures[i].as_ref());
            self.rows[base + i] = row;
        }
        // Character: Plastic's surface (skinned slots bring their own roughness and metalness).
        let character = shading.character.unwrap_or(shading.default);
        self.rows[base + CHARACTER_ROW] = MaterialRow::new(&character, &MATERIAL_TABLE[0]);
    }

    /// Sets each material's textures (registry order; `None` keeps procedural detail) in every
    /// style slot. Rows change only when the references do.
    pub fn set_textures(&mut self, textures: &[Option<TextureRef>; MATERIAL_TABLE.len()]) {
        if &self.textures == textures {
            return;
        }
        self.textures = *textures;
        let styles = self.styles.clone();
        for (slot, shading) in styles.iter().enumerate() {
            if let Some(shading) = shading {
                self.write_style(slot, shading);
            }
        }
        self.dirty = true;
    }

    /// The texture of a material (registry index), if any.
    pub fn texture(&self, material: usize) -> Option<&TextureRef> {
        self.textures.get(material).and_then(Option::as_ref)
    }

    /// The shading of a style slot, if set.
    pub fn style(&self, slot: usize) -> Option<&Shading> {
        self.styles.get(slot).and_then(Option::as_ref)
    }

    /// Row index of a material (registry index) in a style slot.
    pub fn row(slot: usize, material: usize) -> u32 {
        (slot.min(MAX_STYLES - 1) * ROWS_PER_STYLE + material.min(ROWS_PER_STYLE - 1)) as u32
    }

    /// All rows, for upload.
    pub fn rows(&self) -> &[MaterialRow; MAX_ROWS] {
        &self.rows
    }

    /// True once after any change (the renderer then uploads the table).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_80_bytes_and_the_table_fits_a_uniform_buffer() {
        assert_eq!(std::mem::size_of::<MaterialRow>(), 80);
        assert_eq!(MAX_ROWS * 80, 15360);
        assert!(MAX_ROWS * 80 <= 16384, "the WebGPU minimum uniform binding size");
    }

    #[test]
    fn textures_fill_every_style_and_untextured_rows_say_procedural() {
        let mut t = MaterialTable::default();
        t.set_style(3, &Shading::uniform(ModelParams::TOON));
        assert!(t.take_dirty());
        let mut refs = [None; MATERIAL_TABLE.len()];
        refs[7] = Some(TextureRef {
            layer: 2,
            class: 1,
            nearest: true,
            tile_studs: 4.0,
            normal_strength: 0.5,
        });
        t.set_textures(&refs);
        assert!(t.take_dirty());
        t.set_textures(&refs);
        assert!(!t.take_dirty(), "the same textures upload nothing");
        for slot in [0, 3] {
            let brick = t.rows()[MaterialTable::row(slot, 7) as usize];
            assert_eq!(brick.texture, [2.0, 0.25, 0.5, 5.0], "slot {slot}");
            let plastic = t.rows()[MaterialTable::row(slot, 0) as usize];
            assert_eq!(plastic.texture[0], -1.0);
        }
        assert_eq!(t.rows()[MaterialTable::row(3, 7) as usize].surface[0], 1.0, "still toon");
        // A style set later gets the textures too.
        t.set_style(5, &Shading::default());
        assert_eq!(t.rows()[MaterialTable::row(5, 7) as usize].texture[0], 2.0);
    }

    #[test]
    fn the_shader_agrees_on_the_table_shape() {
        let wgsl = include_str!("../shaders/scene.wgsl");
        assert!(wgsl.contains(&format!("array<MaterialRow, {MAX_ROWS}>")));
        assert!(wgsl.contains(&format!("const CHARACTER_ROW: u32 = {CHARACTER_ROW}u;")));
        for m in [
            ShadingModel::Lit,
            ShadingModel::Toon,
            ShadingModel::Unlit,
            ShadingModel::Flat,
        ] {
            let name = format!("{m:?}").to_uppercase();
            assert!(
                wgsl.contains(&format!("const MODEL_{name}: u32 = {}u;", m.id())),
                "{name}"
            );
        }
    }

    #[test]
    fn slots_past_the_table_are_ignored() {
        let mut t = MaterialTable::default();
        assert!(t.take_dirty());
        let before = *t.rows();
        assert!(!t.set_style(MAX_STYLES, &Shading::uniform(ModelParams::UNLIT)));
        assert!(!t.set_style(usize::MAX, &Shading::uniform(ModelParams::UNLIT)));
        assert!(!t.take_dirty(), "nothing to upload");
        assert_eq!(*t.rows(), before);
        assert!(t.style(MAX_STYLES).is_none());
        assert!(t.set_style(MAX_STYLES - 1, &Shading::uniform(ModelParams::UNLIT)));
    }

    #[test]
    fn realistic_rows_keep_the_material_surface() {
        let t = MaterialTable::default();
        for (i, m) in MATERIAL_TABLE.iter().enumerate() {
            let r = t.rows()[MaterialTable::row(0, i) as usize];
            assert_eq!(
                r.surface,
                [0.0, m.roughness, m.metalness, 0.0],
                "{}",
                m.name
            );
        }
    }

    #[test]
    fn overrides_floors_and_slots() {
        let mut soft = Shading::uniform(ModelParams {
            wrap: 0.3,
            roughness_floor: 0.5,
            ..ModelParams::LIT
        });
        soft.materials.push(("Glass".into(), ModelParams::LIT));
        let mut t = MaterialTable::default();
        assert!(t.take_dirty());
        t.set_style(1, &soft);
        assert!(t.take_dirty());
        t.set_style(1, &soft);
        assert!(!t.take_dirty(), "the same shading again uploads nothing");
        let metal = t.rows()[MaterialTable::row(1, 2) as usize];
        assert_eq!(metal.surface[1], 0.5, "roughness floor");
        assert_eq!(metal.surface[3], 0.3, "wrap");
        let glass = t.rows()[MaterialTable::row(1, 5) as usize];
        assert_eq!(
            glass.surface[1], 0.06,
            "Glass override keeps its own roughness"
        );
        assert_eq!(glass.surface[3], 0.0);
        let toon = MaterialRow::new(&ModelParams::TOON, &MATERIAL_TABLE[0]);
        assert_eq!(toon.surface[0], 1.0);
        assert_eq!(toon.extra[2] as u32, 0xFFFFFF, "rim colour packs exactly");
    }
}
