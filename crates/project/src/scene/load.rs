//! [`load_scene`]: scene text into a game, all or nothing.
//!
//! Loading runs in three stages, and only the last touches the game's existing instances:
//!
//! 1. **Read:** parse, upgrade an older format, and check every instance, property, attribute and reference against
//!    the class registry. Whatever cannot load is skipped with a warning; what is wrong with the file as a whole
//!    (not JSON, a newer format, two instances with one id, a missing service) is an error, and nothing changed.
//! 2. **Stage:** create the new instances, set their properties and attributes, and join them into subtrees, all
//!    outside the game. If the data model refuses anything, the staged instances are destroyed and the game is as
//!    it was.
//! 3. **Commit:** clear the saved services, set their properties (the file's, or the defaults), move the staged
//!    subtrees in, and set the references that point from services. Everything here was checked in stage 1, so
//!    only a failing class hook can stop it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use metallic_core::{ClassId, DataModel, Id, PropId, PropType, Registry, Value};
use serde::Deserialize;
use serde_json::{Map, Value as Json};

use super::value::{self, Attribute, Read};
use super::{LoadedScene, SCENE_FORMAT, SceneError, SceneIds, SceneWarning, is_saved, migrate};

/// The whole file, as read (after any upgrade; `format` was checked before).
#[derive(Deserialize)]
struct FileIn {
    #[serde(default)]
    instances: Vec<InstanceIn>,
}

/// One instance, as read.
#[derive(Deserialize)]
struct InstanceIn {
    id: u64,
    #[serde(default)]
    parent: Option<u64>,
    class: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    properties: Map<String, Json>,
    #[serde(default)]
    attributes: Map<String, Json>,
}

/// Where a loaded instance goes.
#[derive(Clone, Copy)]
enum Place {
    /// It is this saved service of the game.
    Service(Id),
    /// It is new, under the entry with this index.
    Under(usize),
}

/// An instance that passed stage 1, ready to stage.
struct Entry {
    scene_id: u64,
    class: ClassId,
    place: Place,
    name: Option<Arc<str>>,
    values: Vec<(PropId, Value)>,
    /// References, to entry indices.
    refs: Vec<(PropId, usize)>,
    attributes: Vec<(String, Value)>,
}

/// A reference waiting for the whole scene to be read.
struct PendingRef {
    entry: usize,
    prop: PropId,
    property: &'static str,
    /// The class the property declares it holds.
    class: &'static str,
    target: u64,
}

/// Loads the scene in `text` into `dm`, replacing the contents of every saved service (the classes marked
/// `persisted`) and their properties. Services the scene leaves out are emptied and get their default properties.
///
/// All or nothing: on an error the game is unchanged. Unknown classes, properties and values, and an older format,
/// are warnings (see the [module documentation](self)). `dm` must be a whole game: a root with every service.
pub fn load_scene(dm: &mut DataModel, text: &str) -> Result<LoadedScene, SceneError> {
    let doc: Json = serde_json::from_str(text).map_err(|error| SceneError::Syntax(error.to_string()))?;
    load_json(dm, doc)
}

/// [`load_scene`] from the raw bytes of a scene file, which must be UTF-8.
pub fn load_scene_bytes(dm: &mut DataModel, bytes: &[u8]) -> Result<LoadedScene, SceneError> {
    let doc: Json = serde_json::from_slice(bytes).map_err(|error| SceneError::Syntax(error.to_string()))?;
    load_json(dm, doc)
}

fn load_json(dm: &mut DataModel, mut doc: Json) -> Result<LoadedScene, SceneError> {
    let found = doc
        .get("format")
        .and_then(Json::as_u64)
        .ok_or(SceneError::MissingFormat)?;
    let format = u32::try_from(found)
        .ok()
        .filter(|&format| format <= SCENE_FORMAT)
        .ok_or(SceneError::UnsupportedFormat {
            found,
            newest: SCENE_FORMAT,
        })?;
    let mut warnings = Vec::new();
    migrate::upgrade(&mut doc, format, &mut warnings)?;
    let file: FileIn = serde_json::from_value(doc).map_err(|error| SceneError::Malformed(error.to_string()))?;

    let registry = Arc::clone(dm.registry());
    let services = saved_services(dm, &registry)?;
    let entries = read(dm, &registry, file, &mut warnings)?;
    let targets = stage(dm, &entries)?;
    commit(dm, &registry, &services, &entries, &targets)?;

    let mut ids = SceneIds::new();
    for (entry, &id) in entries.iter().zip(&targets) {
        ids.insert(id, entry.scene_id);
    }
    Ok(LoadedScene { ids, format, warnings })
}

/// Every saved service of the game, in service order, or an error naming one the game lacks.
fn saved_services(dm: &DataModel, registry: &Registry) -> Result<Vec<Id>, SceneError> {
    registry
        .services()
        .iter()
        .filter_map(|&class| registry.class(class).filter(|info| info.persisted()))
        .map(|info| dm.service(info.id()).ok_or(SceneError::MissingService(info.name())))
        .collect()
}

