//! `BindableFunction`: a function one script exposes and other scripts on the same side call, waiting for its
//! results.

use metallic_core::{CallbackDesc, ClassDesc, MethodCall, MethodDesc, Param, ScriptError, ScriptValue, Ty};

/// Any number of values of any type: what `Invoke` takes and `OnInvoke` receives, and what they return.
static ARGS: &[Param] = &[Param::new("args", Ty::Variadic(&Ty::Any))];

/// `Invoke` calls a script function, which only a script host can reach: backends route it to the host through
/// `invokes_callback`, so this runs only for Rust callers.
fn invoke(_: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Err(ScriptError::runtime(
        "Invoke calls a script function, so it can only be called from a script",
    ))
}

/// A function scripts call through the instance.
pub static CLASS: ClassDesc = ClassDesc::new("BindableFunction", Some("Instance"))
    .describe(
        "A function that one script exposes and others call: a script assigns a function to OnInvoke, and Invoke \
         calls it and returns what it returns. It works between scripts on the same side (server or client).",
    )
    .creatable()
    .methods(&[MethodDesc::new("Invoke", invoke)
        .params(ARGS)
        .returns(Ty::Any)
        .invokes_callback("OnInvoke")
        .describe(
            "Calls the function assigned to OnInvoke with all the arguments given and returns its results. The \
             calling script waits while the function runs, even if it yields. If no function is assigned yet, the \
             caller waits until one is; an error in the function is raised in the caller.",
        )])
    .callbacks(&[CallbackDesc::new("OnInvoke")
        .params(ARGS)
        .returns(Ty::Any)
        .describe(
            "The function Invoke calls. Assign a function to it (or nil to remove it); it receives Invoke's \
             arguments, and what it returns is what Invoke returns. It cannot be read.",
        )]);

#[cfg(test)]
mod tests {
    use metallic_core::ScriptValue;

    use crate::shared_registry;
    use crate::testing::{call, game, new_in, workspace};

    #[test]
    fn bindable_function_has_invoke_and_on_invoke() {
        let registry = shared_registry().unwrap();
        let class = registry.class_id("BindableFunction").unwrap();
        let info = registry.class(class).unwrap();
        assert!(info.creatable());
        let invoke = registry.method(class, "Invoke").unwrap();
        assert_eq!(invoke.invokes, Some("OnInvoke"));
        assert_eq!(invoke.params, Some(super::ARGS));
        let on_invoke = registry.callback(class, "OnInvoke").unwrap();
        assert_eq!(on_invoke.params, Some(super::ARGS));
        assert!(registry.prop_id(class, "OnInvoke").is_none());
    }

    #[test]
    fn invoke_from_rust_says_it_needs_a_script() {
        let mut dm = game();
        let ws = workspace(&dm);
        let function = new_in(&mut dm, "BindableFunction", ws, "Ask");
        let error = call(&mut dm, function, "Invoke", vec![ScriptValue::Number(1.0)]).unwrap_err();
        assert!(error.to_string().contains("from a script"), "{error}");
    }
}
