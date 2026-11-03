//! `Instance` userdata: members by name through the registry, cached per (class, atom).
//!
//! An instance is a tagged userdata holding its [`Id`]. `part.Position` goes `__index` → the class's member cache
//! (indexed by the string's atom) → `DataModel::get` → a native vector, with no hashing and no allocation.
//! `part:IsA("BasePart")` goes through `__namecall` and the class's `MethodDesc`, as every language's calls do.
//!
//! # Destroyed instances
//!
//! A userdata pins its instance (see `ffi`'s module docs), so after `part:Destroy()` the data model still answers
//! for it, and these bindings follow the data model's contract (`metallic_core`'s `dm/pins.rs`):
//!
//! | Member | After `Destroy()` |
//! | --- | --- |
//! | `Name`, `ClassName`, every other property | its last value |
//! | `Parent` | `nil` |
//! | `IsA`, `GetFullName` (the name alone), `GetAttribute(s)`, `IsDescendantOf` / `IsAncestorOf` (`false`) | work |
//! | `GetChildren`, `GetDescendants` / `FindFirstChild...`, `FindFirstAncestor`, `part.Child` | empty / `nil` / not a member |
//! | `Destroy`, `ClearAllChildren` | do nothing |
//! | writing a property (`Parent` included), `SetAttribute`, `Clone`, `Connect` | error: `cannot set X: the instance was destroyed`, or `the instance was destroyed` |
//! | `==`, `tostring` | work |

use std::ffi::c_int;

use metallic_core::{Args as MethodArgs, DataModel, DmError, Id, MethodCall};
use metallic_script::{SignalRef, YieldReason};

use crate::convert::{read_script, read_value};
use crate::ffi::{Args, Call, Kind, Out, Push, Raise, Ret, SignalHandle, lua_fn, with_ctx};
use crate::shared::{GET_PROPERTY_CHANGED_SIGNAL, Member, Shared};

lua_fn!(pub(crate) index => index_body);
lua_fn!(pub(crate) new_index => new_index_body);
lua_fn!(pub(crate) namecall => namecall_body);
lua_fn!(pub(crate) method_value => method_value_body);
lua_fn!(pub(crate) eq => eq_body);
lua_fn!(pub(crate) to_string => to_string_body);
lua_fn!(pub(crate) new => new_body);

/// `X is not a valid member of Part "Workspace.Part"`, as Roblox words it.
fn not_member(dm: &DataModel, id: Id, key: &str) -> Raise {
    let class = dm.class_name(id).unwrap_or("Instance");
    let path = dm.full_name(id).unwrap_or_default();
    Raise(format!("{key} is not a valid member of {class} \"{path}\""))
}

/// The instance at `idx`, or an error saying the member was not called on one.
fn this(args: &Args<'_>, idx: c_int, member: &str) -> Result<Id, Raise> {
    args.payload::<Id>(idx).ok_or_else(|| {
        Raise(format!(
            "Expected ':' not '.' calling member function {member} (got {})",
            args.type_name(idx)
        ))
    })
}

/// `instance.Key`: a property, a method as a function value, a signal, or else a child named `Key`.
fn index_body(call: &Call<'_>) -> Ret {
    let args = &call.args;
    let shared = call.shared()?;
    let Some(id) = args.payload::<Id>(1) else {
        return Err(Raise::from("attempt to index a non-instance"));
    };
    let Some((atom, key)) = args.str_atom(2) else {
        return Err(Raise(format!("invalid member key of type {}", args.type_name(2))));
    };
    call.with_ctx(|ctx| {
        let dm = &*ctx.dm;
        let class = dm.class_of(id)?;
        let member = shared.member(class, atom, key);
        let push = match member {
            Member::Prop(prop) if prop.computed => Push::Value(dm.get(id, prop.id)?),
            Member::Prop(prop) => Push::Value(dm.get_ref(id, prop.id)?.clone()),
            Member::Method(method) => Push::InstanceMethod(method.name),
            Member::GetPropertyChangedSignal => Push::InstanceMethod(GET_PROPERTY_CHANGED_SIGNAL),
            Member::Signal(signal) => Push::Signal(SignalHandle {
                target: id,
                signal: SignalRef::Signal(signal),
            }),
            Member::Callback(name) => {
                let class = dm.class_name(id).unwrap_or("Instance");
                return Err(Raise(format!(
                    "{name} is a callback member of {class}; you can only set the callback value, get is not available"
                )));
            }
            Member::Missing | Member::Unresolved => match dm.find_first_child(id, key, false)? {
                Some(child) => Push::Instance(child),
                None => return Err(not_member(dm, id, key)),
            },
        };
        Ok(Out::One(push))
    })?
}

/// `instance.Key = value`: properties only, converted to the declared type; read-only ones refuse.
fn new_index_body(call: &Call<'_>) -> Ret {
    let args = &call.args;
    let shared = call.shared()?;
    let Some(id) = args.payload::<Id>(1) else {
        return Err(Raise::from("attempt to index a non-instance"));
    };
    let Some((atom, key)) = args.str_atom(2) else {
        return Err(Raise(format!("invalid member key of type {}", args.type_name(2))));
    };
    call.with_ctx(|ctx| {
        let class = ctx.dm.class_of(id)?;
        match shared.member(class, atom, key) {
            Member::Prop(prop) if prop.read_only => {
                Err(Raise(format!("Unable to assign property {key}. Property is read only")))
            }
            Member::Prop(prop) => {
                let value = read_value(args, 3, &prop.ty, shared)
                    .map_err(|e| Raise(format!("Unable to assign property {key}. {}", e.0)))?;
                ctx.dm.set(id, prop.id, value).map_err(|e| match e {
                    DmError::Destroyed(_) => Raise(format!("cannot set {key}: {e}")),
                    e => Raise(format!("Unable to assign property {key}. {e}")),
                })?;
                Ok(Out::None)
            }
            Member::Callback(name) => match args.kind(3) {
                Kind::Function => {
                    let listener = ctx.set_callback(id, name)?;
                    Ok(Out::KeepRef { idx: 3, listener })
                }
                Kind::Nil | Kind::None => {
                    ctx.clear_callback(id, name)?;
                    Ok(Out::None)
                }
                _ => Err(Raise(format!(
                    "Unable to assign {key}. Expected a function or nil, got {}",
                    args.type_name(3)
                ))),
            },
            Member::Missing | Member::Unresolved => Err(not_member(ctx.dm, id, key)),
            Member::Method(_) | Member::Signal(_) | Member::GetPropertyChangedSignal => {
                Err(Raise(format!("Unable to assign property {key}. It is not a property")))
            }
        }
    })?
}

