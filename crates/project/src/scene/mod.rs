//! Scene files: a game's saved instances as versioned JSON (ADR 0006, "Scenes").
//!
//! [`save_scene`] writes every service whose class is marked `persisted` (`Workspace`, `Lighting`, ...) and
//! everything under it; [`load_scene`] puts a scene back into a game's data model. Both work on text or bytes, not
//! paths: a scene is a member of a `.mtproj` archive, and finding it there is the project code's job.
//!
//! # The format
//!
//! ```json
//! {
//!   "format": 2,
//!   "instances": [{
//!     "id": 1,
//!     "class": "Workspace",
//!     "properties": {
//!       "CurrentCamera": 2
//!     }
//!   }, {
//!     "id": 2,
//!     "parent": 1,
//!     "class": "Camera",
//!     "properties": {
//!       "CFrame": [0.0, 5.0, 10.0, -0.2297529, 0.0, 0.0, 0.97324896]
//!     }
//!   }, {
//!     "id": 3,
//!     "parent": 1,
//!     "class": "Part",
//!     "name": "Baseplate",
//!     "properties": {
//!       "Color": [0.388, 0.373, 0.384],
//!       "Material": "Concrete",
//!       "Size": [160.0, 2.0, 160.0]
//!     },
//!     "attributes": {
//!       "Speed": {"Number": 3.0}
//!     }
//!   }]
//! }
//! ```
//!
//! - `format` is the version ([`SCENE_FORMAT`]). A newer one is refused before anything loads; an older one is
//!   upgraded by the migrations in `migrate.rs`, each reported as a [`SceneWarning::Migrated`].
//! - `instances` is a flat list, parents before their children, children in their order in the tree. A flat list
//!   keeps every instance one entry deep, so a deep tree never hits a nesting limit, and an edit to one instance
//!   is a change to one entry. An entry without `parent` is a saved service.
//! - `id` is the instance's stable id in this scene ([`SceneIds`]); `parent` and instance references use it, so
//!   reordering or inserting instances never re-points a reference.
//! - `name` is left out when it is the class's name. `properties` holds only the properties the class registry
//!   marks as saved (stored, not computed, not `NO_SAVE`, not `RUNTIME`, not hidden) whose value differs from the
//!   class default, keys sorted. The registry's type for each property says how its value reads, so values are
//!   untagged: numbers, strings, booleans, arrays for `Vector2` / `Vector3` / `Color3`, the seven numbers of a
//!   `CFrame` (position, then rotation quaternion), an enum item's name, and an instance's `id` for a reference.
//! - `attributes` have no declared type, so each value names its own: `{"Vector3": [1.0, 2.0, 3.0]}`.
//! - Script instances are data like any other: their `Source` property holds the archive member path of their code,
//!   such as `Scripts/Door.luau` (ADR 0006). Moving a member rewrites the paths that name it ([`PATH_PROPERTIES`]);
//!   assets referred to by id (`MeshPart.MeshId`) need no rewrite.
//!
//! The text is deterministic: the same game and [`SceneIds`] always give the same bytes, so saving a scene that was
//! just loaded writes the file it was loaded from.

mod json;
mod load;
mod migrate;
mod references;
mod save;
mod value;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use metallic_core::{DmError, Id, PropFlags, ResolvedProp};

pub use load::{load_scene, load_scene_bytes};
pub use references::{PATH_PROPERTIES, SceneReference};
pub(crate) use references::{find as find_references, rewrite_paths};
pub use save::save_scene;

/// The scene format version this engine writes. Older versions load through migrations; newer ones are refused.
pub const SCENE_FORMAT: u32 = 2;

/// The file name extension of a scene inside a project (`Scenes/Main.scene`).
pub const SCENE_EXTENSION: &str = "scene";

