//! Per-material shading parameters. Without a texture set ([`crate::textures`]) surface detail is
//! generated in the shader, so no texture files are needed.

/// Shading parameters of one material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialParams {
    /// Material name, equal to an entry of `metallic_core::classes::MATERIALS`.
    pub name: &'static str,
    /// Microfacet roughness, 0 (mirror) to 1 (fully diffuse).
    pub roughness: f32,
    /// 0 for dielectrics, 1 for bare metal.
    pub metalness: f32,
    /// The shader's procedural detail pattern (an index of the first ten materials): the look
    /// without textures, and the fallback when a texture set has no layer for the material.
    pub detail: u32,
    /// Studs one texture repeat covers (triplanar, in the part's own studs), or 0 for a
    /// material that stays untextured (Plastic, SmoothPlastic and Glass keep their procedural
    /// look in every texture set).
    pub tile_studs: f32,
}

const fn m(name: &'static str, roughness: f32, metalness: f32, detail: u32, tile: f32) -> MaterialParams {
    MaterialParams {
        name,
        roughness,
        metalness,
        detail,
        tile_studs: tile,
    }
}

/// One entry per registry material, in registry order (the index is also the instance's
/// material id).
pub const MATERIAL_TABLE: [MaterialParams; 22] = [
    m("Plastic", 0.55, 0.0, 0, 0.0),
    m("SmoothPlastic", 0.28, 0.0, 1, 0.0),
    m("Metal", 0.34, 1.0, 2, 4.0),
    m("Wood", 0.72, 0.0, 3, 6.0),
    m("Concrete", 0.92, 0.0, 4, 8.0),
    m("Glass", 0.06, 0.0, 5, 0.0),
    m("Grass", 0.95, 0.0, 6, 8.0),
    m("Brick", 0.85, 0.0, 7, 6.4),
    m("Ice", 0.12, 0.0, 8, 8.0),
    m("Sand", 0.98, 0.0, 9, 8.0),
    m("WoodPlanks", 0.7, 0.0, 3, 8.0),
    m("Slate", 0.8, 0.0, 4, 8.0),
    m("Cobblestone", 0.88, 0.0, 4, 6.0),
    m("Marble", 0.22, 0.0, 1, 10.0),
    m("Granite", 0.6, 0.0, 4, 6.0),
    m("Pebble", 0.85, 0.0, 9, 5.0),
    m("Fabric", 0.9, 0.0, 0, 2.0),
    m("DiamondPlate", 0.38, 1.0, 2, 3.0),
    m("CorrodedMetal", 0.7, 1.0, 2, 6.0),
    m("Snow", 0.8, 0.0, 9, 10.0),
    m("Rock", 0.9, 0.0, 4, 10.0),
    m("Asphalt", 0.9, 0.0, 4, 8.0),
];

/// Index of a material in [`MATERIAL_TABLE`]; unknown names fall back to `Plastic` (index 0).
pub fn material_index(name: &str) -> usize {
    MATERIAL_TABLE
        .iter()
        .position(|m| m.name == name)
        .unwrap_or(0)
}

/// Parameters for a material name, `Plastic` if unknown.
pub fn material_params(name: &str) -> &'static MaterialParams {
    &MATERIAL_TABLE[material_index(name)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::classes::MATERIALS;

    #[test]
    fn material_table_has_every_registry_material() {
        assert_eq!(MATERIAL_TABLE.len(), MATERIALS.len());
        for (i, name) in MATERIALS.iter().enumerate() {
            assert_eq!(
                MATERIAL_TABLE[i].name, *name,
                "registry order differs at {i}"
            );
            assert_eq!(material_index(name), i);
        }
    }

    #[test]
    fn parameters_are_in_range() {
        for m in &MATERIAL_TABLE {
            assert!((0.0..=1.0).contains(&m.roughness));
            assert!((0.0..=1.0).contains(&m.metalness));
        }
    }

    #[test]
    fn detail_patterns_exist_and_the_classic_materials_keep_their_own() {
        for (i, m) in MATERIAL_TABLE.iter().enumerate() {
            assert!(m.detail < 10, "{}", m.name);
            if i < 10 {
                assert_eq!(m.detail, i as u32, "{}", m.name);
            }
        }
        for name in ["Plastic", "SmoothPlastic", "Glass"] {
            assert_eq!(material_params(name).tile_studs, 0.0, "{name} stays untextured");
        }
    }

    #[test]
    fn unknown_material_falls_back_to_plastic() {
        assert_eq!(material_params("Unobtainium").name, "Plastic");
    }
}
