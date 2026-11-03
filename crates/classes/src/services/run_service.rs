//! `RunService`: the frame's events, and where and how the game runs.
//!
//! # Who fires the events
//!
//! The engine loop (`metallic_engine`) fires `RenderStepped`, `PreSimulation`, `PostSimulation` and `Heartbeat`
//! once per frame, each just before the script step of the same name, so their listeners run in that step
//! (`metallic_script`'s crate docs give the frame order). This file only declares them.
//!
//! # How the class learns where it runs
//!
//! `IsStudio`, `IsRunning`, `IsClient` and `IsServer` read four hidden, read-only, unsaved properties (`InStudio`,
//! `Running`, `OnClient`, `OnServer`). Scripts cannot write them (`READ_ONLY` binds scripts, not the engine), Studio's
//! Properties panel does not show them, and scene files do not save them. The engine writes them through
//! [`set_run_flags`] when it creates a game and when it starts or stops it, from its run context: a server is
//! `OnServer`, a client `OnClient`, and a standalone game (one context playing both sides) is both.

use metallic_core::{
    ClassDesc, DataModel, MethodCall, MethodDesc, MethodFn, Param, PropFlags, PropMeta, ScriptError, ScriptValue,
    SignalDesc, Ty, Value,
};

use crate::GameError;

/// The group the hidden flags are listed under in the API dump.
const GROUP: &str = "Run state";

/// The flags are engine state: scripts read them through the methods, Studio does not list them, and they are never
/// saved (a scene loaded later gets fresh ones from the engine that loads it).
const FLAG: PropFlags = PropFlags::READ_ONLY
    .union(PropFlags::HIDDEN)
    .union(PropFlags::NO_SAVE)
    .union(PropFlags::RUNTIME);

static PROPS: &[PropMeta] = &[
    PropMeta::boolean("InStudio", false)
        .flags(FLAG)
        .group(GROUP)
        .order(0)
        .desc("Set by the engine when the game runs inside Studio. Read it with IsStudio()."),
    PropMeta::boolean("Running", false)
        .flags(FLAG)
        .group(GROUP)
        .order(1)
        .desc("Set by the engine while the game runs (paused included). Read it with IsRunning()."),
    PropMeta::boolean("OnClient", false)
        .flags(FLAG)
        .group(GROUP)
        .order(2)
        .desc("Set by the engine when this side is a player's client. Read it with IsClient()."),
    PropMeta::boolean("OnServer", false)
        .flags(FLAG)
        .group(GROUP)
        .order(3)
        .desc("Set by the engine when this side is the server. Read it with IsServer()."),
];

/// Where and how the game runs, as the engine reports it through [`set_run_flags`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunFlags {
    /// The game runs inside Studio.
    pub studio: bool,
    /// The game runs (started and not stopped; a paused game still runs).
    pub running: bool,
    /// This side is a player's client.
    pub client: bool,
    /// This side is the server.
    pub server: bool,
}

/// Writes `flags` into the game's `RunService`, creating the service if the game lacks it. The engine calls it; the
/// methods `IsStudio`, `IsRunning`, `IsClient` and `IsServer` read them back.
pub fn set_run_flags(dm: &mut DataModel, flags: RunFlags) -> Result<(), GameError> {
    let class = dm
        .registry()
        .class_id(CLASS.name)
        .ok_or(GameError::MissingClass(CLASS.name))?;
    let service = match dm.service(class) {
        Some(service) => service,
        None => dm.create_service(class)?,
    };
    for (name, value) in [
        ("InStudio", flags.studio),
        ("Running", flags.running),
        ("OnClient", flags.client),
        ("OnServer", flags.server),
    ] {
        dm.set_by_name(service, name, Value::Bool(value))?;
    }
    Ok(())
}

/// Reads one hidden flag of the `RunService` the method was called on.
fn flag(call: &MethodCall<'_>, name: &str) -> Result<ScriptValue, ScriptError> {
    match call.dm.get_by_name(call.this, name)? {
        Value::Bool(value) => Ok(ScriptValue::Bool(value)),
        _ => Err(ScriptError::runtime(format!("RunService.{name} is not a boolean"))),
    }
}

fn is_studio(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    flag(call, "InStudio")
}

fn is_running(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    flag(call, "Running")
}

fn is_client(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    flag(call, "OnClient")
}

fn is_server(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    flag(call, "OnServer")
}

/// A method with no parameters that returns a boolean.
const fn query(name: &'static str, func: MethodFn, description: &'static str) -> MethodDesc {
    MethodDesc::new(name, func)
        .params(&[])
        .returns(Ty::Bool)
        .describe(description)
}

/// What every frame event passes: the frame's time step.
static DELTA_TIME: &[Param] = &[Param::new("deltaTime", Ty::Number)];