/// The stable id of each instance of a scene: what `id`, `parent` and references hold in the file.
///
/// Ids survive load, edit and save: [`load_scene`] returns the ids the file gave, and [`save_scene`] keeps them
/// and gives new instances fresh ones. So a reference keeps pointing at the same instance however the tree is
/// edited. An editor keeps one `SceneIds` per open scene; a first save starts from [`SceneIds::new`], which numbers
/// instances from 1 in tree order.
#[derive(Clone, Debug)]
pub struct SceneIds {
    ids: HashMap<Id, u64>,
    /// The next id to give out. Never moves back, so an id freed by a deleted instance is not reused while the
    /// scene is open.
    next: u64,
}

impl SceneIds {
    /// No ids yet: the first save numbers instances from 1.
    pub fn new() -> Self {
        Self {
            ids: HashMap::new(),
            next: 1,
        }
    }

    /// The scene id of `instance`, if it has one.
    pub fn get(&self, instance: Id) -> Option<u64> {
        self.ids.get(&instance).copied()
    }

    /// The instance with scene id `scene_id`, if one has it: which instance of a loaded copy is which instance of the
    /// scene it was saved from (Studio's Play, ADR 0010). Looks through every id.
    pub fn instance(&self, scene_id: u64) -> Option<Id> {
        self.ids
            .iter()
            .find(|(_, id)| **id == scene_id)
            .map(|(instance, _)| *instance)
    }

    /// How many instances have an id.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether no instance has an id.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Every instance with an id, and its id, in no order.
    pub fn iter(&self) -> impl Iterator<Item = (Id, u64)> + '_ {
        self.ids.iter().map(|(&instance, &id)| (instance, id))
    }

    /// Gives `instance` the scene id `id`, for a save that must use ids chosen elsewhere (Team Create's net ids, so
    /// a participant's copy has them). Ids given out later stay above it.
    pub fn assign(&mut self, instance: Id, id: u64) {
        self.insert(instance, id);
    }

    /// Records that `instance` has the scene id `id`.
    fn insert(&mut self, instance: Id, id: u64) {
        self.ids.insert(instance, id);
        self.next = self.next.max(id.saturating_add(1));
    }
}

impl Default for SceneIds {
    fn default() -> Self {
        Self::new()
    }
}

/// A saved scene: the file's text and what could not be saved.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedScene {
    /// The scene file's contents; its UTF-8 bytes (`into_bytes`) are the archive member.
    pub text: String,
    /// What was left out, in the order it was found (the caller reports it).
    pub warnings: Vec<SceneWarning>,
}

/// What [`load_scene`] loaded.
#[derive(Clone, Debug)]
pub struct LoadedScene {
    /// The scene id of every loaded instance and saved service, to pass to the next [`save_scene`].
    pub ids: SceneIds,
    /// The format version the file was written in, before any migration.
    pub format: u32,
    /// What was skipped or upgraded, in file order after the migrations (the caller reports them).
    pub warnings: Vec<SceneWarning>,
}

