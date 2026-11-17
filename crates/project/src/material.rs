//! Material assets (#678, ADR 0012): a material is a file of its own (`Assets/Materials/Wood.material`), which any
//! part can use and anyone can edit, replace or delete. There is no fixed list of materials and no variants: the
//! familiar ones ([`builtin`]) are ordinary files a new project starts with.
//!
//! # The file
//!
//! Versioned JSON, with every value at its default left out, so a file says only what makes the material what it is:
//!
//! ```json
//! {
//!   "format": 1,
//!   "roughness": 0.75,
//!   "textures": {"color": "9f3c0a5e1b7d4c2a8e6f0b1d3a5c7e9f"}
//! }
//! ```
//!
//! - `color` (sRGB, multiplied by the part's colour), `roughness`, `metallic`;
//! - `emission` (sRGB, multiplied by the part's colour) and `emission_strength`: Neon glows with strength 1;
//! - `glass`: when the part is see-through, its reflections stay at full strength while its body fades;
//! - `normal_strength`, `tile_size` (metres a texture covers), `uv_scale`, `uv_offset`, `filter`, `wrap`;
//! - `textures`: a picture per channel (`color`, `normal`, `roughness`, `metallic`, `emission`, `occlusion`) by
//!   asset id, and `packed`: one picture holding ambient occlusion, roughness and metallic in its channels.
//!
//! A key this engine does not know is kept out and reported as a warning, never an error, as scenes do; a newer
//! format is refused.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::asset_index::AssetId;

/// The file name extension of a material.
pub const MATERIAL_EXTENSION: &str = "material";

/// The `format` version this engine writes and reads.
pub const MATERIAL_FORMAT: u32 = 1;

/// The folder the template keeps its materials in, and Studio makes new ones in.
pub const MATERIALS_FOLDER: &str = "Assets/Materials";

/// How a texture is sampled between its pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filter {
    /// As its pictures are (#781): pixelated (nearest) when the first picture it uses is, else smooth.
    #[default]
    Picture,
    /// Smoothly, between the four nearest pixels and between mip levels.
    Linear,
    /// The nearest pixel: crisp, for pixel art.
    Nearest,
}

/// What a texture does past its edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wrap {
    /// It repeats.
    #[default]
    Repeat,
    /// It repeats mirrored, so its edges meet.
    Mirror,
    /// Its edge pixels stretch on.
    Clamp,
}

/// A channel of a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Red.
    R,
    /// Green.
    G,
    /// Blue.
    B,
    /// Alpha.
    A,
}

/// One picture holding several of a material's values in its channels: fewer textures to sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Packed {
    /// The picture, by asset id.
    pub texture: AssetId,
    /// The channel holding ambient occlusion (red by default), or none.
    #[serde(default = "red", skip_serializing_if = "is_red")]
    pub occlusion: Option<Channel>,
    /// The channel holding roughness (green by default), or none.
    #[serde(default = "green", skip_serializing_if = "is_green")]
    pub roughness: Option<Channel>,
    /// The channel holding metallic (blue by default), or none.
    #[serde(default = "blue", skip_serializing_if = "is_blue")]
    pub metallic: Option<Channel>,
}

fn red() -> Option<Channel> {
    Some(Channel::R)
}
fn green() -> Option<Channel> {
    Some(Channel::G)
}
fn blue() -> Option<Channel> {
    Some(Channel::B)
}
fn is_red(c: &Option<Channel>) -> bool {
    *c == red()
}
fn is_green(c: &Option<Channel>) -> bool {
    *c == green()
}
fn is_blue(c: &Option<Channel>) -> bool {
    *c == blue()
}

impl Packed {
    /// `texture` holding ambient occlusion in red, roughness in green and metallic in blue: the usual packing.
    pub fn new(texture: AssetId) -> Self {
        Self {
            texture,
            occlusion: red(),
            roughness: green(),
            metallic: blue(),
        }
    }
}

