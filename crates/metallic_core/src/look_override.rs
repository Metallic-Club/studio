//! Per-part graphics style: the `LookOverride` property of `BasePart` and `Model`
//! (`docs/design/building-tools.md` section 12.2).
//!
//! A part's effective style is its own `LookOverride` when that is not `inherit`, else the
//! value of the nearest ancestor `Model` that is not `inherit`, else the place Look
//! (`Lighting.Look`, #64). Only Models pass a style down: a part parented to a part does not
//! take its parent's style.
//!
//! [`resolve`] answers for one instance by walking its ancestors. [`LookOverrides`] keeps the
//! answer for every part and brings it up to date from the change journal, so a renderer can
//! read it per part without walking the tree each frame.
use crate::instance::{Changes, InstanceId, World};
use crate::value::Value;
use std::collections::HashMap;

/// The property name.
pub const PROPERTY: &str = "LookOverride";

/// The stored value that means "no override here".
pub const INHERIT: &str = "inherit";

/// Allowed stored values (Look ids), `inherit` first.
pub const LOOK_OVERRIDES: &[&str] = &[
    INHERIT,
    "realistic",
    "soft",
    "toon",
    "low-poly",
    "voxel",
    "pixel-3d",
    "retro-32",
];

/// Display names, in [`LOOK_OVERRIDES`] order.
const DISPLAY_NAMES: &[&str] = &[
    "Inherit",
    "Realistic",
    "Soft",
    "Toon",
    "Low-poly",
    "Voxel",
    "Pixel 3D",
    "Retro / PS1",
];

/// Further names accepted on write: the Look's own display name (Look spec, "Retro 32-bit").
const ALIASES: &[(&str, &str)] = &[("retro32bit", "retro-32")];

/// Lowercase letters and digits only: `"Retro / PS1"` -> `"retrops1"`.
fn compact(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The stored id for an id or display name, ignoring case, spaces and punctuation
/// (`"Retro / PS1"`, `"retro-32"` and `"Retro 32-bit"` all give `"retro-32"`). `None` when it
/// names no allowed style.
pub fn look_override_id(name: &str) -> Option<&'static str> {
    let key = compact(name);
    if key.is_empty() {
        return None;
    }
    LOOK_OVERRIDES
        .iter()
        .zip(DISPLAY_NAMES)
        .find(|(id, display)| compact(id) == key || compact(display) == key)
        .map(|(id, _)| *id)
        .or_else(|| ALIASES.iter().find(|(a, _)| *a == key).map(|(_, id)| *id))
}

/// The display name of a stored id (`"retro-32"` -> `"Retro / PS1"`).
pub fn display_name(id: &str) -> Option<&'static str> {
    LOOK_OVERRIDES
        .iter()
        .position(|v| *v == id)
        .map(|i| DISPLAY_NAMES[i])
}

/// An instance's own `LookOverride`, `None` when it is `inherit` or the instance has none.
pub fn own(world: &World, id: InstanceId) -> Option<&'static str> {
    match world.get_ref(id, PROPERTY) {
        Ok(Value::String(s)) if s != INHERIT => LOOK_OVERRIDES.iter().copied().find(|v| v == s),
        _ => None,
    }
}

/// The effective style of `id`: its own value, else the nearest ancestor Model's that is not
/// `inherit`. `None` means the place Look (also for instances without the property).
pub fn resolve(world: &World, id: InstanceId) -> Option<&'static str> {
    if let Some(v) = own(world, id) {
        return Some(v);
    }
    inherited(world, id)
}

/// What `id` would inherit: the nearest non-`inherit` value among its ancestor Models.
fn inherited(world: &World, id: InstanceId) -> Option<&'static str> {
    let mut cur = world.parent(id);
    while let Some(a) = cur {
        if world.is_a(a, "Model")
            && let Some(v) = own(world, a)
        {
            return Some(v);
        }
        cur = world.parent(a);
    }
    None
}

/// The resolved style of every `BasePart`, kept current from the change journal.
///
/// Only parts with an effective override and Models with their own value are stored, so a
/// world without overrides costs nothing but the journal scan.
#[derive(Debug, Default, Clone)]
pub struct LookOverrides {
    parts: HashMap<InstanceId, &'static str>,
    models: HashMap<InstanceId, &'static str>,
}

impl LookOverrides {
    /// An empty cache; call [`LookOverrides::rebuild`] before the first read.
    pub fn new() -> LookOverrides {
        LookOverrides::default()
    }

