//! [`ScriptHost`]: runs the game's scripts and feeds events (Touched, deaths, new characters,
//! remotes) to them. The default host has server/client VMs over one shared `World`;
//! [`ScriptHost::new_server`] starts only the server side without a synthetic local player.
//!
//! The runtime (task 8) owns the host and calls [`ScriptHost::step`] every frame. Inputs
//! that come from other systems (physics, character controller) are queued with
//! [`ScriptHost::touched`], [`ScriptHost::humanoid_died`] and
//! [`ScriptHost::character_added`] and reach the scripts on the next `step`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use metallic_core::{ChangeCursor, Changes, InstanceId, Value as CoreValue, World};
use mlua::Value;

use crate::api::{
    self, ApiState, BoundResult, EventKind, GuiEvent, HostShared, InputActionEvent, InputDevice,
    InputValue, MAX_GUI_EVENTS_PER_STEP, RemoteTarget, TouchButtonInfo,
};
use crate::vm::{Context, OutputSink, RESUME_BUDGET, ScriptError, ScriptVm};

/// Name of the one local player created by the default combined [`ScriptHost::start`].
pub const LOCAL_PLAYER_NAME: &str = "Player1";

/// One VM together with the API state that drives it.
struct Side {
    vm: ScriptVm,
    state: Rc<ApiState>,
}

/// A script that was started: where it runs.
struct RunningScript {
    context: Context,
}

