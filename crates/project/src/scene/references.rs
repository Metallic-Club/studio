//! What a scene file refers to, found and rewritten in its JSON without a class registry (for the project's file
//! operations, which run without a game).
//!
//! Scenes refer to project members in two ways:
//!
//! - **By asset id**, such as `MeshPart.MeshId`: an id is 128 random bits, so any string property equal to an
//!   asset's id refers to that asset. Renaming or moving the asset does not change its id, so these never need
//!   rewriting.
//! - **By member path**, in the properties named in [`PATH_PROPERTIES`] (a script's `Source`, such as
//!   `Scripts/Door.luau`). These are rewritten when the member moves ([`rewrite_paths`]).
//!
//! A rewrite edits the file's text in place, replacing only the path strings it changes, so everything else stays as
//! it was, byte for byte: a scene written by [`save_scene`](super::save_scene) reads as if it had been saved with the
//! new paths.

use serde_json::Value as Json;

use super::{SCENE_FORMAT, SceneError};

/// The saved properties whose value is a project member path, by name. `SourceContainer.Source` (every script
/// class) is the only one: `MeshPart.MeshId` and `TextureId` hold asset ids, which survive moves. A test checks this
/// list against every saved string property of the built-in classes.
pub const PATH_PROPERTIES: &[&str] = &["Source"];

/// A property of a scene instance that refers to a project member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneReference {
    /// The scene's member path.
    pub scene: String,
    /// The instance's id in the scene.
    pub instance: u64,
    /// The instance's class name.
    pub class: String,
    /// The instance's name.
    pub name: String,
    /// The property that holds the reference.
    pub property: String,
}

/// The parsed scene `text`, refused like [`load_scene`](super::load_scene) refuses it: not JSON, no `format`, or a
/// newer format than this engine reads.
fn parse(text: &str) -> Result<Json, SceneError> {
    let doc: Json = serde_json::from_str(text).map_err(|error| SceneError::Syntax(error.to_string()))?;
    let found = doc
        .get("format")
        .and_then(Json::as_u64)
        .ok_or(SceneError::MissingFormat)?;
    if found > u64::from(SCENE_FORMAT) {
        return Err(SceneError::UnsupportedFormat {
            found,
            newest: SCENE_FORMAT,
        });
    }
    Ok(doc)
}

/// Every string property of the scene `text` (the member `scene`) for which `refers(property, value)` holds, in
/// file order.
pub(crate) fn find(
    scene: &str,
    text: &str,
    refers: impl Fn(&str, &str) -> bool,
) -> Result<Vec<SceneReference>, SceneError> {
    let doc = parse(text)?;
    let mut found = Vec::new();
    let instances = doc
        .get("instances")
        .and_then(Json::as_array)
        .map_or(&[][..], Vec::as_slice);
    for instance in instances {
        let Some(properties) = instance.get("properties").and_then(Json::as_object) else {
            continue;
        };
        let class = instance.get("class").and_then(Json::as_str).unwrap_or_default();
        for (property, value) in properties {
            if value.as_str().is_some_and(|value| refers(property, value)) {
                found.push(SceneReference {
                    scene: scene.to_owned(),
                    instance: instance.get("id").and_then(Json::as_u64).unwrap_or_default(),
                    class: class.to_owned(),
                    name: instance.get("name").and_then(Json::as_str).unwrap_or(class).to_owned(),
                    property: property.clone(),
                });
            }
        }
    }
    Ok(found)
}

/// One open JSON container while scanning: an object with the key being read, or an array.
enum Frame {
    Object {
        /// The latest key read.
        key: Option<String>,
        /// Whether the next string is a key (after `{` or `,`) rather than a value (after `:`).
        want_key: bool,
    },
    Array,
}

