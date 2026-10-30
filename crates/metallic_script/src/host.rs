//! [`ScriptHost`]: runs the game's scripts in a server VM and a client VM over one shared
//! `World`, and feeds events (Touched, deaths, new characters, remotes) to them.
//!
//! The runtime (task 8) owns the host and calls [`ScriptHost::step`] every frame. Inputs
//! that come from other systems (physics, character controller) are queued with
//! [`ScriptHost::touched`], [`ScriptHost::humanoid_died`] and
//! [`ScriptHost::character_added`] and reach the scripts on the next `step`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use metallic_core::{ChangeCursor, Changes, InstanceId, Value as CoreValue, World};
use mlua::Value;

use crate::api::{self, ApiState, EventKind, HostShared, RemoteTarget};
use crate::vm::{Context, OutputSink, ScriptError, ScriptVm};

/// Name of the one local player created by [`ScriptHost::start`].
pub const LOCAL_PLAYER_NAME: &str = "Player1";

/// One VM together with the API state that drives it.
struct Side {
    vm: ScriptVm,
    state: Rc<ApiState>,
}

/// An input from another system, delivered on the next `step`.
enum HostEvent {
    Touched {
        part: InstanceId,
        hit: InstanceId,
    },
    Died {
        humanoid: InstanceId,
    },
    CharacterAdded {
        player: InstanceId,
        model: InstanceId,
    },
    PlayerAdded {
        player: InstanceId,
    },
}

/// Runs the scripts of a world: `Script`s in the server VM, `LocalScript`s in the client VM.
pub struct ScriptHost {
    world: Rc<RefCell<World>>,
    output: Rc<dyn OutputSink>,
    shared: Rc<HostShared>,
    server: Option<Side>,
    client: Option<Side>,
    inputs: Vec<HostEvent>,
    /// Scripts that already ran (each runs once per `start`).
    started: HashSet<InstanceId>,
    /// Where the world's change journal was when scripts were last looked for.
    cursor: Option<ChangeCursor>,
}

impl ScriptHost {
    /// Creates a host that is not running yet. Output of all scripts goes to `output`.
    pub fn new(world: Rc<RefCell<World>>, output: Rc<dyn OutputSink>) -> ScriptHost {
        ScriptHost {
            world,
            output,
            shared: Rc::new(HostShared::default()),
            server: None,
            client: None,
            inputs: Vec::new(),
            started: HashSet::new(),
            cursor: None,
        }
    }

    /// True between `start` and `stop`.
    pub fn is_running(&self) -> bool {
        self.server.is_some()
    }

    /// Creates the local player `Player1` (UserId 0, with a `PlayerGui`), starts both VMs
    /// and runs every enabled script in tree order: `Script`s under Workspace and
    /// ServerScriptService in the server VM, `LocalScript`s under StarterPlayerScripts and
    /// a `PlayerGui` in the client VM. A script that errors is reported to the output sink
    /// and the others still run. `PlayerAdded` fires for `Player1` on the first `step`.
    ///
    /// Scripts cloned into those places later (for example StarterGui contents copied
    /// into the `PlayerGui` when the character spawns) start on the next `step`.
    /// Calling `start` while running does nothing.
    ///
    /// # Errors
    /// Returns an error if the world has no `Players` service or a VM cannot be set up.
    pub fn start(&mut self) -> Result<(), ScriptError> {
        if self.is_running() {
            return Ok(());
        }
        let player = self.ensure_local_player()?;
        self.shared.local_player.set(Some(player));
        self.shared.joined.borrow_mut().clear();
        self.shared.remotes.borrow_mut().clear();
        self.inputs = vec![HostEvent::PlayerAdded { player }];
        self.started.clear();
        self.cursor = None;

        self.server = Some(self.make_side(Context::Server)?);
        self.client = Some(self.make_side(Context::Client)?);
        self.start_new_scripts();
        Ok(())
    }

    /// Advances the scripts to time `now` (seconds). In this order: starts scripts that
    /// appeared since the last step, delivers the queued remotes and inputs, fires
    /// `RunService.Heartbeat(dt)`, then resumes `WaitForChild` and `task.wait` threads.
    /// Does nothing before `start`.
    pub fn step(&mut self, now: f64, dt: f64) {
        if !self.is_running() {
            return;
        }
        for side in self.sides() {
            side.state.now.set(now);
        }
        self.start_new_scripts();
        // Remotes first: one fired by a handler in this step waits for the next one.
        self.deliver_remotes();
        self.deliver_inputs();

        let run_service = self
            .world
            .try_borrow()
            .ok()
            .and_then(|w| w.root("RunService"));
        if let Some(run_service) = run_service {
            for side in self.sides() {
                side.state.fire((run_service, EventKind::Heartbeat), &|_| {
                    Ok(vec![Value::Number(dt)])
                });
            }
        }
        for side in self.sides() {
            side.state.wake_waiters();
        }
        for side in self.sides_mut() {
            side.vm.step(now);
        }
    }

