//! `EnumItem`: one item of a named enum, such as `Enum.Material.Plastic`.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One item of an engine enum, named by its enum type and its item name (`Material`, `Plastic`).
///
/// Both names sit behind one shared pointer, so an `EnumItem` is 8 bytes and clones are cheap; no
/// language-specific representation is involved. An `EnumItem` is not checked against the registry here: any type
/// and name can be built, and the registry decides whether they exist.
///
/// # Serialized form
///
/// `{"enum_type": "Material", "name": "Plastic"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnumItem(Arc<EnumNames>);

/// The two names, kept together so `EnumItem` is a single thin pointer.
#[derive(Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct EnumNames {
    enum_type: Box<str>,
    name: Box<str>,
}

impl EnumItem {
    /// An item `name` of the enum `enum_type`, both without the `Enum.` prefix.
    pub fn new(enum_type: &str, name: &str) -> Self {
        Self(Arc::new(EnumNames {
            enum_type: enum_type.into(),
            name: name.into(),
        }))
    }

    /// The enum's name, such as `Material`.
    pub fn enum_type(&self) -> &str {
        &self.0.enum_type
    }

    /// The item's name, such as `Plastic`.
    pub fn name(&self) -> &str {
        &self.0.name
    }
}

/// Writes the full script path, such as `Enum.Material.Plastic`.
impl fmt::Display for EnumItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Enum.{}.{}", self.enum_type(), self.name())
    }
}

impl Serialize for EnumItem {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EnumItem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        EnumNames::deserialize(deserializer).map(|names| Self(Arc::new(names)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_as_script_path() {
        let item = EnumItem::new("Material", "Plastic");
        assert_eq!(item.to_string(), "Enum.Material.Plastic");
        assert_eq!(item.enum_type(), "Material");
        assert_eq!(item.name(), "Plastic");
    }

    #[test]
    fn serde_round_trips() {
        let item = EnumItem::new("Material", "Plastic");
        let json = serde_json::to_string(&item).expect("serialize");
        assert_eq!(json, r#"{"enum_type":"Material","name":"Plastic"}"#);
        assert_eq!(serde_json::from_str::<EnumItem>(&json).expect("deserialize"), item);
    }

    #[test]
    fn is_one_pointer() {
        assert_eq!(size_of::<EnumItem>(), size_of::<usize>());
        let item = EnumItem::new("Material", "Plastic");
        assert_eq!(item.clone(), item);
        assert_ne!(EnumItem::new("Material", "Wood"), item);
    }
}