/// The frame's events and the run state.
pub static CLASS: ClassDesc = ClassDesc::new("RunService", Some("Instance"))
    .describe(
        "The engine's frame loop, seen from scripts: events that fire every frame, and whether the game runs in \
         Studio, on a client or on the server. Get it with game:GetService(\"RunService\").",
    )
    .service(5)
    .hidden_in_explorer()
    .props(PROPS)
    .methods(&[
        query(
            "IsStudio",
            is_studio,
            "Returns true when the game runs inside Studio (in play mode or while editing).",
        ),
        query(
            "IsRunning",
            is_running,
            "Returns true while the game runs: after it starts and until it stops. A paused game still runs.",
        ),
        query(
            "IsClient",
            is_client,
            "Returns true on a player's client. A standalone game, with no separate server, is both client and \
             server.",
        ),
        query(
            "IsServer",
            is_server,
            "Returns true on the server. A standalone game, with no separate server, is both client and server.",
        ),
    ])
    .signals(&[
        SignalDesc::new("RenderStepped").params(DELTA_TIME).describe(
            "Fires every frame before the frame is drawn, before physics, with the frame's time step in seconds. \
             Only on clients; a server never fires it. Use it for the camera and other things that must match what \
             is drawn.",
        ),
        SignalDesc::new("PreSimulation").params(DELTA_TIME).describe(
            "Fires every frame just before physics steps, with the frame's time step in seconds. Use it to move \
             things physics should see this frame.",
        ),
        SignalDesc::new("PostSimulation").params(DELTA_TIME).describe(
            "Fires every frame just after physics has stepped, with the frame's time step in seconds. Use it to read \
             where physics put things.",
        ),
        SignalDesc::new("Heartbeat").params(DELTA_TIME).describe(
            "Fires every frame at its end, after physics, with the frame's time step in seconds. Threads waiting in \
             task.wait resume in the same step.",
        ),
    ]);

#[cfg(test)]
mod tests {
    use metallic_core::{ListenerKey, ScriptValue, Value};

    use super::{RunFlags, set_run_flags};
    use crate::testing::{call, game};

    fn run_service(dm: &metallic_core::DataModel) -> metallic_core::Id {
        let class = dm.registry().class_id("RunService").unwrap();
        dm.service(class).unwrap()
    }

    #[test]
    fn a_new_game_has_a_hidden_run_service() {
        let dm = game();
        let service = run_service(&dm);
        let class = dm.class_of(service).unwrap();
        assert!(dm.registry().class(class).unwrap().desc().hidden_in_explorer);
        for name in ["InStudio", "Running", "OnClient", "OnServer"] {
            assert_eq!(dm.get_by_name(service, name), Ok(Value::Bool(false)), "{name}");
        }
    }

    #[test]
    fn the_methods_read_what_the_engine_sets() {
        let mut dm = game();
        let service = run_service(&dm);
        let ask = |dm: &mut metallic_core::DataModel, method: &str| call(dm, service, method, vec![]).unwrap();
        assert_eq!(ask(&mut dm, "IsServer"), ScriptValue::Bool(false));

        let flags = RunFlags {
            studio: true,
            running: true,
            client: false,
            server: true,
        };
        set_run_flags(&mut dm, flags).unwrap();
        assert_eq!(ask(&mut dm, "IsStudio"), ScriptValue::Bool(true));
        assert_eq!(ask(&mut dm, "IsRunning"), ScriptValue::Bool(true));
        assert_eq!(ask(&mut dm, "IsClient"), ScriptValue::Bool(false));
        assert_eq!(ask(&mut dm, "IsServer"), ScriptValue::Bool(true));
    }

    #[test]
    fn scripts_cannot_write_the_flags() {
        let dm = game();
        let service = run_service(&dm);
        let class = dm.class_of(service).unwrap();
        for name in ["InStudio", "Running", "OnClient", "OnServer"] {
            let prop = dm.registry().prop_id(class, name).unwrap();
            let meta = dm.registry().prop(class, prop).unwrap();
            assert!(meta.meta.flags.contains(metallic_core::PropFlags::READ_ONLY), "{name}");
            assert!(meta.meta.flags.contains(metallic_core::PropFlags::NO_SAVE), "{name}");
        }
    }

    #[test]
    fn frame_events_pass_the_time_step() {
        let mut dm = game();
        let service = run_service(&dm);
        for name in ["RenderStepped", "PreSimulation", "PostSimulation", "Heartbeat"] {
            let signal = dm.signal_id(service, name).unwrap();
            dm.connect(service, signal, ListenerKey(1)).unwrap();
            let mut out = Vec::new();
            dm.drain_deliveries(&mut out);
            out.clear();
            dm.fire(service, signal, vec![ScriptValue::Number(0.5)]).unwrap();
            dm.drain_deliveries(&mut out);
            assert_eq!(out.len(), 1, "{name}");
            assert_eq!(out[0].args, [ScriptValue::Number(0.5)], "{name}");
        }
    }
}
