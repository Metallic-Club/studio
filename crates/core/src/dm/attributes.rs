//! Attributes: user-defined name → value pairs on any instance (`GetAttribute` / `SetAttribute`).
//!
//! An instance without attributes pays one pointer for them. Attributes hold plain values: anything but an
//! instance reference.

use std::sync::Arc;

use super::props::is_finite;
use super::storage::Attribute;
use super::{DataModel, DirtyBits, DmError, Notice};
use crate::{Id, Value};

impl DataModel {
    /// The attribute `name` of the instance, or `None` when it is not set.
    pub fn attribute(&self, id: Id, name: &str) -> Result<Option<&Value>, DmError> {
        Ok(self.attributes(id)?.iter().find(|(n, _)| &**n == name).map(|(_, v)| v))
    }

    /// Every attribute of the instance, in the order they were first set.
    pub fn attributes(&self, id: Id) -> Result<&[Attribute], DmError> {
        Ok(self.body(id)?.attributes.as_deref().map_or(&[], Vec::as_slice))
    }

    /// Sets the attribute `name` to `value`, or removes it with `None`. Setting the current value does nothing.
    /// Refuses instance references and non-finite numbers.
    pub fn set_attribute(&mut self, id: Id, name: &str, value: Option<Value>) -> Result<(), DmError> {
        if let Some(value) = &value {
            if matches!(value, Value::Ref(_)) {
                return Err(DmError::RefAttribute { name: name.to_owned() });
            }
            if !is_finite(value) {
                return Err(DmError::NotFinite { prop: name.to_owned() });
            }
        }
        let Some(slot) = self.slots.get_mut(id) else {
            return Err(DmError::NoInstance(id));
        };
        let Some(body) = slot.body.as_mut() else {
            return Err(DmError::NoInstance(id));
        };
        let existing = body
            .attributes
            .as_ref()
            .and_then(|list| list.iter().position(|(n, _)| &**n == name));
        let key: Arc<str> = match (existing, value) {
            (None, None) => return Ok(()),
            (Some(i), None) => {
                let Some(list) = body.attributes.as_mut() else {
                    return Ok(());
                };
                let (key, _) = list.remove(i);
                if list.is_empty() {
                    body.attributes = None;
                }
                key
            }
            (Some(i), Some(value)) => {
                let Some(entry) = body.attributes.as_mut().and_then(|list| list.get_mut(i)) else {
                    return Ok(());
                };
                if entry.1 == value {
                    return Ok(());
                }
                entry.1 = value;
                Arc::clone(&entry.0)
            }
            (None, Some(value)) => {
                let key: Arc<str> = Arc::from(name);
                body.attributes
                    .get_or_insert_with(Box::default)
                    .push((Arc::clone(&key), value));
                key
            }
        };
        self.changes.mark(slot, id, DirtyBits::PROPERTY);
        self.notify(Notice::AttributeChanged { id, name: key });
        Ok(())
    }
}
