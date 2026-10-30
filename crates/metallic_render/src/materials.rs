//! Per-material shading parameters. Surface detail is generated in the shader, so no texture files are needed.

/// Shading parameters of one material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialParams {
    /// Material name, equal to an entry of `metallic_core::classes::MATERIALS`.
    pub name: &'static str,
    /// Microfacet roughness, 0 (mirror) to 1 (fully diffuse).
    pub roughness: f32,
    /// 0 for dielectrics, 1 for bare metal.
    pub metalness: f32,
}

/// One entry per registry material, in registry order (the index is also the shader's material id).
pub const MATERIAL_TABLE: [MaterialParams; 10] = [
    MaterialParams {
        name: "Plastic",
        roughness: 0.55,
        metalness: 0.0,
    },
    MaterialParams {
        name: "SmoothPlastic",
        roughness: 0.28,
        metalness: 0.0,
    },
    MaterialParams {
        name: "Metal",
        roughness: 0.34,
        metalness: 1.0,
    },
    MaterialParams {
        name: "Wood",
        roughness: 0.72,
        metalness: 0.0,
    },
    MaterialParams {
        name: "Concrete",
        roughness: 0.92,
        metalness: 0.0,
    },
    MaterialParams {
        name: "Glass",
        roughness: 0.06,
        metalness: 0.0,
    },
    MaterialParams {
        name: "Grass",
        roughness: 0.95,
        metalness: 0.0,
    },
    MaterialParams {
        name: "Brick",
        roughness: 0.85,
        metalness: 0.0,
    },
    MaterialParams {
        name: "Ice",
        roughness: 0.12,
        metalness: 0.0,
    },
    MaterialParams {
        name: "Sand",
        roughness: 0.98,
        metalness: 0.0,
    },
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
    fn unknown_material_falls_back_to_plastic() {
        assert_eq!(material_params("Unobtainium").name, "Plastic");
    }
}