/// Stage 1: checks the file against the registry and the game, without changing anything.
fn read(
    dm: &DataModel,
    registry: &Registry,
    file: FileIn,
    warnings: &mut Vec<SceneWarning>,
) -> Result<Vec<Entry>, SceneError> {
    let mut entries: Vec<Entry> = Vec::with_capacity(file.instances.len());
    // Every scene id seen: its entry, or `None` when it was skipped.
    let mut index: HashMap<u64, Option<usize>> = HashMap::with_capacity(file.instances.len());
    let mut services_seen: HashSet<ClassId> = HashSet::new();
    let mut pending: Vec<PendingRef> = Vec::new();

    for instance in file.instances {
        let id = instance.id;
        if index.contains_key(&id) {
            return Err(SceneError::DuplicateId(id));
        }
        let place = place(dm, registry, &instance, &index, &mut services_seen, warnings);
        let Some((class, place)) = place else {
            index.insert(id, None);
            continue;
        };
        let class_name = registry.class(class).map_or("", |info| info.name());
        let entry_index = entries.len();
        let mut entry = Entry {
            scene_id: id,
            class,
            place,
            name: instance.name.map(Arc::from),
            values: Vec::new(),
            refs: Vec::new(),
            attributes: Vec::new(),
        };

        for (key, json) in &instance.properties {
            let found = registry
                .prop_id(class, key)
                .and_then(|prop| Some((prop, registry.prop(class, prop)?)))
                .filter(|(_, resolved)| is_saved(resolved));
            let Some((prop, resolved)) = found else {
                warnings.push(SceneWarning::UnknownProperty {
                    id,
                    class: class_name,
                    property: key.clone(),
                });
                continue;
            };
            let meta = resolved.meta;
            match value::read(registry, meta.ty, json) {
                Some(Read::Value(value)) => entry.values.push((prop, value)),
                Some(Read::Ref(target)) => pending.push(PendingRef {
                    entry: entry_index,
                    prop,
                    property: meta.name,
                    class: match meta.ty {
                        PropType::Ref(class) => class,
                        _ => "Instance",
                    },
                    target,
                }),
                None => warnings.push(SceneWarning::BadValue {
                    id,
                    class: class_name,
                    property: meta.name,
                    expected: meta.ty.to_string(),
                }),
            }
        }

        for (name, json) in &instance.attributes {
            match Attribute::read(json) {
                Some(value) => entry.attributes.push((name.clone(), value)),
                None => warnings.push(SceneWarning::BadAttribute { id, name: name.clone() }),
            }
        }

        index.insert(id, Some(entry_index));
        entries.push(entry);
    }

    // References last, so they may point at instances later in the file.
    for reference in pending {
        let declared = registry.class_id(reference.class);
        let target = index.get(&reference.target).copied().flatten().filter(|&target| {
            let class = entries.get(target).map(|e| e.class);
            match (declared, class) {
                (Some(declared), Some(class)) => registry.is_a(class, declared),
                (None, Some(_)) => true,
                (_, None) => false,
            }
        });
        let Some(entry) = entries.get_mut(reference.entry) else {
            continue;
        };
        match target {
            Some(target) => entry.refs.push((reference.prop, target)),
            None => warnings.push(SceneWarning::BadReference {
                id: entry.scene_id,
                class: registry.class(entry.class).map_or("", |info| info.name()),
                property: reference.property,
                target: reference.target,
            }),
        }
    }
    Ok(entries)
}

/// The class of `instance` and where it goes, or `None` (with a warning, unless its parent was already skipped)
/// when it cannot load.
fn place(
    dm: &DataModel,
    registry: &Registry,
    instance: &InstanceIn,
    index: &HashMap<u64, Option<usize>>,
    services_seen: &mut HashSet<ClassId>,
    warnings: &mut Vec<SceneWarning>,
) -> Option<(ClassId, Place)> {
    let id = instance.id;
    let parent = match instance.parent {
        None => None,
        Some(parent) => match index.get(&parent) {
            Some(Some(entry)) => Some(*entry),
            // Its parent was skipped, with a warning that covers everything under it.
            Some(None) => return None,
            None => {
                warnings.push(SceneWarning::Orphan { id, parent });
                return None;
            }
        },
    };
    let Some(info) = registry
        .class_id(&instance.class)
        .and_then(|class| registry.class(class))
    else {
        warnings.push(SceneWarning::UnknownClass {
            id,
            class: instance.class.clone(),
        });
        return None;
    };
    let class = info.id();
    let is_service = info.service_order().is_some();
    let place = match parent {
        None if is_service && info.persisted() && services_seen.insert(class) => dm.service(class).map(Place::Service),
        Some(parent) if !is_service && info.creatable() => Some(Place::Under(parent)),
        _ => None,
    };
    if place.is_none() {
        warnings.push(SceneWarning::Misplaced {
            id,
            class: instance.class.clone(),
        });
    }
    Some((class, place?))
}