/// A material's pictures, one per channel, by asset id. A channel without one uses the material's constant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Textures {
    /// The base colour (albedo), multiplied by `color` and the part's colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<AssetId>,
    /// The tangent-space normal map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<AssetId>,
    /// Roughness, in its red channel, multiplied by `roughness`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<AssetId>,
    /// Metallic, in its red channel, multiplied by `metallic`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic: Option<AssetId>,
    /// Emitted light, multiplied by `emission` and `emission_strength`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission: Option<AssetId>,
    /// Ambient occlusion, in its red channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occlusion: Option<AssetId>,
}

impl Textures {
    /// Whether no channel has a picture.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Every picture named, in channel order.
    pub fn ids(&self) -> impl Iterator<Item = AssetId> + '_ {
        [
            self.color,
            self.normal,
            self.roughness,
            self.metallic,
            self.emission,
            self.occlusion,
        ]
        .into_iter()
        .flatten()
    }
}

/// A material: what a part looks like it is made of. See the [module documentation](self).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Material {
    /// The base colour's tint, sRGB 0 to 1, multiplied by the part's colour.
    #[serde(skip_serializing_if = "is_white")]
    pub color: [f32; 3],
    /// Microfacet roughness, from 0 (a mirror) to 1 (fully matte).
    #[serde(skip_serializing_if = "is_default_roughness")]
    pub roughness: f32,
    /// 0 for dielectrics (plastic, wood), 1 for bare metal.
    #[serde(skip_serializing_if = "is_zero")]
    pub metallic: f32,
    /// The emitted light's tint, sRGB 0 to 1, multiplied by the part's colour.
    #[serde(skip_serializing_if = "is_white")]
    pub emission: [f32; 3],
    /// How strongly it glows: 0 not at all, 1 at the part's colour (Neon).
    #[serde(skip_serializing_if = "is_zero")]
    pub emission_strength: f32,
    /// Glass: when the part is see-through, its reflections stay at full strength while its body fades.
    #[serde(skip_serializing_if = "is_false")]
    pub glass: bool,
    /// How strongly the normal map bends the light: 0 flat, 1 as painted.
    #[serde(skip_serializing_if = "is_one")]
    pub normal_strength: f32,
    /// How many metres one repeat of the textures covers on a part.
    #[serde(skip_serializing_if = "is_default_tile")]
    pub tile_size: f32,
    /// Repeats per tile along U and V.
    #[serde(skip_serializing_if = "is_one_2")]
    pub uv_scale: [f32; 2],
    /// Where the textures start, in tiles.
    #[serde(skip_serializing_if = "is_zero_2")]
    pub uv_offset: [f32; 2],
    /// How textures are sampled between pixels.
    #[serde(skip_serializing_if = "is_default")]
    pub filter: Filter,
    /// What textures do past their edges.
    #[serde(skip_serializing_if = "is_default")]
    pub wrap: Wrap,
    /// The pictures, by channel.
    #[serde(skip_serializing_if = "Textures::is_empty")]
    pub textures: Textures,
    /// One picture holding ambient occlusion, roughness and metallic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packed: Option<Packed>,
}

/// The default roughness: plastic's.
const DEFAULT_ROUGHNESS: f32 = 0.6;
/// The default tile size, in metres.
const DEFAULT_TILE: f32 = 4.0;

