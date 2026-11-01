//! The methods core declares `Host` and this crate implements: they fire remotes, park threads,
//! call back into script code or read host state. Design: `docs/design/class-system.md` section 5.6.
//!
//! Nothing dispatches through this table yet: the Luau binding starts to with #462, and methods
//! move here from the old tables with #465-#467. `tests/host_methods.rs` holds the completeness
//! rule.
use metallic_core::classes::{self, HostMethodError, HostMethods};

/// What only the script host has. Empty until #462 gives the binding a reason to fill it.
pub struct HostState(());

/// One call of a method this crate implements.
pub type HostCall<'a> = classes::HostCall<'a, HostState>;

/// A method body this crate implements. A WASM binding will call the same functions.
pub type HostMethodFn = classes::HostMethodFn<HostState>;

/// `Host` methods still dispatched by the old tables, as `(class, method)`. An entry leaves when
/// its method gets a body here or becomes `Native`; the completeness test fails on a stale one.
pub const NOT_MIGRATED_HOST_METHODS: &[(&str, &str)] = &[
    // `api/pivot.rs` until #465.
    ("PVInstance", "GetPivot"),
    ("PVInstance", "PivotTo"),
];

/// Every `Host` method body of this crate, registered by the name of its declaration.
pub fn host_methods() -> Result<HostMethods<HostState>, HostMethodError> {
    // #465-#467 add `methods.implement("RemoteEvent", "FireServer", fire_server)?;` here.
    Ok(HostMethods::new(classes::registry()))
}