/// Something in a scene that was skipped or changed rather than refused.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SceneWarning {
    /// The file was written in an older format and upgraded while loading.
    #[error("upgraded the scene from format {from} to {to}: {change}")]
    Migrated {
        /// The version the step started from.
        from: u32,
        /// The version it upgraded to.
        to: u32,
        /// What changed between them.
        change: &'static str,
    },
    /// No class has this name (it may come from a newer or older engine). The instance and everything under it
    /// were skipped.
    #[error("instance {id}: there is no class `{class}`; it and everything under it were skipped")]
    UnknownClass {
        /// The instance's scene id.
        id: u64,
        /// The class name the file gives.
        class: String,
    },
    /// The class exists but a scene cannot hold it where it is: an abstract or engine-only class, a service below
    /// the top, or a class at the top that is not a saved service. Skipped with everything under it.
    #[error("instance {id}: a `{class}` cannot be loaded here; it and everything under it were skipped")]
    Misplaced {
        /// The instance's scene id.
        id: u64,
        /// The class name.
        class: String,
    },
    /// The instance's parent is not an earlier instance of the scene, so it has nowhere to go. Skipped with
    /// everything under it.
    #[error("instance {id}: its parent {parent} is not earlier in the scene; it was skipped")]
    Orphan {
        /// The instance's scene id.
        id: u64,
        /// The parent id the file gives.
        parent: u64,
    },
    /// The class has no saved property of this name: it is unknown (from a newer or older engine) or one that
    /// scene files do not hold (computed, `NO_SAVE`, `RUNTIME`). It keeps its default.
    #[error("instance {id}: `{class}` has no saved property `{property}`; it was skipped")]
    UnknownProperty {
        /// The instance's scene id.
        id: u64,
        /// The instance's class.
        class: &'static str,
        /// The property name the file gives.
        property: String,
    },
    /// The property's value does not fit its declared type (or names an enum item that does not exist). The
    /// property keeps its default.
    #[error("instance {id}: `{class}.{property}` must be {expected}; the value was skipped")]
    BadValue {
        /// The instance's scene id.
        id: u64,
        /// The instance's class.
        class: &'static str,
        /// The property.
        property: &'static str,
        /// The declared type, as a game developer reads it.
        expected: String,
    },
    /// A reference names an instance that is not in the scene (or was skipped), or one of the wrong class. The
    /// property stays nil.
    #[error("instance {id}: `{class}.{property}` refers to instance {target}, which is missing or of the wrong class")]
    BadReference {
        /// The instance's scene id.
        id: u64,
        /// The instance's class.
        class: &'static str,
        /// The property.
        property: &'static str,
        /// The scene id it refers to.
        target: u64,
    },
    /// An attribute's value is not one an attribute can hold. The attribute was skipped.
    #[error("instance {id}: attribute `{name}` has a value an attribute cannot hold; it was skipped")]
    BadAttribute {
        /// The instance's scene id.
        id: u64,
        /// The attribute's name.
        name: String,
    },
    /// Saving: a reference points outside the saved part of the game (a service that is not saved, or an
    /// instance with `Archivable` off), so it was not written and loads as nil.
    #[error("{instance}.{property} refers to an instance that is not saved; it was written as nil")]
    ReferenceNotSaved {
        /// The full name of the instance holding the reference.
        instance: String,
        /// The property.
        property: &'static str,
    },
}

/// Why a scene could not be saved or loaded. A load that fails leaves the data model as it was.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SceneError {
    /// The text is not JSON.
    #[error("the scene is not valid JSON: {0}")]
    Syntax(String),
    /// The file has no `format` version, or it is not a whole number.
    #[error("the scene has no format version (a whole-number `format` key)")]
    MissingFormat,
    /// The file was written by a newer engine.
    #[error("the scene is in format {found}, but this engine reads formats up to {newest}")]
    UnsupportedFormat {
        /// The version in the file.
        found: u64,
        /// The newest version this engine reads ([`SCENE_FORMAT`]).
        newest: u32,
    },
    /// The JSON does not have the shape of a scene (a missing `id` or `class`, a list that is not a list, ...).
    #[error("the scene is malformed: {0}")]
    Malformed(String),
    /// Two instances share an id, so references to it would be ambiguous.
    #[error("two instances in the scene have the id {0}")]
    DuplicateId(u64),
    /// The data model lacks a saved service, so it is not a whole game.
    #[error("the game has no `{0}` service to load the scene into")]
    MissingService(&'static str),
    /// The data model refused a read or a change (while loading: a class hook failed as the scene was put in
    /// place).
    #[error(transparent)]
    DataModel(#[from] DmError),
    /// The scene could not be written as JSON.
    #[error("the scene could not be written: {0}")]
    Write(String),
}

/// Whether scene files hold `prop`: stored, visible, and neither `NO_SAVE` nor `RUNTIME`.
fn is_saved(prop: &ResolvedProp) -> bool {
    !prop.hidden
        && prop.meta.computed.is_none()
        && !prop.meta.flags.intersects(PropFlags::NO_SAVE.union(PropFlags::RUNTIME))
}