    /// The resolved style of a part; `None` means the place Look.
    pub fn get(&self, id: InstanceId) -> Option<&'static str> {
        self.parts.get(&id).copied()
    }

    /// Number of parts with an effective override.
    pub fn overridden_parts(&self) -> usize {
        self.parts.len()
    }

    /// Reads the whole world again.
    pub fn rebuild(&mut self, world: &World) {
        self.parts.clear();
        self.models.clear();
        let mut stack: Vec<(InstanceId, Option<&'static str>)> =
            world.roots().iter().rev().map(|&r| (r, None)).collect();
        while let Some((id, from_above)) = stack.pop() {
            let pass_down = self.visit(world, id, from_above);
            stack.extend(world.children(id).iter().rev().map(|&c| (c, pass_down)));
        }
    }

    /// Records `id` given what its ancestors pass down; returns what it passes to its children.
    fn visit(
        &mut self,
        world: &World,
        id: InstanceId,
        from_above: Option<&'static str>,
    ) -> Option<&'static str> {
        let mine = own(world, id);
        if world.is_a(id, "BasePart") {
            set_or_clear(&mut self.parts, id, mine.or(from_above));
            from_above
        } else if world.is_a(id, "Model") {
            set_or_clear(&mut self.models, id, mine);
            mine.or(from_above)
        } else {
            from_above
        }
    }

    /// Brings the cache up to date for the instances named by the journal (`Changes::Ids`) and
    /// returns the parts whose resolved style changed. Parts below a Model whose own value
    /// changed are included even though the journal names only the Model. On
    /// `Changes::Overflow` the cache is rebuilt and every part with an override before or
    /// after is returned.
    pub fn apply(&mut self, world: &World, changes: &Changes) -> Vec<InstanceId> {
        let ids = match changes {
            Changes::Ids(ids) => ids,
            Changes::Overflow => {
                let mut before: Vec<InstanceId> = self.parts.keys().copied().collect();
                self.rebuild(world);
                before.extend(self.parts.keys().copied());
                before.sort_unstable();
                before.dedup();
                return before;
            }
        };
        self.apply_ids(world, ids)
    }

    /// [`LookOverrides::apply`] for the ids of a `Changes::Ids`.
    pub fn apply_ids(&mut self, world: &World, ids: &[InstanceId]) -> Vec<InstanceId> {
        let mut changed = Vec::new();
        for &id in ids {
            if !world.is_alive(id) {
                if self.parts.remove(&id).is_some() {
                    changed.push(id);
                }
                self.models.remove(&id);
                continue;
            }
            if world.is_a(id, "BasePart") {
                let now = resolve(world, id);
                if self.parts.get(&id).copied() != now {
                    set_or_clear(&mut self.parts, id, now);
                    changed.push(id);
                }
            } else if world.is_a(id, "Model") {
                let now = own(world, id);
                if self.models.get(&id).copied() != now {
                    set_or_clear(&mut self.models, id, now);
                    self.refresh_below(world, id, &mut changed);
                }
            }
        }
        changed
    }

    /// Re-resolves every descendant of `model`, collecting the parts that changed.
    fn refresh_below(&mut self, world: &World, model: InstanceId, changed: &mut Vec<InstanceId>) {
        let pass_down = own(world, model).or_else(|| inherited(world, model));
        let mut stack: Vec<(InstanceId, Option<&'static str>)> = world
            .children(model)
            .iter()
            .map(|&c| (c, pass_down))
            .collect();
        while let Some((id, from_above)) = stack.pop() {
            let before = self.parts.get(&id).copied();
            let pass = self.visit(world, id, from_above);
            if world.is_a(id, "BasePart") && self.parts.get(&id).copied() != before {
                changed.push(id);
            }
            stack.extend(world.children(id).iter().map(|&c| (c, pass)));
        }
    }
}

fn set_or_clear(
    map: &mut HashMap<InstanceId, &'static str>,
    id: InstanceId,
    v: Option<&'static str>,
) {
    match v {
        Some(v) => {
            map.insert(id, v);
        }
        None => {
            map.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_display_names_normalise() {
        assert_eq!(look_override_id("Retro / PS1"), Some("retro-32"));
        assert_eq!(look_override_id("retro-32"), Some("retro-32"));
        assert_eq!(look_override_id("Retro 32-bit"), Some("retro-32"));
        assert_eq!(look_override_id("Pixel 3D"), Some("pixel-3d"));
        assert_eq!(look_override_id("LOW-POLY"), Some("low-poly"));
        assert_eq!(look_override_id("Inherit"), Some("inherit"));
        assert_eq!(look_override_id("comic"), None);
        assert_eq!(look_override_id(""), None);
        assert_eq!(look_override_id(" / "), None);
        for id in LOOK_OVERRIDES {
            assert_eq!(look_override_id(id), Some(*id));
            assert_eq!(look_override_id(display_name(id).unwrap()), Some(*id));
        }
    }
}