/// Stage 2: creates every new instance outside the game and fills it in. Returns the instance of each entry. On an
/// error, destroys what it created.
fn stage(dm: &mut DataModel, entries: &[Entry]) -> Result<Vec<Id>, SceneError> {
    let mut targets = Vec::with_capacity(entries.len());
    let mut created = Vec::new();
    let staged = fill_staged(dm, entries, &mut targets, &mut created);
    if let Err(error) = staged {
        // Children before parents. An instance already gone with its parent is fine to skip.
        for &id in created.iter().rev() {
            if dm.contains(id) {
                dm.destroy(id).ok();
            }
        }
        return Err(error);
    }
    Ok(targets)
}

fn fill_staged(
    dm: &mut DataModel,
    entries: &[Entry],
    targets: &mut Vec<Id>,
    created: &mut Vec<Id>,
) -> Result<(), SceneError> {
    dm.reserve(entries.iter().filter(|e| matches!(e.place, Place::Under(_))).count());
    for entry in entries {
        let id = match entry.place {
            Place::Service(id) => id,
            Place::Under(_) => {
                let id = dm.create(entry.class)?;
                created.push(id);
                id
            }
        };
        targets.push(id);
    }
    for (entry, &id) in entries.iter().zip(targets.iter()) {
        let Place::Under(parent) = entry.place else {
            continue;
        };
        fill(dm, id, entry, targets)?;
        // Subtrees join now; the ones under a service wait for the commit.
        if let Some(Entry {
            place: Place::Under(_), ..
        }) = entries.get(parent)
            && let Some(&parent) = targets.get(parent)
        {
            dm.set_parent(id, Some(parent))?;
        }
    }
    Ok(())
}

/// Stage 3: replaces the saved services' contents and properties with the scene's.
fn commit(
    dm: &mut DataModel,
    registry: &Registry,
    services: &[Id],
    entries: &[Entry],
    targets: &[Id],
) -> Result<(), SceneError> {
    for &service in services {
        dm.destroy_children(service)?;
    }

    let by_service: HashMap<Id, &Entry> = entries
        .iter()
        .filter_map(|entry| match entry.place {
            Place::Service(id) => Some((id, entry)),
            Place::Under(_) => None,
        })
        .collect();
    for &service in services {
        let class = dm.class_of(service)?;
        let Some(info) = registry.class(class) else {
            continue;
        };
        let entry = by_service.get(&service).copied();
        // What the file does not set goes back to its default, so the service holds exactly what the scene says.
        let name = entry
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| Arc::from(info.name()));
        dm.set_name(service, name)?;
        for (i, resolved) in info.props().iter().enumerate() {
            let Ok(raw) = u16::try_from(i) else {
                continue;
            };
            let prop = PropId::new(raw);
            if !is_saved(resolved) || entry.is_some_and(|e| e.refs.iter().any(|&(p, _)| p == prop)) {
                continue;
            }
            let value = entry
                .and_then(|e| e.values.iter().rev().find(|&&(p, _)| p == prop))
                .map_or(&resolved.default, |(_, value)| value);
            dm.set(service, prop, value.clone())?;
        }
        let stale: Vec<String> = dm
            .attributes(service)?
            .iter()
            .map(|(name, _)| name.to_string())
            .filter(|name| entry.is_none_or(|e| !e.attributes.iter().any(|(n, _)| n == name)))
            .collect();
        for name in stale {
            dm.set_attribute(service, &name, None)?;
        }
        if let Some(entry) = entry {
            for (name, value) in &entry.attributes {
                dm.set_attribute(service, name, Some(value.clone()))?;
            }
        }
    }

    for (entry, &id) in entries.iter().zip(targets) {
        if let Place::Under(parent) = entry.place
            && let Some(Entry {
                place: Place::Service(service),
                ..
            }) = entries.get(parent)
        {
            dm.set_parent(id, Some(*service))?;
        }
    }
    for (entry, &id) in entries.iter().zip(targets) {
        if matches!(entry.place, Place::Service(_)) {
            set_refs(dm, id, entry, targets)?;
        }
    }
    Ok(())
}

/// Sets a staged instance's name, properties, attributes and references.
fn fill(dm: &mut DataModel, id: Id, entry: &Entry, targets: &[Id]) -> Result<(), SceneError> {
    if let Some(name) = &entry.name {
        dm.set_name(id, Arc::clone(name))?;
    }
    for (prop, value) in &entry.values {
        dm.set(id, *prop, value.clone())?;
    }
    for (name, value) in &entry.attributes {
        dm.set_attribute(id, name, Some(value.clone()))?;
    }
    set_refs(dm, id, entry, targets)
}

fn set_refs(dm: &mut DataModel, id: Id, entry: &Entry, targets: &[Id]) -> Result<(), SceneError> {
    for &(prop, target) in &entry.refs {
        if let Some(&target) = targets.get(target) {
            dm.set(id, prop, Value::Ref(Some(target)))?;
        }
    }
    Ok(())
}
