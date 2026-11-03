//! The object model every other crate stands on.
//!
//! Owns: the value types and math (`Id`, `Value`, `Vector3`, `CFrame`, `Color3`), the language-neutral calling
//! convention (`ScriptValue`, `MethodCall`), the class registry and its descriptors (`ClassDesc`, `PropMeta`, ...),
//! the `DataModel` instance tree and signals.
//!
//! Must not depend on: any other Metallic crate, or any GPU, window, scripting VM or OS-specific code. Only small
//! portable crates are allowed (`glam`, `serde`, `thiserror`). See `docs/adr/0002-object-model.md`.

// Lets the test class set, shared with the integration tests and benches, name this crate the way they do.
#[cfg(test)]
extern crate self as metallic_core;

mod args;
mod dm;
mod enum_item;
mod id;
mod math;
mod registry;
mod script_value;
mod value;

pub use glam;

pub use args::Args;
pub use dm::{
    BuiltinSignal, Children, ConnectionId, ConsumerId, DataModel, Delivery, Descendants, DirtyBits, DmError,
    ListenerKey, MAX_CONSUMERS,
};
pub use enum_item::EnumItem;
pub use id::Id;
pub use math::{CFrame, Color3, Vector2, Vector3};
pub use registry::{
    ClassBehavior, ClassDesc, ClassId, ClassInfo, ComponentDesc, Computed, DefaultValue, EnumDesc, Getter, MemberKind,
    MethodCall, MethodDesc, MethodFn, Owner, Param, PropFlags, PropId, PropMeta, Registry, RegistryError,
    RegistryErrors, RegistryInput, ResolvedMember, ResolvedMethod, ResolvedProp, ResolvedSignal, Setter, SignalDesc,
    SignalId, Ty, Widget,
};
pub use script_value::{ScriptError, ScriptValue};
pub use value::{PropType, Value};
