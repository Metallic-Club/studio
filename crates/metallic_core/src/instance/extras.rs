//! Attributes: named values attached to any instance, outside the class registry.
//!
//! They live in a boxed side record of the node ([`Extras`]), so an instance without attributes
//! pays one `None` pointer and the property paths (`get`, `set`) never look at them. Changes are
//! recorded in the ordinary change journal (the instance counts as changed) and in the
//! attribute log, which tells readers *which* attribute changed: scripts use it to fire
//! `AttributeChanged`.
//!
//! Replication (#305) will treat an attribute as a property-like value of its instance; the
//! registry there must list the allowed [`Value`] types the same way [`check_attribute`] does.

use std::collections::{BTreeMap, VecDeque};

use super::{InstanceId, World};
use crate::CoreError;
use crate::value::Value;

/// Longest attribute name, in bytes (Roblox: 100).
pub const MAX_ATTRIBUTE_NAME: usize = 100;
/// Longest string attribute value, in bytes.
pub const MAX_ATTRIBUTE_STRING: usize = 65_536;
/// Most attributes one instance can hold.
pub const MAX_ATTRIBUTES: usize = 1024;
/// Most attribute changes the log keeps before the oldest half is dropped.
const LOG_LIMIT: usize = 4096;

/// What a node holds besides class, parent, children and registry properties.
#[derive(Clone, Default)]
pub(super) struct Extras {
    pub(super) attributes: BTreeMap<String, Value>,
}

impl Extras {
    fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }
}

/// A position in a world's attribute log; see [`World::attribute_changes_since`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeCursor {
    world: u64,
    seq: u64,
}

/// One attribute that changed (set, changed value, or removed). Read the current value from the
/// world: `None` there means it was removed (or the instance is gone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeChange {
    /// The instance.
    pub id: InstanceId,
    /// The attribute name.
    pub name: String,
}

/// Attribute changes since an [`AttributeCursor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeChanges {
    /// In order, one entry per accepted change.
    Changes(Vec<AttributeChange>),
    /// The log no longer reaches the cursor (or it is from another world): reread everything.
    Overflow,
}

/// The bounded log of attribute changes.
#[derive(Default)]
pub(super) struct AttributeLog {
    entries: VecDeque<(u64, AttributeChange)>,
    seq: u64,
    /// Entries with a sequence number up to this were dropped.
    floor: u64,
}

impl AttributeLog {
    fn push(&mut self, change: AttributeChange) {
        self.seq += 1;
        self.entries.push_back((self.seq, change));
        if self.entries.len() > LOG_LIMIT {
            let cut = self.entries.len() / 2;
            self.floor = self.entries[cut - 1].0;
            self.entries.drain(..cut);
        }
    }
}

/// Whether `name` is a valid attribute name: 1 to [`MAX_ATTRIBUTE_NAME`] bytes of ASCII letters,
/// digits and underscores, and not starting with `RBX` (reserved, any case).
pub fn check_attribute_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("attribute names cannot be empty".into());
    }
    if name.len() > MAX_ATTRIBUTE_NAME {
        return Err(format!(
            "attribute names are at most {MAX_ATTRIBUTE_NAME} characters"
        ));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("attribute names can only contain letters, digits and underscores".into());
    }
    if name.len() >= 3 && name[..3].eq_ignore_ascii_case("RBX") {
        return Err("attribute names cannot start with RBX".into());
    }
    Ok(())
}

/// Checks that `value` can be stored as an attribute: a bool, number, string, Vector3, Color3,
/// UDim2 or CFrame, with finite numbers and a string of at most [`MAX_ATTRIBUTE_STRING`] bytes.
/// `Nil` is accepted (it removes the attribute).
pub fn check_attribute_value(value: &Value) -> Result<(), String> {
    match value {
        Value::Ref(_) => {
            return Err("attributes cannot hold instance references".into());
        }
        Value::String(s) if s.len() > MAX_ATTRIBUTE_STRING => {
            return Err(format!(
                "string attributes are at most {MAX_ATTRIBUTE_STRING} bytes"
            ));
        }
        _ => {}
    }
    if !value.has_finite_components() {
        return Err("all numeric components must be finite".into());
    }
    Ok(())
}

