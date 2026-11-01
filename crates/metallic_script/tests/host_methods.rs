//! Every method core declares `Host` has a body in this crate, or is listed as not migrated (#461).
use metallic_core::classes::{ApiError, CallOutcome, HostMethodError};
use metallic_script::{HostCall, NOT_MIGRATED_HOST_METHODS, host_methods};

fn body(_: &mut HostCall<'_>) -> Result<CallOutcome, ApiError> {
    Ok(true.into())
}

#[test]
fn every_declared_host_method_has_an_implementation() {
    let methods = host_methods().expect("every body names a declared Host method");
    methods
        .check_complete(NOT_MIGRATED_HOST_METHODS)
        .expect("implement it, or list it while the old tables dispatch it");
}

#[test]
fn the_rule_sees_a_method_missing_from_the_list() {
    let methods = host_methods().unwrap();
    assert_eq!(
        methods.check_complete(&[("PVInstance", "GetPivot")]),
        Err(HostMethodError::Missing {
            class: "PVInstance",
            method: "PivotTo",
        })
    );
}

#[test]
fn the_list_can_only_shrink() {
    let mut methods = host_methods().unwrap();
    methods.implement("PVInstance", "GetPivot", body).unwrap();
    assert_eq!(
        methods.check_complete(NOT_MIGRATED_HOST_METHODS),
        Err(HostMethodError::NeedlessEntry {
            class: "PVInstance",
            method: "GetPivot",
        })
    );
    methods.implement("PVInstance", "PivotTo", body).unwrap();
    assert_eq!(methods.check_complete(&[]), Ok(()));
}

#[test]
fn an_implementation_must_name_a_declared_host_method() {
    let mut methods = host_methods().unwrap();
    for (class, method) in [
        ("RemoteEvent", "NoSuchMethod"),
        ("NoSuchClass", "GetPivot"),
        // Part inherits it; PVInstance declares it.
        ("Part", "GetPivot"),
    ] {
        assert_eq!(
            methods.implement(class, method, body),
            Err(HostMethodError::NotDeclared { class, method })
        );
    }
}
