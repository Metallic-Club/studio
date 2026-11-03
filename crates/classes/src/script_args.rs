//! Reading script arguments that several class files share, such as a class name passed as a string.

use metallic_core::{ClassId, MethodCall, ScriptError};

/// Argument `index` as a class name, looked up in the registry. A name that is not a class is a script error
/// (`unknown class 'Foo'`) rather than a silent `false` or `nil`, so a typo shows up where it was made.
pub(crate) fn class_arg(call: &MethodCall<'_>, index: usize) -> Result<ClassId, ScriptError> {
    let name = call.args.string(index)?;
    call.dm
        .registry()
        .class_id(&name)
        .ok_or_else(|| ScriptError::runtime(format!("unknown class '{name}'")))
}
