//! `EnumItem`: one item of a named enum, such as `Enum.Material.Plastic`.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// One item of an engine enum, named by its enum type and its item name (`Material`, `Plastic`).
///
/// Names are shared strings, so clones are cheap, and no language-specific representation is involved. An
/// `EnumItem` is not checked against the registry here: any type and name can be built, and the registry decides
/// whether they exist.
///
/// # Serialized form
///
/// `{"enum_type": "Material", "name": "Plastic"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnumItem {
    enum_type: Arc<str>,
    name: Arc<str>,
}

impl EnumItem {
    /// An item `name` of the enum `enum_type`, both without the `Enum.` prefix.
    pub fn new(enum_type: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            enum_type: enum_type.into(),
            name: name.into(),
        }
    }

    /// The enum's name, such as `Material`.
    pub fn enum_type(&self) -> &str {
        &self.enum_type
    }

    /// The item's name, such as `Plastic`.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Writes the full script path, such as `Enum.Material.Plastic`.
impl fmt::Display for EnumItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Enum.{}.{}", self.enum_type, self.name)
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
}