/// Rewrites the member paths in the scene `text`'s [`PATH_PROPERTIES`] that `remap` gives a new path for. Returns
/// the new text, or `None` when nothing changed. Only the rewritten strings change; every other byte is kept.
pub(crate) fn rewrite_paths(text: &str, remap: impl Fn(&str) -> Option<String>) -> Result<Option<String>, SceneError> {
    // Checked first, so the scan below only ever sees valid JSON.
    parse(text)?;
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    let mut stack: Vec<Frame> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => stack.push(Frame::Object {
                key: None,
                want_key: true,
            }),
            b'[' => stack.push(Frame::Array),
            b'}' | b']' => {
                stack.pop();
            }
            b',' | b':' => {
                if let Some(Frame::Object { want_key, .. }) = stack.last_mut() {
                    *want_key = bytes[i] == b',';
                }
            }
            b'"' => {
                let end = string_end(bytes, i);
                // Quotes and backslashes are ASCII, never inside a multi-byte character, so both ends are character
                // boundaries.
                let literal = &text[i..end];
                let value: String =
                    serde_json::from_str(literal).map_err(|error| SceneError::Syntax(error.to_string()))?;
                if let Some(Frame::Object { key, want_key: true }) = stack.last_mut() {
                    *key = Some(value);
                } else if is_path_property(&stack)
                    && let Some(path) = remap(&value)
                {
                    out.push_str(&text[copied..i]);
                    out.push_str(&serde_json::to_string(&path).map_err(|error| SceneError::Write(error.to_string()))?);
                    copied = end;
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if copied == 0 {
        return Ok(None);
    }
    out.push_str(&text[copied..]);
    Ok(Some(out))
}

/// The index just past the closing quote of the JSON string that opens at `start`.
fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Whether the scan is at the value of `instances[n].properties.<a path property>`.
fn is_path_property(stack: &[Frame]) -> bool {
    fn key(frame: &Frame) -> Option<&str> {
        match frame {
            Frame::Object { key, .. } => key.as_deref(),
            Frame::Array => None,
        }
    }
    match stack {
        [instances, Frame::Array, properties, property] => {
            key(instances) == Some("instances")
                && key(properties) == Some("properties")
                && key(property).is_some_and(|name| PATH_PROPERTIES.contains(&name))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = "{\n  \"format\": 1,\n  \"instances\": [{\n    \"id\": 1,\n    \"class\": \"Workspace\"\n  }, \
                         {\n    \"id\": 2,\n    \"parent\": 1,\n    \"class\": \"Script\",\n    \"name\": \"Door\",\n    \
                         \"properties\": {\n      \"Source\": \"Scripts/Door.luau\"\n    },\n    \"attributes\": {\n      \
                         \"Source\": {\"String\": \"Scripts/Door.luau\"}\n    }\n  }, {\n    \"id\": 3,\n    \
                         \"parent\": 1,\n    \"class\": \"StringValue\",\n    \"properties\": {\n      \
                         \"Value\": \"Scripts/Door.luau\"\n    }\n  }]\n}\n";

    fn door(path: &str) -> Option<String> {
        (path == "Scripts/Door.luau").then(|| "Scripts/Doors/Front \"door\".luau".to_owned())
    }

    #[test]
    fn only_path_properties_are_rewritten_and_nothing_else_moves() {
        let rewritten = rewrite_paths(SCENE, door).unwrap().unwrap();
        let expected = SCENE.replacen(
            "\"Source\": \"Scripts/Door.luau\"",
            "\"Source\": \"Scripts/Doors/Front \\\"door\\\".luau\"",
            1,
        );
        assert_eq!(
            rewritten, expected,
            "the attribute and the StringValue keep the old text"
        );
        assert_eq!(rewrite_paths(SCENE, |_| None).unwrap(), None);
    }

    #[test]
    fn references_are_found_with_their_instance() {
        let found = find("Scenes/Main.scene", SCENE, |property, value| {
            PATH_PROPERTIES.contains(&property) && value == "Scripts/Door.luau"
        })
        .unwrap();
        assert_eq!(
            found,
            [SceneReference {
                scene: "Scenes/Main.scene".to_owned(),
                instance: 2,
                class: "Script".to_owned(),
                name: "Door".to_owned(),
                property: "Source".to_owned(),
            }]
        );
    }

    /// A new string property that holds a member path must join [`PATH_PROPERTIES`], or moving that member breaks
    /// it. This lists every saved string property of the built-in classes, so adding one means deciding.
    #[test]
    fn every_saved_string_property_is_known_to_be_a_path_or_not() {
        // `Lighting.TimeOfDay`, `MeshPart.MeshId` and `TextureId` (asset ids), `StringValue.Value`.
        const NOT_PATHS: &[&str] = &["TimeOfDay", "MeshId", "TextureId", "Value"];
        let registry = metallic_classes::shared_registry().unwrap();
        let mut unknown = Vec::new();
        for class in registry.classes() {
            for prop in class.props() {
                let name = prop.meta.name;
                let string = prop.meta.ty == metallic_core::PropType::String;
                if string
                    && super::super::is_saved(prop)
                    && !PATH_PROPERTIES.contains(&name)
                    && !NOT_PATHS.contains(&name)
                {
                    unknown.push(format!("{}.{name}", class.name()));
                }
            }
        }
        assert!(
            unknown.is_empty(),
            "saved string properties neither paths nor known ids/text: {unknown:?}"
        );
    }

    #[test]
    fn scenes_that_do_not_read_are_refused() {
        assert!(matches!(rewrite_paths("{", door), Err(SceneError::Syntax(_))));
        assert_eq!(rewrite_paths("{}", door), Err(SceneError::MissingFormat));
        assert!(matches!(
            find("a.scene", "{\"format\": 99}", |_, _| true),
            Err(SceneError::UnsupportedFormat { found: 99, .. })
        ));
    }
}