    /// Shuts both VMs down, dropping every connection and sleeping thread. `start` may be
    /// called again afterwards; scripts then run again from the beginning.
    pub fn stop(&mut self) {
        for side in [self.server.take(), self.client.take()]
            .into_iter()
            .flatten()
        {
            side.vm.shutdown();
        }
        self.inputs.clear();
        self.shared.remotes.borrow_mut().clear();
        self.shared.joined.borrow_mut().clear();
        self.started.clear();
        self.cursor = None;
    }

    /// Queues `part.Touched(hit)` for the next `step`.
    pub fn touched(&mut self, part: InstanceId, hit: InstanceId) {
        self.inputs.push(HostEvent::Touched { part, hit });
    }

    /// Queues `Humanoid.Died` for the next `step`.
    pub fn humanoid_died(&mut self, humanoid: InstanceId) {
        self.inputs.push(HostEvent::Died { humanoid });
    }

    /// Sets `Player.Character` to `model` and queues `Player.CharacterAdded(model)` for
    /// the next `step`.
    pub fn character_added(&mut self, player: InstanceId, model: InstanceId) {
        if let Ok(mut world) = self.world.try_borrow_mut() {
            // Fails only for a destroyed player, whose event is dropped on delivery.
            let _ = world.set(player, "Character", CoreValue::Ref(Some(model)));
        }
        self.inputs
            .push(HostEvent::CharacterAdded { player, model });
    }

    fn sides(&self) -> Vec<&Side> {
        self.server.iter().chain(self.client.iter()).collect()
    }

    fn sides_mut(&mut self) -> Vec<&mut Side> {
        self.server
            .iter_mut()
            .chain(self.client.iter_mut())
            .collect()
    }

    fn make_side(&self, context: Context) -> Result<Side, ScriptError> {
        let vm = ScriptVm::with_output(context, self.output.clone())?;
        let state = api::install(
            &vm,
            self.world.clone(),
            self.shared.clone(),
            self.output.clone(),
        )?;
        Ok(Side { vm, state })
    }

    fn side_for(&mut self, context: Context) -> Option<&mut Side> {
        match context {
            Context::Server => self.server.as_mut(),
            Context::Client => self.client.as_mut(),
        }
    }

    /// Finds or creates `Players.Player1` and its `PlayerGui`.
    fn ensure_local_player(&self) -> Result<InstanceId, ScriptError> {
        let failed = |what: &str| ScriptError {
            message: format!("cannot create the local player: {what}"),
        };
        let mut world = self
            .world
            .try_borrow_mut()
            .map_err(|_| failed("the world is busy"))?;
        let players = world
            .root("Players")
            .ok_or_else(|| failed("the world has no Players service"))?;
        let existing = world.find_first_child(players, LOCAL_PLAYER_NAME);
        let player = match existing {
            Some(p) if world.is_a(p, "Player") => p,
            _ => {
                let p = world
                    .create("Player", Some(players))
                    .map_err(|e| failed(&e.to_string()))?;
                world
                    .set(p, "Name", CoreValue::String(LOCAL_PLAYER_NAME.to_string()))
                    .map_err(|e| failed(&e.to_string()))?;
                p
            }
        };
        let has_gui = world
            .children(player)
            .iter()
            .any(|&c| world.is_a(c, "PlayerGui"));
        if !has_gui {
            world
                .create("PlayerGui", Some(player))
                .map_err(|e| failed(&e.to_string()))?;
        }
        Ok(player)
    }