fn is_white(c: &[f32; 3]) -> bool {
    *c == [1.0; 3]
}
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
fn is_one(v: &f32) -> bool {
    *v == 1.0
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn is_default_roughness(v: &f32) -> bool {
    *v == DEFAULT_ROUGHNESS
}
fn is_default_tile(v: &f32) -> bool {
    *v == DEFAULT_TILE
}
fn is_one_2(v: &[f32; 2]) -> bool {
    *v == [1.0; 2]
}
fn is_zero_2(v: &[f32; 2]) -> bool {
    *v == [0.0; 2]
}
fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

impl Default for Material {
    /// Plain plastic: what a part with no material looks like.
    fn default() -> Self {
        Self {
            color: [1.0; 3],
            roughness: DEFAULT_ROUGHNESS,
            metallic: 0.0,
            emission: [1.0; 3],
            emission_strength: 0.0,
            glass: false,
            normal_strength: 1.0,
            tile_size: DEFAULT_TILE,
            uv_scale: [1.0; 2],
            uv_offset: [0.0; 2],
            filter: Filter::Picture,
            wrap: Wrap::Repeat,
            textures: Textures::default(),
            packed: None,
        }
    }
}

/// Why a material file could not be read.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum MaterialError {
    /// Not JSON, or not an object.
    #[error("not a material file: {0}")]
    Malformed(String),
    /// Written by a newer engine.
    #[error("material format {found} is newer than this engine reads ({MATERIAL_FORMAT})")]
    TooNew {
        /// The file's format.
        found: u32,
    },
}

/// A material read from a file, with what was skipped in it.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedMaterial {
    /// The material.
    pub material: Material,
    /// Keys this engine does not know, and values it could not read, each left at its default.
    pub warnings: Vec<String>,
}

/// The keys of a material file this engine knows.
const KEYS: [&str; 15] = [
    "format",
    "color",
    "roughness",
    "metallic",
    "emission",
    "emission_strength",
    "glass",
    "normal_strength",
    "tile_size",
    "uv_scale",
    "uv_offset",
    "filter",
    "wrap",
    "textures",
    "packed",
];

impl Material {
    /// Reads a material file. Unknown keys and unreadable values are left out with a warning; a file that is not
    /// a JSON object, or of a newer format, is refused.
    pub fn from_bytes(bytes: &[u8]) -> Result<LoadedMaterial, MaterialError> {
        let json: Json = serde_json::from_slice(bytes).map_err(|error| MaterialError::Malformed(error.to_string()))?;
        let Json::Object(object) = json else {
            return Err(MaterialError::Malformed("not a JSON object".into()));
        };
        let format = object.get("format").and_then(Json::as_u64).unwrap_or(1);
        if format > u64::from(MATERIAL_FORMAT) {
            return Err(MaterialError::TooNew {
                found: u32::try_from(format).unwrap_or(u32::MAX),
            });
        }
        let mut warnings = Vec::new();
        let mut known = serde_json::Map::new();
        for (key, value) in object {
            if key == "format" {
                continue;
            }
            if KEYS.contains(&key.as_str()) {
                known.insert(key, value);
            } else {
                warnings.push(format!("unknown key `{key}` skipped"));
            }
        }
        // Each value on its own, so one bad value costs only itself.
        let mut material = Material::default();
        let mut merged = serde_json::to_value(material).map_err(|error| MaterialError::Malformed(error.to_string()))?;
        for (key, value) in known {
            let mut trial = merged.clone();
            if let Json::Object(fields) = &mut trial {
                fields.insert(key.clone(), value);
            }
            match serde_json::from_value::<Material>(trial.clone()) {
                Ok(read) => {
                    material = read;
                    merged = trial;
                }
                Err(error) => warnings.push(format!("`{key}` skipped: {error}")),
            }
        }
        Ok(LoadedMaterial {
            material: material.clamped(),
            warnings,
        })
    }

    /// The file's text: its format, then every value not at its default, in a fixed order, two-space indented.
    pub fn to_json(&self) -> String {
        let mut fields: BTreeMap<&str, Json> = BTreeMap::new();
        if let Ok(Json::Object(object)) = serde_json::to_value(self) {
            for key in KEYS.iter().skip(1) {
                if let Some(value) = object.get(*key) {
                    fields.insert(key, value.clone());
                }
            }
        }
        let mut out = format!("{{\n  \"format\": {MATERIAL_FORMAT}");
        // KEYS' order, not alphabetical: the look first, the textures last.
        for key in KEYS.iter().skip(1) {
            if let Some(value) = fields.get(key) {
                let text = serde_json::to_string(value).unwrap_or_default();
                out.push_str(&format!(",\n  \"{key}\": {text}"));
            }
        }
        out.push_str("\n}\n");
        out
    }