/// The owner id a script's threads and connections carry (never 0).
fn owner_id(id: InstanceId) -> u64 {
    id.to_bits() + 1
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

/// Runs a world's scripts: server `Script`s and, in the default combined mode, `LocalScript`s.
pub struct ScriptHost {
    world: Rc<RefCell<World>>,
    output: Rc<dyn OutputSink>,
    shared: Rc<HostShared>,
    server: Option<Side>,
    client: Option<Side>,
    /// Fixed by construction: combined hosts also create the client VM and local player.
    run_client: bool,
    inputs: Vec<HostEvent>,
    /// Scripts that were started and not stopped since (each runs once per `start`).
    running: HashMap<InstanceId, RunningScript>,
    /// Where the world's change journal was when scripts were last looked for.
    cursor: Option<ChangeCursor>,
    /// Resume budget of both VMs (see [`ScriptVm::set_resume_budget`]).
    resume_budget: Duration,
    /// `dispatch_input_now` already delivered this frame's action events.
    input_dispatched: bool,
    /// GUI events for the client scripts, delivered by the next step.
    gui_events: Vec<GuiEvent>,
}

impl ScriptHost {
    /// Creates a combined server/client host that is not running yet.
    /// Output of all scripts goes to `output`; [`Self::start`] creates the local player.
    pub fn new(world: Rc<RefCell<World>>, output: Rc<dyn OutputSink>) -> ScriptHost {
        ScriptHost {
            world,
            output,
            shared: Rc::new(HostShared::default()),
            server: None,
            client: None,
            run_client: true,
            inputs: Vec::new(),
            running: HashMap::new(),
            cursor: None,
            resume_budget: RESUME_BUDGET,
            input_dispatched: false,
            gui_events: Vec::new(),
        }
    }

    /// Creates a fixed server-only host that is not running yet.
    ///
    /// [`Self::start`] creates only the server VM and runs eligible `Script`s. It creates no
    /// local player, PlayerGui or synthetic PlayerAdded event; existing world instances remain.
    /// `LocalScript`s do not run, including those created or moved while the host is running.
    /// Events, waits, budgets and stop/restart use the same host lifecycle as [`Self::new`].
    ///
    /// This constructor adds no networking, player admission or dedicated-server process.
    /// Client-targeted remotes have no receiver and are discarded on the next delivery step.
    pub fn new_server(world: Rc<RefCell<World>>, output: Rc<dyn OutputSink>) -> ScriptHost {
        let mut host = Self::new(world, output);
        host.run_client = false;
        host
    }

    /// Sets the resume budget of active VMs, now and after the next `start`
    /// ([`RESUME_BUDGET`] by default).
    pub fn set_resume_budget(&mut self, budget: Duration) {
        self.resume_budget = budget;
        for side in [&self.server, &self.client].into_iter().flatten() {
            side.vm.set_resume_budget(budget);
        }
    }

    /// True between `start` and `stop`.
    pub fn is_running(&self) -> bool {
        self.server.is_some()
    }

    /// In the default combined host, creates local player `Player1` (UserId 0, with a
    /// `PlayerGui`), starts both VMs and runs every enabled script in tree order: `Script`s
    /// under Workspace and ServerScriptService in the server VM, `LocalScript`s under StarterPlayerScripts and
    /// a `PlayerGui` in the client VM. A script that errors is reported to the output sink
    /// and the others still run. `PlayerAdded` fires for `Player1` on the first `step`.
    /// A [`Self::new_server`] host starts only the server side, without a synthetic player or
    /// PlayerAdded, and excludes client scripts before recording them as running.
    ///
    /// Scripts cloned into those places later (for example StarterGui contents copied
    /// into the `PlayerGui` when the character spawns) start on the next `step`.
    /// Calling `start` while running does nothing.
    ///
    /// # Errors
    /// Returns an error if a combined host has no `Players` service or a VM cannot be set up.
    pub fn start(&mut self) -> Result<(), ScriptError> {
        if self.is_running() {
            return Ok(());
        }
        let player = self
            .run_client
            .then(|| self.ensure_local_player())
            .transpose()?;
        self.shared.local_player.set(player);
        self.shared.joined.borrow_mut().clear();
        self.shared.remotes.borrow_mut().clear();
        self.shared.queued.set([(0, 0); 2]);
        self.shared.input.clear();
        self.inputs = player
            .map(|player| HostEvent::PlayerAdded { player })
            .into_iter()
            .collect();
        self.running.clear();
        self.cursor = None;

        self.server = Some(self.make_side(Context::Server)?);
        self.client = self
            .run_client
            .then(|| self.make_side(Context::Client))
            .transpose()?;
        self.sync_scripts();
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
            side.state.begin_step();
        }
        self.sync_scripts();
        for side in self.sides() {
            side.state.run_deferred();
        }
        // Remotes first: one fired by a handler in this step waits for the next one.
        self.deliver_remotes();
        self.deliver_inputs();
        self.deliver_input_actions();
        self.deliver_gui_events();

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
            side.state.prune_hub();
            side.state.end_step();
        }
        for side in self.sides_mut() {
            side.vm.step(now);
        }
    }

    /// Shuts active VMs down, dropping every connection and sleeping thread. `start` may be
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
        self.shared.queued.set([(0, 0); 2]);
        self.shared.joined.borrow_mut().clear();
        self.shared.input.clear();
        self.running.clear();
        self.cursor = None;
        self.gui_events.clear();
    }

    /// Queues an event of an `InputAction` instance for the client scripts: the next `step`
    /// delivers `Pressed`, `Released`, `Triggered` or `StateChanged` and the handlers of
    /// `ContextActionService`, oldest event time first, at most
    /// [`MAX_ACTION_EVENTS_PER_STEP`](crate::MAX_ACTION_EVENTS_PER_STEP) per step.
    /// Server-only hosts have no receiver and discard these events at the next step.
    pub fn input_action_event(&mut self, event: InputActionEvent) {
        self.shared.input.push_event(event);
    }

    /// Sets what `InputAction:GetState()` and `GetHoldProgress()` return from now on (the
    /// values as of the start of the frame). `hold_progress` is 0 to 1.
    pub fn set_action_state(&mut self, action: InstanceId, value: InputValue, hold_progress: f32) {
        let mut states = self.shared.input.states.borrow_mut();
        // The host reports every action every frame, so dropping the map loses nothing.
        if states.len() >= 4096 && !states.contains_key(&action) {
            states.clear();
        }
        states.insert(action, (value, hold_progress));
    }

    /// Sets `UserInputService.TouchEnabled`, `KeyboardEnabled` and `GamepadEnabled`.
    pub fn set_input_devices(&mut self, touch: bool, keyboard: bool, gamepad: bool) {
        self.shared.input.set_devices(touch, keyboard, gamepad);
    }

    /// Sets the touch overlay's buttons that `ContextActionService:GetButton` returns. Call it
    /// when the overlay places its buttons again or shows or hides; unchanged lists cost a
    /// comparison only.
    pub fn set_touch_buttons(&mut self, buttons: &[TouchButtonInfo]) {
        let mut mine = self.shared.input.touch_buttons.borrow_mut();
        if mine.as_slice() != buttons {
            mine.clear();
            mine.extend_from_slice(buttons);
        }
    }

    /// Sets `UserInputService.PreferredInput`.
    pub fn set_preferred_input(&mut self, device: InputDevice) {
        self.shared.input.set_preferred(device);
    }

    /// Sets what `UserInputService:GetLastInputType()` returns, by the name of an
    /// `Enum.UserInputType` item. When it changes, `LastInputTypeChanged` fires on the next
    /// `step`. Returns false for an unknown name.
    pub fn set_last_input_type(&mut self, name: &str) -> bool {
        self.shared.input.set_last_input_type(name)
    }

    /// Takes what the `ContextActionService` handlers answered since the last call.
    pub fn take_bound_results(&mut self) -> Vec<BoundResult> {
        std::mem::take(&mut *self.shared.input.results.borrow_mut())
    }

    /// Like [`ScriptHost::take_bound_results`], but hands each result to `f` and keeps the
    /// buffer, so a frame loop allocates nothing.
    pub fn drain_bound_results(&mut self, mut f: impl FnMut(BoundResult)) {
        for r in self.shared.input.results.borrow_mut().drain(..) {
            f(r);
        }
    }

    /// How many queued action events were dropped because the queue was full.
    pub fn dropped_action_events(&self) -> u64 {
        self.shared.input.dropped.get()
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

    /// How many times signal handler arguments were built so far (both VMs). A test hook
    /// that shows the dispatch cost stays bounded when handlers cannot keep up.
    #[doc(hidden)]
    pub fn dispatch_args_built(&self) -> u64 {
        self.sides().iter().map(|s| s.state.args_built()).sum()
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
        vm.set_resume_budget(self.resume_budget);
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

    fn runnable_context(&self, world: &World, id: InstanceId) -> Option<Context> {
        script_context(world, id).filter(|context| self.run_client || *context == Context::Server)
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

    /// Starts scripts that exist now and are not running, and stops running scripts that
    /// were destroyed, disabled or moved out of a place where scripts run. Looks at the whole
    /// tree the first time and only at changed instances afterwards (the world's journal).
    fn sync_scripts(&mut self) {
        let (starts, stops) = {
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
            let (candidates, stop_candidates): (Vec<InstanceId>, Vec<InstanceId>) = match changed {
                Some(ids) => {
                    let running = ids
                        .iter()
                        .copied()
                        .filter(|id| self.running.contains_key(id))
                        .collect();
                    (ids, running)
                }
                None => {
                    let mut all = Vec::new();
                    for &root in world.roots() {
                        collect_tree(&world, root, &mut all);
                    }
                    (all, self.running.keys().copied().collect())
                }
            };
            let stops: Vec<InstanceId> = stop_candidates
                .into_iter()
                .filter(|&id| self.runnable_context(&world, id).is_none())
                .collect();
            let mut starts = Vec::new();
            for id in candidates {
                if self.running.contains_key(&id) {
                    continue;
                }
                if let Some(context) = self.runnable_context(&world, id) {
                    starts.push((id, context, world.full_name(id), source_of(&world, id)));
                }
            }
            (starts, stops)
        };
        for id in stops {
            let Some(script) = self.running.remove(&id) else {
                continue;
            };
            if let Some(side) = self.side_for(script.context) {
                side.state.stop_owner(owner_id(id));
            }
        }
        for (id, context, chunk_name, source) in starts {
            if self.running.contains_key(&id) {
                continue;
            }
            self.running.insert(id, RunningScript { context });
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
                    side.vm.run_script_owned(
                        &chunk_name,
                        &source,
                        vec![("script", Value::UserData(script))],
                        owner_id(id),
                    )
                });
            if let Err(e) = result {
                self.output.error(&e.message);
            }
        }
    }

    /// Queues an event of a GUI element for the client scripts; the next `step` fires it, in
    /// report order. At most [`MAX_GUI_EVENTS_PER_STEP`] wait at once (more are dropped), and
    /// server-only hosts discard them.
    pub fn gui_event(&mut self, event: GuiEvent) {
        if self.run_client && self.gui_events.len() < MAX_GUI_EVENTS_PER_STEP {
            self.gui_events.push(event);
        }
    }

    fn deliver_gui_events(&mut self) {
        if let Some(client) = &self.client {
            for event in &self.gui_events {
                client.state.deliver_gui_event(event);
            }
        }
        // Keeps the capacity: no allocation per step once grown.
        self.gui_events.clear();
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

    /// Fires `event` of `source` with an optional instance argument in active VMs. Dropped
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

    fn deliver_input_actions(&mut self) {
        if !self.run_client {
            self.shared.input.events.borrow_mut().clear();
        }
        let changes = std::mem::take(&mut *self.shared.input.type_changes.borrow_mut());
        if !changes.is_empty() {
            for side in self.sides() {
                side.state.deliver_input_type_changes(&changes);
            }
        }
        if !self.input_dispatched
            && let Some(client) = &self.client
        {
            client.state.deliver_action_events();
        }
    }

    /// Delivers the queued action events to the client scripts now (at most
    /// [`MAX_ACTION_EVENTS_PER_STEP`](crate::MAX_ACTION_EVENTS_PER_STEP)), so the answers of
    /// `ContextActionService` handlers are known before the character reads its input. The
    /// `step` of the same frame then delivers no more action events and continues the time
    /// budget started here; call [`ScriptHost::end_frame`] when the frame is done.
    pub fn dispatch_input_now(&mut self, now: f64) {
        let Some(client) = &self.client else { return };
        client.state.now.set(now);
        client.state.restart_step();
        client.state.run_deferred();
        client.state.deliver_action_events();
        client.state.hold_step_deadline();
        self.input_dispatched = true;
    }

    /// Ends a frame that used [`ScriptHost::dispatch_input_now`].
    pub fn end_frame(&mut self) {
        self.input_dispatched = false;
    }

    fn deliver_remotes(&mut self) {
        let queued = std::mem::take(&mut *self.shared.remotes.borrow_mut());
        self.shared.queued.set([(0, 0); 2]);
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

#[cfg(test)]
mod role_tests {
    use super::*;

    struct Quiet;
    impl OutputSink for Quiet {
        fn print(&self, _: &str) {}
        fn warn(&self, _: &str) {}
        fn error(&self, _: &str) {}
    }

    #[test]
    fn server_role_has_one_vm_and_never_records_client_scripts_as_running() {
        let mut world = World::new_with_services();
        let starter = world
            .find_first_child(world.root("StarterPlayer").unwrap(), "StarterPlayerScripts")
            .unwrap();
        let local = world.create("LocalScript", Some(starter)).unwrap();
        let replicated = world.root("ReplicatedStorage").unwrap();
        let action = world.create("InputAction", Some(replicated)).unwrap();
        let remote = world.create("RemoteEvent", Some(replicated)).unwrap();
        world
            .set(remote, "Name", CoreValue::String("Notify".into()))
            .unwrap();
        let server = world
            .create("Script", world.root("ServerScriptService"))
            .unwrap();
        world
            .set(
                server,
                "Source",
                CoreValue::String(
                    "game:GetService('ReplicatedStorage').Notify:FireAllClients('ignored')".into(),
                ),
            )
            .unwrap();
        let world = Rc::new(RefCell::new(world));
        let mut host = ScriptHost::new_server(world.clone(), Rc::new(Quiet));
        host.start().unwrap();
        assert!(host.server.is_some() && host.client.is_none());
        assert!(host.shared.local_player.get().is_none());
        assert!(host.inputs.is_empty());
        assert_eq!(host.running.len(), 1);
        assert!(host.running.contains_key(&server));
        assert!(!host.running.contains_key(&local));
        assert_eq!(host.shared.remotes.borrow().len(), 1);
        host.input_action_event(InputActionEvent {
            action,
            kind: crate::InputEventKind::Pressed,
            value: InputValue::Bool(true),
            time: 0.0,
        });
        assert_eq!(host.shared.input.events.borrow().len(), 1);
        host.dispatch_input_now(0.0);
        host.step(0.1, 0.1);
        assert!(host.shared.remotes.borrow().is_empty());
        assert_eq!(host.shared.queued.get(), [(0, 0); 2]);
        assert!(host.shared.input.events.borrow().is_empty());

        let late = world
            .borrow_mut()
            .create("LocalScript", Some(starter))
            .unwrap();
        host.step(0.2, 0.1);
        world
            .borrow_mut()
            .set(late, "Enabled", CoreValue::Bool(false))
            .unwrap();
        host.step(0.3, 0.1);
        world
            .borrow_mut()
            .set(late, "Enabled", CoreValue::Bool(true))
            .unwrap();
        host.step(0.4, 0.1);
        assert_eq!(host.running.len(), 1);
        assert!(!host.running.contains_key(&late));
        assert!(host.server.is_some() && host.client.is_none());

        host.stop();
        assert!(host.server.is_none() && host.client.is_none());
        assert!(host.running.is_empty());
        assert!(host.shared.remotes.borrow().is_empty());
    }
}