    /// Runs the scripts that exist now and have not run yet, in tree order. Looks at the
    /// whole tree the first time and only at changed instances afterwards.
    fn start_new_scripts(&mut self) {
        let found = {
            let Ok(world) = self.world.try_borrow() else {
                return;
            };
            let changed = match self.cursor {
                Some(cursor) => match world.changes_since(cursor).0 {
                    Changes::Ids(ids) => Some(ids),
                    Changes::Overflow => None,
                },
                None => None,
            };
            self.cursor = Some(world.change_cursor());
            let candidates = match changed {
                Some(ids) => ids,
                None => {
                    let mut all = Vec::new();
                    for &root in world.roots() {
                        collect_tree(&world, root, &mut all);
                    }
                    all
                }
            };
            let mut found = Vec::new();
            for id in candidates {
                if self.started.contains(&id) {
                    continue;
                }
                if let Some(context) = script_context(&world, id) {
                    found.push((id, context, world.full_name(id), source_of(&world, id)));
                }
            }
            found
        };
        for (id, context, chunk_name, source) in found {
            if !self.started.insert(id) {
                continue;
            }
            let Some(side) = self.side_for(context) else {
                continue;
            };
            let result = side
                .state
                .wrap(side.vm.lua(), id)
                .map_err(|e| ScriptError {
                    message: crate::vm::format_error(&e),
                })
                .and_then(|script| {
                    side.vm.run_script_with(
                        &chunk_name,
                        &source,
                        vec![("script", Value::UserData(script))],
                    )
                });
            if let Err(e) = result {
                self.output.error(&e.message);
            }
        }
    }

    fn deliver_inputs(&mut self) {
        for input in std::mem::take(&mut self.inputs) {
            match input {
                HostEvent::Touched { part, hit } => {
                    self.fire_instance_event(part, EventKind::Touched, Some(hit));
                }
                HostEvent::Died { humanoid } => {
                    self.fire_instance_event(humanoid, EventKind::Died, None);
                }
                HostEvent::CharacterAdded { player, model } => {
                    self.fire_instance_event(player, EventKind::CharacterAdded, Some(model));
                }
                HostEvent::PlayerAdded { player } => {
                    let players = self
                        .world
                        .try_borrow()
                        .ok()
                        .and_then(|w| w.is_alive(player).then(|| w.root("Players")).flatten());
                    if let Some(players) = players {
                        self.shared.joined.borrow_mut().insert(player);
                        self.fire_instance_event(players, EventKind::PlayerAdded, Some(player));
                    }
                }
            }
        }
    }

    /// Fires `event` of `source` with an optional instance argument in both VMs. Dropped
    /// when either instance no longer exists.
    fn fire_instance_event(&self, source: InstanceId, event: EventKind, arg: Option<InstanceId>) {
        let alive = self
            .world
            .try_borrow()
            .is_ok_and(|w| w.is_alive(source) && arg.is_none_or(|a| w.is_alive(a)));
        if !alive {
            return;
        }
        for side in self.sides() {
            let state = &side.state;
            state.fire((source, event), &|lua| match arg {
                Some(id) => Ok(vec![Value::UserData(state.wrap(lua, id)?)]),
                None => Ok(Vec::new()),
            });
        }
    }

    fn deliver_remotes(&mut self) {
        let queued = std::mem::take(&mut *self.shared.remotes.borrow_mut());
        for message in queued {
            let side = match message.target {
                RemoteTarget::Server { .. } => self.server.as_ref(),
                RemoteTarget::Client { player } => {
                    let local = self.shared.local_player.get();
                    if player.is_some() && player != local {
                        continue;
                    }
                    self.client.as_ref()
                }
            };
            if let Some(side) = side {
                side.state.deliver_remote(&message);
            }
        }
    }
}

/// All instances below `id` (and `id`), parents first, in tree order.
fn collect_tree(world: &World, id: InstanceId, out: &mut Vec<InstanceId>) {
    out.push(id);
    for &child in world.children(id) {
        collect_tree(world, child, out);
    }
}

fn source_of(world: &World, id: InstanceId) -> String {
    match world.get(id, "Source") {
        Ok(CoreValue::String(s)) => s,
        _ => String::new(),
    }
}

/// Which VM runs `id`, if it is an enabled script in a place where scripts run.
fn script_context(world: &World, id: InstanceId) -> Option<Context> {
    let class = world.class_name(id)?;
    if !matches!(class, "Script" | "LocalScript") {
        return None;
    }
    if !matches!(world.get(id, "Enabled"), Ok(CoreValue::Bool(true))) {
        return None;
    }
    let mut cur = world.parent(id);
    match class {
        "Script" => {
            while let Some(a) = cur {
                if matches!(
                    world.class_name(a),
                    Some("Workspace" | "ServerScriptService")
                ) {
                    return Some(Context::Server);
                }
                cur = world.parent(a);
            }
            None
        }
        "LocalScript" => {
            while let Some(a) = cur {
                if matches!(
                    world.class_name(a),
                    Some("StarterPlayerScripts" | "PlayerGui")
                ) {
                    return Some(Context::Client);
                }
                cur = world.parent(a);
            }
            None
        }
        _ => None,
    }
}