    /// The material with every value in its range: colours and the 0-to-1 values clamped, sizes positive.
    pub fn clamped(mut self) -> Self {
        let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        self.color = self.color.map(unit);
        self.emission = self.emission.map(unit);
        self.roughness = unit(self.roughness);
        self.metallic = unit(self.metallic);
        self.emission_strength = if self.emission_strength.is_finite() {
            self.emission_strength.clamp(0.0, 100.0)
        } else {
            0.0
        };
        self.normal_strength = if self.normal_strength.is_finite() {
            self.normal_strength.clamp(0.0, 4.0)
        } else {
            1.0
        };
        if !(self.tile_size.is_finite() && self.tile_size > 0.0) {
            self.tile_size = DEFAULT_TILE;
        }
        self.uv_scale = self.uv_scale.map(|v| if v.is_finite() && v != 0.0 { v } else { 1.0 });
        self.uv_offset = self.uv_offset.map(|v| if v.is_finite() { v } else { 0.0 });
        self
    }

    /// Every asset this material refers to: its pictures, the packed one included.
    pub fn references(&self) -> Vec<AssetId> {
        let mut ids: Vec<AssetId> = self.textures.ids().collect();
        ids.extend(self.packed.map(|packed| packed.texture));
        ids
    }

    /// Whether its textures are sampled nearest: as its filter says, or for [`Filter::Picture`] as its pictures are
    /// (#781), `pixelated` saying which are: the first it uses, its colour first.
    pub fn nearest(&self, pixelated: impl Fn(AssetId) -> bool) -> bool {
        match self.filter {
            Filter::Linear => false,
            Filter::Nearest => true,
            Filter::Picture => self.references().first().is_some_and(|&id| pixelated(id)),
        }
    }
}

/// The materials a new project starts with, by name (the file's name without `.material`), in the order Studio lists
/// them: the familiar set, as plain values a project can change or delete.
pub const BUILTIN_NAMES: [&str; 21] = [
    "Plastic",
    "SmoothPlastic",
    "Metal",
    "DiamondPlate",
    "CorrodedMetal",
    "Foil",
    "Wood",
    "WoodPlanks",
    "Concrete",
    "Brick",
    "Cobblestone",
    "Granite",
    "Marble",
    "Slate",
    "Pebble",
    "Sand",
    "Grass",
    "Fabric",
    "Ice",
    "Glass",
    "Neon",
];

/// The built-in material called `name` ([`BUILTIN_NAMES`]): what a part whose `Material` is that name looks like
/// when its project has no material of that name. `None` for any other name.
pub fn builtin(name: &str) -> Option<Material> {
    let (roughness, metallic, emission_strength, glass) = match name {
        "Plastic" => (0.6, 0.0, 0.0, false),
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
    Some(Material {
        roughness,
        metallic,
        emission_strength,
        glass,
        ..Material::default()
    })
}

/// The member path of the material called `name` in [`MATERIALS_FOLDER`].
pub fn builtin_path(name: &str) -> String {
    format!("{MATERIALS_FOLDER}/{name}.{MATERIAL_EXTENSION}")
}

/// Whether `member` is a material file.
pub fn is_material(member: &str) -> bool {
    member
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case(MATERIAL_EXTENSION))
}

