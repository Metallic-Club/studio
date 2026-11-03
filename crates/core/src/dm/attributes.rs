//! Attributes: user-defined name → value pairs on any instance (`GetAttribute` / `SetAttribute`).
//!
//! An instance with neither attributes nor a name of its own pays one pointer for both. Attributes hold plain values: anything but an
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
        Ok(self.view(id)?.body.attributes())
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
            return Err(self.not_live(id));
        };
        let Some(body) = slot.body.as_mut() else {
            return Err(self.not_live(id));
        };
        let existing = body.attributes().iter().position(|(n, _)| &**n == name);
        let key: Arc<str> = match (existing, value) {
            (None, None) => return Ok(()),
            (Some(i), None) => {
                let (key, _) = body.attributes_mut().remove(i);
                body.drop_empty_extras();
                key
            }
            (Some(i), Some(value)) => {
                let Some(entry) = body.attributes_mut().get_mut(i) else {
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
                body.attributes_mut().push((Arc::clone(&key), value));
                key
            }
        };
        self.changes.mark(slot, id, DirtyBits::PROPERTY);
        self.notify(Notice::AttributeChanged { id, name: key });
        Ok(())
    }
}