fn invalid(name: &str, reason: String) -> CoreError {
    CoreError::InvalidValue {
        property: name.to_string(),
        reason,
    }
}

impl World {
    /// Sets attribute `name` of `id` to `value`; `Value::Nil` removes it. Setting the value it
    /// already has (or removing one that is not there) changes nothing and records nothing.
    ///
    /// Errors: [`CoreError::InvalidInstance`]; [`CoreError::InvalidValue`] for a bad name (see
    /// [`check_attribute_name`]), a value type attributes cannot hold, a non-finite number, an
    /// oversized string, or an instance that already holds [`MAX_ATTRIBUTES`] attributes.
    pub fn set_attribute(
        &mut self,
        id: InstanceId,
        name: &str,
        value: Value,
    ) -> Result<(), CoreError> {
        if self.node(id).is_none() {
            return Err(CoreError::InvalidInstance);
        }
        check_attribute_name(name).map_err(|e| invalid(name, e))?;
        check_attribute_value(&value).map_err(|e| invalid(name, e))?;
        let removing = matches!(value, Value::Nil);
        let node = self.node_mut(id).ok_or(CoreError::InvalidInstance)?;
        if removing {
            let Some(extras) = node.extras.as_mut() else {
                return Ok(());
            };
            if extras.attributes.remove(name).is_none() {
                return Ok(());
            }
            if extras.is_empty() {
                node.extras = None;
            }
        } else {
            let extras = node.extras.get_or_insert_with(Default::default);
            match extras.attributes.get(name) {
                Some(old) if *old == value => return Ok(()),
                None if extras.attributes.len() >= MAX_ATTRIBUTES => {
                    if extras.is_empty() {
                        node.extras = None;
                    }
                    return Err(invalid(
                        name,
                        format!("an instance holds at most {MAX_ATTRIBUTES} attributes"),
                    ));
                }
                _ => {}
            }
            extras.attributes.insert(name.to_string(), value);
        }
        self.record(id);
        self.attribute_log.push(AttributeChange {
            id,
            name: name.to_string(),
        });
        Ok(())
    }

    /// The value of attribute `name`, or `None` when it is not set (or `id` is not alive).
    pub fn attribute(&self, id: InstanceId, name: &str) -> Option<&Value> {
        self.node(id)?.extras.as_ref()?.attributes.get(name)
    }

    /// All attributes of `id`, sorted by name; empty for an instance without any.
    pub fn attributes(&self, id: InstanceId) -> impl Iterator<Item = (&str, &Value)> {
        self.node(id)
            .and_then(|n| n.extras.as_ref())
            .into_iter()
            .flat_map(|e| e.attributes.iter().map(|(k, v)| (k.as_str(), v)))
    }

    /// The current end of the attribute log.
    pub fn attribute_cursor(&self) -> AttributeCursor {
        AttributeCursor {
            world: self.uid,
            seq: self.attribute_log.seq,
        }
    }

    /// Attribute changes since `cursor`, plus a cursor to continue from. Needs only `&World`, so
    /// several readers (the server VM, the client VM, replication) each keep their own cursor.
    /// The log is bounded: when it no longer reaches `cursor`, or `cursor` came from another
    /// world, the result is [`AttributeChanges::Overflow`].
    pub fn attribute_changes_since(
        &self,
        cursor: AttributeCursor,
    ) -> (AttributeChanges, AttributeCursor) {
        let now = self.attribute_cursor();
        if cursor.world != self.uid || cursor.seq < self.attribute_log.floor {
            return (AttributeChanges::Overflow, now);
        }
        let log = &self.attribute_log.entries;
        let start = log.partition_point(|(seq, _)| *seq <= cursor.seq);
        let changes = log.iter().skip(start).map(|(_, c)| c.clone()).collect();
        (AttributeChanges::Changes(changes), now)
    }
}