/// The name of the material file `member`: its file name without `.material`.
pub fn material_name(member: &str) -> &str {
    let name = member.rsplit_once('/').map_or(member, |(_, name)| name);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_material_is_only_its_format() {
        assert_eq!(Material::default().to_json(), "{\n  \"format\": 1\n}\n");
        let read = Material::from_bytes(b"{\"format\": 1}").unwrap();
        assert_eq!(read.material, Material::default());
        assert!(read.warnings.is_empty());
    }

    #[test]
    fn every_value_round_trips_and_writes_in_a_fixed_order() {
        let id = AssetId::from_u128(0x1234);
        let material = Material {
            color: [0.5, 0.25, 1.0],
            roughness: 0.3,
            metallic: 1.0,
            emission: [1.0, 0.5, 0.0],
            emission_strength: 2.0,
            glass: true,
            normal_strength: 0.5,
            tile_size: 2.0,
            uv_scale: [2.0, 3.0],
            uv_offset: [0.5, 0.25],
            filter: Filter::Nearest,
            wrap: Wrap::Mirror,
            textures: Textures {
                color: Some(id),
                normal: Some(AssetId::from_u128(0x5678)),
                ..Textures::default()
            },
            packed: Some(Packed {
                metallic: Some(Channel::A),
                ..Packed::new(AssetId::from_u128(0x9abc))
            }),
        };
        let text = material.to_json();
        let read = Material::from_bytes(text.as_bytes()).unwrap();
        assert_eq!(read.material, material);
        assert!(read.warnings.is_empty(), "{:?}", read.warnings);
        assert_eq!(read.material.to_json(), text, "writing again gives the same bytes");
        assert!(text.find("\"roughness\"") < text.find("\"textures\""), "{text}");
        assert!(
            text.contains("\"packed\": {\"metallic\":\"a\",\"texture\":\"00000000000000000000000000009abc\"}"),
            "{text}"
        );
        assert_eq!(material.references().len(), 3);
    }

    #[test]
    fn unknown_keys_and_bad_values_are_warnings_and_a_newer_format_is_refused() {
        let read =
            Material::from_bytes(br#"{"format": 1, "roughness": "rough", "metallic": 0.5, "sparkle": 3}"#).unwrap();
        assert_eq!(read.material.metallic, 0.5);
        assert_eq!(read.material.roughness, DEFAULT_ROUGHNESS);
        assert_eq!(read.warnings.len(), 2, "{:?}", read.warnings);
        assert!(matches!(
            Material::from_bytes(br#"{"format": 9}"#),
            Err(MaterialError::TooNew { found: 9 })
        ));
        assert!(Material::from_bytes(b"[1]").is_err());
        let wild = Material::from_bytes(br#"{"roughness": 7, "tile_size": -1}"#)
            .unwrap()
            .material;
        assert_eq!((wild.roughness, wild.tile_size), (1.0, DEFAULT_TILE));
    }

    #[test]
    fn the_builtin_set_is_named_and_in_range() {
        for name in BUILTIN_NAMES {
            let material = builtin(name).unwrap();
            assert_eq!(material, material.clamped(), "{name}");
            assert_eq!(material_name(&builtin_path(name)), name);
            assert!(is_material(&builtin_path(name)));
        }
        assert!(builtin("Glass").unwrap().glass);
        assert_eq!(builtin("Neon").unwrap().emission_strength, 1.0);
        assert_eq!(builtin("Plastic"), Some(Material::default()));
        assert_eq!(builtin("Cheese"), None);
    }

    /// #781: a material sampled as its pictures are follows its first picture's filtering; Smooth and Nearest say
    /// so whatever the pictures are.
    #[test]
    fn a_material_s_filtering_follows_its_pictures_unless_it_chooses() {
        let pixel: AssetId = "00000000000000000000000000000001".parse().unwrap();
        let smooth: AssetId = "00000000000000000000000000000002".parse().unwrap();
        let pixelated = |id: AssetId| id == pixel;
        let mut material = Material::default();
        assert_eq!(material.filter, Filter::Picture, "the default");
        assert!(!material.nearest(pixelated), "no pictures");
        material.textures.color = Some(pixel);
        material.textures.normal = Some(smooth);
        assert!(material.nearest(pixelated), "its colour's");
        material.textures.color = Some(smooth);
        assert!(!material.nearest(pixelated));
        material.filter = Filter::Nearest;
        assert!(material.nearest(pixelated));
        material.textures.color = Some(pixel);
        material.filter = Filter::Linear;
        assert!(!material.nearest(pixelated));
        // Written and read back by name.
        let read = Material::from_bytes(Material::default().to_json().as_bytes()).unwrap();
        assert_eq!(read.material.filter, Filter::Picture);
    }
}
