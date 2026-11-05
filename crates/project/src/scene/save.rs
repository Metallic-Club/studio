//! [`save_scene`]: a game's saved services to scene text.

use std::collections::{BTreeMap, HashMap};

use metallic_core::{ClassId, DataModel, Id, PropId, Registry};
use serde::Serialize;

use super::value::{self, Attribute, Encoded, Written};
use super::{SCENE_FORMAT, SavedScene, SceneError, SceneIds, SceneWarning, is_saved, json};

/// The whole file, as written.
#[derive(Serialize)]
struct FileOut<'a> {
    format: u32,
    instances: Vec<InstanceOut<'a>>,
}

/// One instance, as written. Field order is the key order in the file.
#[derive(Serialize)]
struct InstanceOut<'a> {
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<u64>,
    class: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    properties: BTreeMap<&'static str, Written>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    attributes: BTreeMap<&'a str, Attribute>,
}

/// What saving needs to know about a class, looked up once per class.
struct ClassPlan {
    name: &'static str,
    /// Its `Archivable` property, if it has one.
    archivable: Option<PropId>,
    /// Its saved properties.
    saved: Vec<(PropId, &'static str)>,
}

impl ClassPlan {
    fn new(registry: &Registry, class: ClassId) -> Self {
        let info = registry.class(class);
        let saved = info
            .map(|info| {
                info.props()
                    .iter()
                    .enumerate()
                    .filter(|(_, prop)| is_saved(prop))
                    .filter_map(|(i, prop)| Some((PropId::new(u16::try_from(i).ok()?), prop.meta.name)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            name: info.map_or("", |info| info.name()),
            archivable: info.and_then(|info| info.prop_id("Archivable")),
            saved,
        }
    }
}

/// Writes `dm`'s scene: every service whose class is `persisted`, in service order, with everything under it.
///
/// Instances whose `Archivable` is off are left out with everything under them, as `Clone` leaves them out. Each
/// instance keeps its id in `ids` and new ones get the next free id; `ids` is updated to hold exactly the saved
/// instances. A reference to an instance that is not saved is written as nil, with a warning.
pub fn save_scene(dm: &DataModel, ids: &mut SceneIds) -> Result<SavedScene, SceneError> {
    let registry = dm.registry();
    let mut plans: HashMap<ClassId, ClassPlan> = HashMap::new();

    // Which instances are saved, parents first, with their parents.
    let mut order: Vec<(Id, Option<Id>)> = Vec::new();
    let mut stack: Vec<(Id, Option<Id>)> = Vec::new();
    for &class in registry.services() {
        if !registry.class(class).is_some_and(|info| info.persisted()) {
            continue;
        }
        let Some(service) = dm.service(class) else {
            continue;
        };
        stack.push((service, None));
        while let Some((id, parent)) = stack.pop() {
            order.push((id, parent));
            // Reversed, so the first child comes off the stack first.
            for child in dm.children(id)?.rev() {
                let class = dm.class_of(child)?;
                let plan = plans.entry(class).or_insert_with(|| ClassPlan::new(registry, class));
                let archivable = match plan.archivable {
                    Some(prop) => dm.bool(child, prop)?,
                    None => true,
                };
                if archivable {
                    stack.push((child, Some(id)));
                }
            }
        }
    }

    // Scene ids: kept where known, the next free one otherwise, in tree order so a first save is numbered 1, 2, 3...
    let mut saved_ids = SceneIds {
        ids: HashMap::with_capacity(order.len()),
        next: ids.next,
    };
    // `ids.next` is above every known id, so a fresh one never collides with a kept one.
    for &(id, _) in &order {
        let scene_id = ids.get(id).unwrap_or(saved_ids.next);
        saved_ids.insert(id, scene_id);
    }

    let mut warnings = Vec::new();
    let mut instances = Vec::with_capacity(order.len());
    for &(id, parent) in &order {
        let class = dm.class_of(id)?;
        let plan = plans.entry(class).or_insert_with(|| ClassPlan::new(registry, class));
        let name = dm.name(id)?;
        let mut properties = BTreeMap::new();
        for &(prop, prop_name) in &plan.saved {
            let value = dm.get_ref(id, prop)?;
            if registry.prop(class, prop).is_some_and(|r| r.default == *value) {
                continue;
            }
            match value::write(value, |target| saved_ids.get(target)) {
                Encoded::Value(written) => {
                    properties.insert(prop_name, written);
                }
                Encoded::NotSaved => warnings.push(SceneWarning::ReferenceNotSaved {
                    instance: dm.full_name(id)?,
                    property: prop_name,
                }),
            }
        }
        let attributes = dm
            .attributes(id)?
            .iter()
            .filter_map(|(name, value)| Some((&**name, Attribute::from_value(value)?)))
            .collect();
        instances.push(InstanceOut {
            id: saved_ids.get(id).unwrap_or_default(),
            parent: parent.and_then(|p| saved_ids.get(p)),
            class: plan.name,
            name: (name != plan.name).then_some(name),
            properties,
            attributes,
        });
    }

    let file = FileOut {
        format: SCENE_FORMAT,
        instances,
    };
    let text = json::to_text(&file).map_err(|error| SceneError::Write(error.to_string()))?;
    *ids = saved_ids;
    Ok(SavedScene { text, warnings })
}