/// `instance:Method(...)`.
fn namecall_body(call: &Call<'_>) -> Ret {
    let Some((atom, name)) = call.args.namecall() else {
        return Err(Raise::from("no method name"));
    };
    let id = this(&call.args, 1, name)?;
    invoke(call, id, atom, name)
}

/// `instance.Method(instance, ...)`, through a method fetched as a value; its name is the closure's upvalue.
fn method_value_body(call: &Call<'_>) -> Ret {
    let Some((atom, name)) = call.args.upvalue_str_atom(1) else {
        return Err(Raise::from("no method name"));
    };
    let id = this(&call.args, 1, name)?;
    invoke(call, id, atom, name)
}

/// Calls method `name` of `id` with arguments 2.. of the call.
fn invoke(call: &Call<'_>, id: Id, atom: i32, name: &str) -> Ret {
    let args = &call.args;
    let shared = call.shared()?;
    call.with_ctx(|ctx| {
        let class = ctx.dm.class_of(id)?;
        match shared.member(class, atom, name) {
            Member::Method(method) => {
                let values = method_args(args, shared)?;
                if let Some(callback) = method.invokes {
                    // The host runs the script function; this thread waits for its results.
                    let reason = YieldReason::InvokeCallback {
                        target: id,
                        callback,
                        args: values,
                    };
                    let mut pending = shared
                        .pending_yield
                        .try_borrow_mut()
                        .map_err(|_| Raise::from("the engine API was re-entered"))?;
                    *pending = Some(reason);
                    return Ok(Out::Yield);
                }
                let mut method_call = MethodCall {
                    this: id,
                    args: MethodArgs::new(values),
                    dm: &mut *ctx.dm,
                };
                Ok(Out::One(Push::Script((method.func)(&mut method_call)?)))
            }
            Member::GetPropertyChangedSignal => {
                let Some(prop_name) = args.str(2) else {
                    return Err(Raise(format!(
                        "bad argument #1 to '{name}' (string expected, got {})",
                        args.type_name(2)
                    )));
                };
                let prop = ctx.dm.prop_id(id, prop_name)?;
                Ok(Out::One(Push::Signal(SignalHandle {
                    target: id,
                    signal: SignalRef::PropertyChanged(prop),
                })))
            }
            Member::Prop(_) | Member::Signal(_) | Member::Callback(_) => {
                let class = ctx.dm.class_name(id).unwrap_or("Instance");
                Err(Raise(format!("{name} of {class} is not a method")))
            }
            Member::Missing | Member::Unresolved => Err(not_member(ctx.dm, id, name)),
        }
    })?
}

/// Arguments 2.. as engine values.
fn method_args(args: &Args<'_>, shared: &Shared) -> Result<Vec<metallic_core::ScriptValue>, Raise> {
    (2..=args.len()).map(|i| read_script(args, i, shared)).collect()
}

/// `a == b`: the same instance. (Each live instance has one userdata, so this only runs for the rare copies made
/// when the cache could not be used.)
fn eq_body(call: &Call<'_>) -> Ret {
    let same = match (call.args.payload::<Id>(1), call.args.payload::<Id>(2)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    Ok(Out::One(Push::Bool(same)))
}

/// `tostring(instance)`: its `Name`.
fn to_string_body(call: &Call<'_>) -> Ret {
    let Some(id) = call.args.payload::<Id>(1) else {
        return Ok(Out::One(Push::Static("Instance")));
    };
    let name = with_ctx(|ctx| ctx.dm.name(id).map(str::to_owned))?;
    Ok(Out::One(Push::Text(name.unwrap_or_else(|_| "<destroyed>".to_owned()))))
}

/// `Instance.new(className, parent?)`.
fn new_body(call: &Call<'_>) -> Ret {
    let args = &call.args;
    let Some(class_name) = args.str(1) else {
        return Err(Raise(format!(
            "bad argument #1 to 'new' (string expected, got {})",
            args.type_name(1)
        )));
    };
    let parent = match args.payload::<Id>(2) {
        Some(parent) => Some(parent),
        None if args.truthy(2) => {
            return Err(Raise(format!(
                "bad argument #2 to 'new' (Instance expected, got {})",
                args.type_name(2)
            )));
        }
        None => None,
    };
    call.with_ctx(|ctx| {
        let registry = ctx.dm.registry();
        let class = registry
            .class_id(class_name)
            .filter(|&class| registry.class(class).is_some_and(|info| info.creatable()))
            .ok_or_else(|| Raise(format!("Unable to create an Instance of type \"{class_name}\"")))?;
        let id = ctx.dm.create(class)?;
        if let Some(parent) = parent {
            ctx.dm.set_parent(id, Some(parent))?;
        }
        Ok(Out::One(Push::Instance(id)))
    })?
}
