//! Material names into asset ids (#678, ADR 0012).
//!
//! A part's `Material` names a material asset by its id. Scenes from before material assets hold a material's name
//! instead (`"Concrete"`: the old `Material` enum's items were saved by name), and so does the Baseplate template,
//! whose ids are only made when a project is made from it. [`Project::link_material_names`] turns each name into the
//! id of the project's material of that name, first making the material from the built-in set
//! ([`crate::material::builtin`]) when the project has none, and rewrites the scenes in place, only where a name
//! changes. A name neither the project nor the built-in set has is left as it is (it draws as plastic, with a
//! warning, until someone picks a material).

use std::collections::BTreeMap;

use serde_json::Value as Json;

use super::{Project, ProjectError};
use crate::asset_index::AssetId;
use crate::material::{builtin, builtin_path, is_material, material_name};
use crate::scene::{SCENE_EXTENSION, rewrite_values};

/// The property that names a part's material.
pub const MATERIAL_PROPERTY: &str = "Material";

/// What [`Project::link_material_names`] changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MaterialLinks {
    /// The scenes rewritten, sorted.
    pub scenes: Vec<String>,
    /// The material files made from the built-in set, sorted.
    pub created: Vec<String>,
    /// Names that neither the project nor the built-in set has, sorted.
    pub unknown: Vec<String>,
}

impl MaterialLinks {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.scenes.is_empty() && self.created.is_empty()
    }
}

/// Whether `value` names a material by name rather than by id.
fn is_name(value: &str) -> bool {
    !value.is_empty() && value.parse::<AssetId>().is_err()
}

/// Every material name the scene `text` holds in a `Material` property.
fn names_in(text: &str) -> Vec<String> {
    let Ok(doc) = serde_json::from_str::<Json>(text) else {
        return Vec::new();
    };
    let instances = doc
        .get("instances")
        .and_then(Json::as_array)
        .map_or(&[][..], Vec::as_slice);
    instances
        .iter()
        .filter_map(|instance| instance.get("properties")?.get(MATERIAL_PROPERTY)?.as_str())
        .filter(|value| is_name(value))
        .map(str::to_owned)
        .collect()
}

impl Project {
    /// The id of the project's material called `name` (the first material file, by path, whose name without
    /// `.material` is `name`).
    pub fn material_named(&self, name: &str) -> Option<AssetId> {
        self.members()
            .into_iter()
            .find(|member| is_material(member) && material_name(member) == name)
            .and_then(|member| self.assets.id(member))
    }

    /// Turns the material names in every scene into the ids of the project's materials of those names, making a
    /// material from the built-in set where the project has none. The project's asset index must be up to date
    /// ([`Project::refresh_assets`]). Changes stay in memory until [`Project::save`]. A scene that does not read is
    /// left alone.
    pub fn link_material_names(&mut self) -> Result<MaterialLinks, ProjectError> {
        let scenes: Vec<String> = self
            .members()
            .into_iter()
            .filter(|member| {
                member
                    .rsplit_once('.')
                    .is_some_and(|(_, extension)| extension == SCENE_EXTENSION)
            })
            .map(str::to_owned)
            .collect();
        let mut texts = Vec::new();
        let mut names: BTreeMap<String, Option<String>> = BTreeMap::new();
        for scene in scenes {
            let Ok(text) = self.read_text(&scene) else {
                continue;
            };
            for name in names_in(&text) {
                names.entry(name).or_insert(None);
            }
            texts.push((scene, text));
        }
        let mut links = MaterialLinks::default();
        for (name, id) in &mut names {
            if let Some(found) = self.material_named(name) {
                *id = Some(found.to_string());
            } else if let Some(material) = builtin(name) {
                let path = builtin_path(name);
                let made = self.create_member(&path, material.to_json())?;
                links.created.push(path);
                *id = Some(made.to_string());
            } else {
                links.unknown.push(name.clone());
            }
        }
        for (scene, text) in texts {
            let rewritten = rewrite_values(&text, &[MATERIAL_PROPERTY], |value| names.get(value).cloned().flatten());
            if let Ok(Some(text)) = rewritten {
                self.write(&scene, text.into_bytes())?;
                links.scenes.push(scene);
            }
        }
        links.scenes.sort();
        links.created.sort();
        Ok(links)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::project::Template;

    #[test]
    fn names_become_ids_of_the_project_s_materials_made_from_the_built_in_set_when_missing() {
        let dir = std::env::temp_dir().join(format!("metallic-material-links-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let template =
            Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d")).unwrap();
        let mut project = Project::create(dir.join("Game.mtproj"), &template, "Game").unwrap();
        // A new project's scene names its materials; made ids, they name them by id.
        project.refresh_assets().unwrap();
        let links = project.link_material_names().unwrap();
        assert_eq!(links.scenes, [project.manifest().start_scene.clone()]);
        assert!(links.created.is_empty() && links.unknown.is_empty());
        let start = project.manifest().start_scene.clone();
        assert!(names_in(&project.read_text(&start).unwrap()).is_empty());
        let concrete = project.material_named("Concrete").unwrap();
        assert!(project.read_text(&start).unwrap().contains(&concrete.to_string()));

        // An old scene: a name the project has, one only the built-in set has, and one nobody has.
        project.delete_member(&builtin_path("Neon")).unwrap();
        let old = "{\n  \"format\": 2,\n  \"instances\": [{\n    \"id\": 1,\n    \"class\": \"Workspace\"\n  }, {\n    \
                   \"id\": 2,\n    \"parent\": 1,\n    \"class\": \"Part\",\n    \"properties\": {\n      \
                   \"Material\": \"Concrete\"\n    }\n  }, {\n    \"id\": 3,\n    \"parent\": 1,\n    \"class\": \
                   \"Part\",\n    \"properties\": {\n      \"Material\": \"Neon\"\n    }\n  }, {\n    \"id\": 4,\n    \
                   \"parent\": 1,\n    \"class\": \"Part\",\n    \"properties\": {\n      \"Material\": \"Cheese\"\n    \
                   }\n  }]\n}\n";
        project.write("Scenes/Old.scene", old).unwrap();
        let links = project.link_material_names().unwrap();
        assert_eq!(links.scenes, ["Scenes/Old.scene"]);
        assert_eq!(links.created, [builtin_path("Neon")]);
        assert_eq!(links.unknown, ["Cheese"]);
        let neon = project.material_named("Neon").unwrap();
        let text = project.read_text("Scenes/Old.scene").unwrap();
        assert!(text.contains(&format!("\"Material\": \"{concrete}\"")), "{text}");
        assert!(text.contains(&format!("\"Material\": \"{neon}\"")), "{text}");
        assert!(text.contains("\"Material\": \"Cheese\""), "{text}");
        // Done: a second pass changes nothing.
        assert!(project.link_material_names().unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
