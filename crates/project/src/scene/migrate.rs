//! Upgrades from older scene formats.
//!
//! Each step rewrites the parsed JSON of one version into the next, before the scene is read, so the reader only
//! ever sees the current format. A file loads through every step from its version up to [`SCENE_FORMAT`], and each
//! step is reported as a [`SceneWarning::Migrated`].
//!
//! To change the format: raise [`SCENE_FORMAT`], add a step from the old version to [`MIGRATIONS`], and check in a
//! fixture written in the old version under `tests/fixtures/`.

use serde_json::Value as Json;

use super::{SCENE_FORMAT, SceneError, SceneWarning};

/// One upgrade step, from version `from` to `from + 1`.
struct Migration {
    /// The version it reads.
    from: u32,
    /// What changed, for the warning.
    change: &'static str,
    /// Rewrites a scene of version `from` into the next version (everything but the `format` key).
    apply: fn(&mut Json) -> Result<(), SceneError>,
}

/// Every upgrade step, oldest first; one per version below [`SCENE_FORMAT`].
static MIGRATIONS: &[Migration] = &[
    Migration {
        from: 0,
        change: "instance names moved from the `Name` property to the `name` key",
        apply: names_out_of_properties,
    },
    Migration {
        from: 1,
        change: "parts are anchored by default: a part left unanchored says so",
        apply: unanchored_parts_say_so,
    },
];

/// The classes that were parts (`BasePart` and what derives from it) in format 1. Fixed: a class added later was
/// never saved in format 1.
const FORMAT_1_PARTS: [&str; 2] = ["Part", "MeshPart"];

/// Upgrades `doc`, a scene of version `from`, to [`SCENE_FORMAT`], adding a warning per step.
pub(super) fn upgrade(doc: &mut Json, from: u32, warnings: &mut Vec<SceneWarning>) -> Result<(), SceneError> {
    let mut version = from;
    while version < SCENE_FORMAT {
        let step = MIGRATIONS
            .iter()
            .find(|m| m.from == version)
            .ok_or_else(|| SceneError::Malformed(format!("no upgrade from format {version}")))?;
        (step.apply)(doc)?;
        warnings.push(SceneWarning::Migrated {
            from: version,
            to: version + 1,
            change: step.change,
        });
        version += 1;
    }
    if let Some(format) = doc.get_mut("format") {
        *format = Json::from(SCENE_FORMAT);
    }
    Ok(())
}

/// Format 0 stored an instance's name as its `Name` property; format 1 has a `name` key, because `Name` is not a
/// saved property (the data model keeps names apart from properties). A name equal to the class name is dropped,
/// as format 1 writes it.
fn names_out_of_properties(doc: &mut Json) -> Result<(), SceneError> {
    let Some(instances) = doc.get_mut("instances").and_then(Json::as_array_mut) else {
        return Ok(());
    };
    for instance in instances {
        let Some(instance) = instance.as_object_mut() else {
            continue;
        };
        let name = instance
            .get_mut("properties")
            .and_then(Json::as_object_mut)
            .and_then(|properties| properties.remove("Name"));
        let Some(name) = name else {
            continue;
        };
        if !name.is_string() {
            return Err(SceneError::Malformed(format!(
                "a `Name` property is not a string: {name}"
            )));
        }
        if instance.get("class") != Some(&name) {
            instance.insert("name".to_owned(), name);
        }
    }
    Ok(())
}

/// Format 1 left a part's `Anchored` out when it was off, the default then; format 2's default is on (#589), so a
/// part without `Anchored` gets `false` and stays unanchored.
fn unanchored_parts_say_so(doc: &mut Json) -> Result<(), SceneError> {
    let Some(instances) = doc.get_mut("instances").and_then(Json::as_array_mut) else {
        return Ok(());
    };
    for instance in instances {
        let Some(instance) = instance.as_object_mut() else {
            continue;
        };
        let is_part = instance
            .get("class")
            .and_then(Json::as_str)
            .is_some_and(|class| FORMAT_1_PARTS.contains(&class));
        if !is_part {
            continue;
        }
        let properties = instance
            .entry("properties")
            .or_insert_with(|| Json::Object(Default::default()));
        let Some(properties) = properties.as_object_mut() else {
            return Err(SceneError::Malformed(format!(
                "a part's `properties` is not an object: {properties}"
            )));
        };
        properties.entry("Anchored").or_insert(Json::Bool(false));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn every_older_version_has_a_step() {
        for version in 0..SCENE_FORMAT {
            assert!(MIGRATIONS.iter().any(|m| m.from == version), "no step from {version}");
        }
    }

    #[test]
    fn format_0_names_move_out_of_properties() {
        let mut doc = json!({
            "format": 0,
            "instances": [
                {"id": 1, "class": "Workspace", "properties": {"Name": "Workspace"}},
                {"id": 2, "parent": 1, "class": "Part", "properties": {"Name": "Floor", "Anchored": true}},
            ],
        });
        let mut warnings = Vec::new();
        upgrade(&mut doc, 0, &mut warnings).unwrap();
        assert_eq!(
            doc,
            json!({
                "format": SCENE_FORMAT,
                "instances": [
                    {"id": 1, "class": "Workspace", "properties": {}},
                    {"id": 2, "parent": 1, "class": "Part", "name": "Floor", "properties": {"Anchored": true}},
                ],
            })
        );
        assert!(matches!(
            warnings.as_slice(),
            [
                SceneWarning::Migrated { from: 0, to: 1, .. },
                SceneWarning::Migrated { from: 1, to: 2, .. }
            ]
        ));
    }

    /// #589: a format 1 part that left `Anchored` out was unanchored, and stays so.
    #[test]
    fn format_1_parts_keep_their_anchored() {
        let mut doc = json!({
            "format": 1,
            "instances": [
                {"id": 1, "class": "Workspace"},
                {"id": 2, "parent": 1, "class": "Part", "name": "Floor", "properties": {"Anchored": true}},
                {"id": 3, "parent": 1, "class": "Part", "name": "Crate", "properties": {"Size": [2.0, 2.0, 2.0]}},
                {"id": 4, "parent": 1, "class": "MeshPart"},
                {"id": 5, "parent": 1, "class": "Folder"},
            ],
        });
        let mut warnings = Vec::new();
        upgrade(&mut doc, 1, &mut warnings).unwrap();
        assert_eq!(
            doc,
            json!({
                "format": SCENE_FORMAT,
                "instances": [
                    {"id": 1, "class": "Workspace"},
                    {"id": 2, "parent": 1, "class": "Part", "name": "Floor", "properties": {"Anchored": true}},
                    {
                        "id": 3, "parent": 1, "class": "Part", "name": "Crate",
                        "properties": {"Anchored": false, "Size": [2.0, 2.0, 2.0]},
                    },
                    {"id": 4, "parent": 1, "class": "MeshPart", "properties": {"Anchored": false}},
                    {"id": 5, "parent": 1, "class": "Folder"},
                ],
            })
        );
        assert!(matches!(
            warnings.as_slice(),
            [SceneWarning::Migrated { from: 1, to: 2, .. }]
        ));
    }

    #[test]
    fn current_files_are_left_alone() {
        let mut doc = json!({"format": SCENE_FORMAT, "instances": []});
        let before = doc.clone();
        let mut warnings = Vec::new();
        upgrade(&mut doc, SCENE_FORMAT, &mut warnings).unwrap();
        assert_eq!(doc, before);
        assert!(warnings.is_empty());
    }
}
