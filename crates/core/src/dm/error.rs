//! `DmError`: why a data model call was refused.

use thiserror::Error;

use crate::{Id, PropType, ScriptError};

/// Why a [`DataModel`](crate::DataModel) call was refused. Messages are written for the game developer who made
/// the call, usually from a script.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum DmError {
    /// The id names no live instance: it was destroyed, or it came from another data model.
    #[error("the instance no longer exists (it was destroyed, or the id {0:?} is not from this game)")]
    NoInstance(Id),
    /// The instance was destroyed. Its last state can still be read (something pins it, or its events are being
    /// delivered), but it cannot be changed, moved, cloned or connected to.
    #[error("the instance was destroyed")]
    Destroyed(Id),
    /// The class id is not in the registry.
    #[error("there is no class with id {0}")]
    UnknownClass(u16),
    /// The class has no property with that id or name.
    #[error("{prop} is not a valid property of {class}")]
    UnknownProperty {
        /// The instance's class.
        class: &'static str,
        /// The property name, or `#<id>` when it was asked for by id.
        prop: String,
    },
    /// The class has no visible signal with that id or name.
    #[error("{signal} is not a valid signal of {class}")]
    UnknownSignal {
        /// The instance's class.
        class: &'static str,
        /// The signal name, or `#<id>` when it was asked for by id.
        signal: String,
    },
    /// The value has the wrong type for the property.
    #[error("{prop} expects {expected}, got {got}")]
    TypeMismatch {
        /// The property's name.
        prop: &'static str,
        /// The declared type.
        expected: PropType,
        /// What was given, named like [`PropType`]'s `Display` (`number`, `Enum.Material`, `nil`, ...).
        got: String,
    },
    /// A number, vector, `CFrame` or colour component was NaN or infinite.
    #[error("{prop} must be a finite value (got NaN or infinity)")]
    NotFinite {
        /// The property's (or attribute's) name.
        prop: String,
    },
    /// Scripts and Studio may not create this class.
    #[error("{class} cannot be created")]
    NotCreatable {
        /// The class's name.
        class: &'static str,
    },
    /// The class or instance is a service, which only the root owns and which cannot be created, moved or destroyed.
    #[error("{class} is a service; services cannot be created, moved or destroyed")]
    IsService {
        /// The service's class name.
        class: &'static str,
    },
    /// The class is not a service, so `create_service` refused it.
    #[error("{class} is not a service")]
    NotAService {
        /// The class's name.
        class: &'static str,
    },
    /// The instance is the root (`game`), which cannot be moved, cloned or destroyed.
    #[error("the root ({class}) cannot be moved, cloned or destroyed")]
    IsRoot {
        /// The root's class name.
        class: &'static str,
    },
    /// `create_root` was called twice.
    #[error("the game already has a root")]
    RootExists,
    /// A service was created before the root.
    #[error("services need a root: create the root first")]
    NoRoot,
    /// The service already exists.
    #[error("{class} already exists; a service is created only once")]
    DuplicateService {
        /// The service's class name.
        class: &'static str,
    },
    /// The new parent is the instance itself or one of its descendants.
    #[error("cannot set the parent of {name}: the new parent is {name} itself or one of its descendants")]
    WouldCreateCycle {
        /// The name of the instance being moved.
        name: String,
    },
    /// The property is computed and has no setter.
    #[error("{prop} of {class} cannot be written")]
    ReadOnlyComputed {
        /// The instance's class.
        class: &'static str,
        /// The property's name.
        prop: &'static str,
    },
    /// The property is computed, so there is no stored value to borrow; `DataModel::get` computes it.
    #[error("{prop} of {class} is computed; read it with get")]
    ComputedRead {
        /// The instance's class.
        class: &'static str,
        /// The property's name.
        prop: &'static str,
    },
    /// A reference property was given an instance of the wrong class.
    #[error("{prop} expects {expected}, got {got}")]
    WrongRefClass {
        /// The property's name.
        prop: &'static str,
        /// The class the property declares.
        expected: &'static str,
        /// The class of the instance given.
        got: &'static str,
    },
    /// A reference property was given an instance that has been destroyed.
    #[error("{prop} cannot refer to an instance that was destroyed")]
    DestroyedRef {
        /// The property's name.
        prop: &'static str,
    },
    /// The enum has no item of that name.
    #[error("{prop} expects an item of Enum.{enum_type}, and Enum.{enum_type}.{item} does not exist")]
    UnknownEnumItem {
        /// The property's name.
        prop: &'static str,
        /// The enum type.
        enum_type: &'static str,
        /// The item name given.
        item: String,
    },
    /// Attributes hold plain values; instance references are not allowed.
    #[error("attribute {name} cannot hold an instance reference")]
    RefAttribute {
        /// The attribute's name.
        name: String,
    },
    /// Every change-consumer slot is taken.
    #[error("at most {max} change consumers can be registered")]
    TooManyConsumers {
        /// The limit.
        max: usize,
    },
    /// A computed property's setter failed.
    #[error(transparent)]
    Script(#[from] ScriptError),
}

/// Lets a method written against the data model use `?` and hand the message to the script.
impl From<DmError> for ScriptError {
    fn from(error: DmError) -> Self {
        match error {
            DmError::Script(inner) => inner,
            DmError::TypeMismatch { expected, got, .. } => ScriptError::wrong_type(expected.to_string(), got),
            other => ScriptError::runtime(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_read_well() {
        let err = DmError::TypeMismatch {
            prop: "Transparency",
            expected: PropType::Number,
            got: "string".into(),
        };
        assert_eq!(err.to_string(), "Transparency expects number, got string");
        let err = DmError::UnknownEnumItem {
            prop: "Material",
            enum_type: "Material",
            item: "Cheese".into(),
        };
        assert_eq!(
            err.to_string(),
            "Material expects an item of Enum.Material, and Enum.Material.Cheese does not exist"
        );
        assert_eq!(
            DmError::IsService { class: "Workspace" }.to_string(),
            "Workspace is a service; services cannot be created, moved or destroyed"
        );
    }

    #[test]
    fn converts_to_script_errors() {
        let mismatch = DmError::TypeMismatch {
            prop: "Size",
            expected: PropType::Vector3,
            got: "number".into(),
        };
        assert_eq!(
            ScriptError::from(mismatch),
            ScriptError::wrong_type("Vector3", "number")
        );
        let inner = ScriptError::runtime("nope");
        assert_eq!(ScriptError::from(DmError::from(inner.clone())), inner);
        assert_eq!(
            ScriptError::from(DmError::RootExists),
            ScriptError::runtime("the game already has a root")
        );
    }
}
