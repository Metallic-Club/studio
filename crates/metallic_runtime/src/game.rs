//! [`Game`]: one running game and its frame loop.
use crate::RuntimeError;
use crate::avatar::{Avatar, default_model};
use crate::body_pose::{CharacterBodies, locomotion_input};
use crate::camera_controller::{CameraController, FOCUS_HEIGHT};
use crate::gui_input::{GuiInput, PointerId, PointerPhase};
use crate::input::{
    ActionEvent, ActionEventKind, ActionId, ActionValue, DeviceKind, FrameInput, InputRuntime,
};
use crate::physics_sync::PhysicsSyncCache;
use crate::project::ProjectConfig;
use metallic_core::{InstanceId, Value, Vec3, World, load_scene};
pub use metallic_physics::RUN_MULTIPLIER;
use metallic_physics::{
    CharacterController, CharacterEvent, HumanoidState, MoveInput, PhysicsWorld, RESPAWN_DELAY,
    spawn_character,
};
use metallic_render::glam::Vec3 as GVec3;
use metallic_render::{Camera, Renderer};
use metallic_script::{
    InputActionEvent, InputDevice, InputEventKind, InputValue, LOCAL_PLAYER_NAME, OutputSink,
    ScriptHost, TouchButtonInfo,
};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Simulation ticks per second (physics, characters and scripts).
pub const TICK_RATE: f64 = 60.0;
/// Length of one simulation tick in seconds.
pub const TICK: f64 = 1.0 / TICK_RATE;
/// Longest frame time [`Game::frame`] simulates; a longer stall (a dragged window, a breakpoint)
/// is dropped instead of being caught up in a burst.
pub const MAX_FRAME_DT: f64 = 0.25;
/// Name of the character part passed as `hit` when a character touches a part.
const ROOT_PART: &str = "HumanoidRootPart";

/// Script output to the terminal: prints to stdout, warnings and errors to stderr.
pub struct StdoutSink;

impl OutputSink for StdoutSink {
    fn print(&self, line: &str) {
        println!("{line}");
    }
    fn warn(&self, line: &str) {
        eprintln!("warning: {line}");
    }
    fn error(&self, line: &str) {
        eprintln!("error: {line}");
    }
}

/// What a frame produced, for the app to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameOutput {
    /// The camera to render this frame with.
    pub camera: Camera,
    /// Simulation ticks run during this frame (0 when the display is faster than [`TICK_RATE`]).
    pub ticks: u32,
    /// The local character's state, if it has one.
    pub state: Option<HumanoidState>,
}

/// One running game: the world, its physics, characters, scripts and camera.
///
/// The world is shared with the script host as `Rc<RefCell<World>>`; the game never holds a
/// borrow of it while scripts run.
pub struct Game {
    name: String,
    world: Rc<RefCell<World>>,
    physics: PhysicsWorld,
    characters: CharacterController,
    scripts: ScriptHost,
    camera: CameraController,
    last_camera: Camera,
    player: Option<InstanceId>,
    character: Option<InstanceId>,
    /// Where the camera looks; kept still while the character is dead.
    focus: Vec3,
    /// Simulated seconds since `start`.
    time: f64,
    accumulator: f64,
    physics_sync: PhysicsSyncCache,
    /// The local character's skinned avatar and the character Model it belongs to.
    avatar: Option<(InstanceId, Avatar)>,
    /// CPU body animation of the stepped characters, which drives their touch volumes.
    bodies: CharacterBodies,
    running: bool,
    /// The copies of `StarterGui` made by the last [`Game::start`] (removed by the next one).
    gui_copies: Vec<InstanceId>,
    /// Seconds the local character has been gone (destroyed or taken out of the Workspace).
    character_gone: f64,
    /// `InputRuntime::device_changes` when the scripts were last told the last-used device.
    input_device_changes: Option<u64>,
    /// Points at the top of the screen that ScreenGuis leave free (see [`Game::set_gui_inset`]).
    gui_inset: f32,
    /// Hover and presses on the GUI (see [`Game::gui_pointer`]).
    gui_input: GuiInput,
}

impl Game {
    /// Loads the project in `project_dir` (its `metallic.toml` and main scene). Script output goes
    /// to the terminal ([`StdoutSink`]).
    ///
    /// Errors: [`RuntimeError::Config`] for a bad `metallic.toml`, [`RuntimeError::MissingScene`]
    /// when the main scene file does not exist, [`RuntimeError::Scene`] when it does not load.
    pub fn load(project_dir: &Path) -> Result<Game, RuntimeError> {
        Game::open(project_dir, None, Rc::new(StdoutSink))
    }

    /// Like [`Game::load`], but plays `scene` (relative to the project folder) instead of the
    /// main scene when given, and sends script output to `output`.
    pub fn open(
        project_dir: &Path,
        scene: Option<&Path>,
        output: Rc<dyn OutputSink>,
    ) -> Result<Game, RuntimeError> {
        let config = ProjectConfig::read(project_dir)?;
        let path = project_dir.join(scene.unwrap_or(&config.main_scene));
        if !path.is_file() {
            return Err(RuntimeError::MissingScene(path));
        }
        let world = load_scene(&path).map_err(|source| RuntimeError::Scene {
            path: path.clone(),
            source,
        })?;
        Ok(Game::from_world(config.name, world, output))
    }

    /// A game over an already loaded world (Studio's Play passes a copy of the edited world).
    pub fn from_world(name: String, world: World, output: Rc<dyn OutputSink>) -> Game {
        let world = Rc::new(RefCell::new(world));
        let scripts = ScriptHost::new(world.clone(), output);
        let camera = CameraController::new();
        Game {
            name,
            world,
            physics: PhysicsWorld::new(),
            characters: CharacterController::new(),
            scripts,
            camera,
            last_camera: Camera::default(),
            player: None,
            character: None,
            focus: Vec3::default(),
            time: 0.0,
            accumulator: 0.0,
            physics_sync: PhysicsSyncCache::default(),
            avatar: None,
            bodies: CharacterBodies::default(),
            running: false,
            gui_copies: Vec::new(),
            character_gone: 0.0,
            input_device_changes: None,
            gui_inset: 0.0,
            gui_input: GuiInput::new(),
        }
    }

    /// The game's name from `metallic.toml`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The world, shared with the scripts. Do not keep a borrow across [`Game::frame`].
    pub fn world(&self) -> Rc<RefCell<World>> {
        self.world.clone()
    }

    /// The local player (`Players.Player1`) once started.
    pub fn player(&self) -> Option<InstanceId> {
        self.player
    }

    /// The local player's current character Model.
    pub fn character(&self) -> Option<InstanceId> {
        self.character
    }

    /// Simulated seconds since [`Game::start`].
    pub fn time(&self) -> f64 {
        self.time
    }

    /// The camera of the last frame.
    pub fn camera(&self) -> Camera {
        self.last_camera
    }

    /// True between [`Game::start`] and [`Game::stop`].
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Starts the game: runs the scripts (which creates the local player `Player1`), copies the
    /// contents of `StarterGui` into the player's `PlayerGui` (once per start; M1 has no
    /// ResetOnSpawn; a restart after [`Game::stop`] replaces the copies of the last start),
    /// spawns the character on the first `SpawnLocation` and fires `CharacterAdded`.
    /// Calling it again while running does nothing.
    ///
    /// Errors: [`RuntimeError::Start`] if the script VMs or the character cannot be created.
    pub fn start(&mut self) -> Result<(), RuntimeError> {
        if self.running {
            return Ok(());
        }
        self.scripts
            .start()
            .map_err(|e| RuntimeError::Start(e.message))?;
        let (player, model) = match self.spawn_local_player() {
            Ok(spawned) => spawned,
            Err(e) => {
                self.scripts.stop();
                return Err(e);
            }
        };
        self.player = Some(player);
        self.character = Some(model);
        self.sync_physics_now();
        self.scripts.character_added(player, model);
        self.focus = self.root_position().unwrap_or_default();
        self.time = 0.0;
        self.accumulator = 0.0;
        self.character_gone = 0.0;
        self.gui_input.reset();
        self.running = true;
        Ok(())
    }

    /// Finds `Player1` (created by the script host), gives it the StarterGui and a character.
    fn spawn_local_player(&mut self) -> Result<(InstanceId, InstanceId), RuntimeError> {
        let mut world = self
            .world
            .try_borrow_mut()
            .map_err(|_| RuntimeError::Start("the world is busy".into()))?;
        let player = world
            .root("Players")
            .and_then(|p| world.find_first_child(p, LOCAL_PLAYER_NAME))
            .ok_or_else(|| RuntimeError::Start("no local player".into()))?;
        for old in self.gui_copies.drain(..) {
            world.destroy(old);
        }
        self.gui_copies = copy_starter_gui(&mut world, player);
        let model = spawn_character(&mut world, player)
            .map_err(|e| RuntimeError::Start(format!("cannot spawn the character: {e}")))?;
        Ok((player, model))
    }

    /// Stops the scripts. The world keeps its last state; the game can be dropped afterwards.
    pub fn stop(&mut self) {
        self.scripts.stop();
        self.gui_input.reset();
        self.running = false;
    }

    /// Evaluates the player's input for one frame and connects it to the scripts; call it before
    /// [`Game::frame`] and pass the result to it. `now` is the input clock (seconds), `dt` the
    /// frame time.
    ///
    /// In order: the runtime follows scene changes (a `BindAction` creates actions) and the
    /// answers of handlers from the last frame are applied; the raw events are evaluated; the
    /// events and values of every `InputAction` instance go to the scripts; the client scripts
    /// handle at most [`metallic_script::MAX_ACTION_EVENTS_PER_STEP`] events now, under the
    /// frame's 0.25 s handler budget; `Sink` answers of `ContextActionService` handlers are
    /// applied at once (spec 3.7), so a sunk key never reaches the contexts below it, the
    /// default character controls included. A `Pass` answer, or a handler that has not answered
    /// (over budget, or yielded), lets the key through. Allocates nothing once the buffers have
    /// grown (the script values handlers receive aside).
    pub fn input_frame(&mut self, rt: &mut InputRuntime, now: f64, dt: f32) -> FrameInput {
        if let Ok(world) = self.world.try_borrow() {
            rt.sync(&world);
        }
        self.apply_script_sinks(rt, now);
        rt.evaluate(now, dt);
        self.feed_input(rt);
        if self.running {
            self.scripts.dispatch_input_now(self.time);
        }
        self.apply_script_sinks(rt, now);
        // Release edges of actions that a sink just blocked.
        self.feed_input(rt);
        rt.frame_input()
    }

    fn apply_script_sinks(&mut self, rt: &mut InputRuntime, now: f64) {
        self.scripts.drain_bound_results(|r| {
            if let Some(action) = rt_action(rt, r.action) {
                rt.set_script_sink(action, r.sink, now);
            }
        });
    }

    /// Sends the runtime's events and the values of its scene actions to the scripts.
    fn feed_input(&mut self, rt: &mut InputRuntime) {
        while let Some(ActionEvent {
            action,
            kind,
            value,
            time,
        }) = rt.pop_event()
        {
            let Some(instance) = rt.instance(action) else {
                continue;
            };
            let kind = match kind {
                ActionEventKind::Pressed => InputEventKind::Pressed,
                ActionEventKind::Released => InputEventKind::Released,
                ActionEventKind::Triggered => InputEventKind::Triggered,
                ActionEventKind::StateChanged => InputEventKind::StateChanged,
            };
            self.scripts.input_action_event(InputActionEvent {
                action: instance,
                kind,
                value: input_value(value),
                time,
            });
        }
        for i in 0..rt.action_count() {
            let id = ActionId(i as u16);
            if let Some(instance) = rt.instance(id) {
                self.scripts.set_action_state(
                    instance,
                    input_value(rt.state(id)),
                    rt.hold_progress(id),
                );
            }
        }
        if self.input_device_changes != Some(rt.device_changes()) {
            self.input_device_changes = Some(rt.device_changes());
            let (device, last) = match rt.last_device() {
                DeviceKind::KeyboardMouse => (InputDevice::KeyboardMouse, "Keyboard"),
                DeviceKind::Gamepad => (InputDevice::Gamepad, "Gamepad1"),
                DeviceKind::Touch => (InputDevice::Touch, "Touch"),
            };
            self.scripts.set_preferred_input(device);
            self.scripts.set_last_input_type(last);
        }
    }

    /// Advances the game by a frame of `dt` seconds and returns the camera to draw it with.
    ///
    /// Camera orbit and zoom apply every frame. The simulation runs in fixed ticks of [`TICK`]
    /// (an accumulator carries the remainder; `dt` is clamped to [`MAX_FRAME_DT`], a negative or
    /// non-finite `dt` counts as 0). Each tick, in order:
    /// 1. character controller (movement from `input`, death, respawn, checkpoints, touches),
    ///    then the character's CPU body animation, whose pose the touch volumes use next tick;
    /// 2. physics step (movers, loose parts); then a character that was destroyed or taken out
    ///    of the Workspace respawns after [`RESPAWN_DELAY`];
    /// 3. events fed to the scripts: part-part touches both ways, character touches as
    ///    `part.Touched(hit)` and `part.TouchEnded(hit)` with the touching body part (`LeftHand`,
    ///    ...) or the `HumanoidRootPart`, `Humanoid.Died`, `CharacterAdded` on respawn;
    /// 4. script step (inputs, `RunService.Heartbeat`, resumed waits);
    /// 5. physics re-sync when scripts created, destroyed or reshaped parts (change journal).
    ///
    /// Then the skinned avatar follows the character (pose, smoothed facing, animation over `dt`)
    /// and the camera follows too. (The touch volumes follow the tick-rate body animation of
    /// step 1, which the avatar's animation matches.) Draw with [`Game::render`] and [`crate::gui::paint`].
    pub fn frame(&mut self, input: &FrameInput, dt: f64) -> FrameOutput {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, MAX_FRAME_DT)
        } else {
            0.0
        };
        self.camera.apply_input(input);
        // The GUI events since the last frame, for the first tick's script step.
        for event in self.gui_input.drain_events() {
            self.scripts.gui_event(event);
        }
        let mut ticks = 0;
        if self.running {
            // Capped so time kept by skipped ticks (see below) never piles up into a burst.
            self.accumulator = (self.accumulator + dt).min(MAX_FRAME_DT + TICK);
            while self.accumulator + 1e-9 >= TICK {
                if !self.tick(input) {
                    // Someone holds a World borrow across frames (a bug in the caller): keep the
                    // time and try again next frame instead of dropping the tick.
                    break;
                }
                self.accumulator -= TICK;
                ticks += 1;
            }
        }
        self.scripts.end_frame();
        let state = self.character.and_then(|c| self.characters.state(c));
        self.update_avatar(dt as f32);
        if state != Some(HumanoidState::Dead)
            && let Some(root) = self.root_position()
        {
            self.focus = root + Vec3::new(0.0, FOCUS_HEIGHT, 0.0);
        }
        let ignore = self.character_parts();
        self.last_camera = self
            .camera
            .camera(self.focus, &self.physics, &ignore, dt as f32);
        FrameOutput {
            camera: self.last_camera,
            ticks,
            state,
        }
    }

    /// Brings `renderer` up to date with the world and draws the frame into `target` (`size` in
    /// pixels) with the camera of the last [`Game::frame`].
    pub fn render(&self, renderer: &mut Renderer, target: &wgpu::TextureView, size: (u32, u32)) {
        self.render_from(renderer, target, size, &self.last_camera);
    }

    /// Like [`Game::render`] but from `camera` (overview shots, Studio's editor camera).
    pub fn render_from(
        &self,
        renderer: &mut Renderer,
        target: &wgpu::TextureView,
        size: (u32, u32),
        camera: &Camera,
    ) {
        self.sync_renderer(renderer);
        renderer.render(target, camera, size);
    }

    /// Brings `renderer` up to date with the world and the avatar without drawing (for
    /// [`Renderer::capture`]).
    pub fn sync_renderer(&self, renderer: &mut Renderer) {
        if let Ok(world) = self.world.try_borrow() {
            renderer.sync(&world);
        }
        match &self.avatar {
            Some((_, avatar)) => renderer.set_skinned(&[avatar.instance()]),
            None => renderer.set_skinned(&[]),
        }
    }

    /// Tells the scripts where the touch overlay's buttons are, for
    /// `ContextActionService:GetButton` (see
    /// [`TouchControls::button_infos`](crate::input::touch::TouchControls::button_infos)).
    pub fn set_touch_buttons(&mut self, buttons: &[TouchButtonInfo]) {
        self.scripts.set_touch_buttons(buttons);
    }

    /// Sets `UserInputService.TouchEnabled`, `KeyboardEnabled` and `GamepadEnabled`.
    pub fn set_input_devices(&mut self, touch: bool, keyboard: bool, gamepad: bool) {
        self.scripts.set_input_devices(touch, keyboard, gamepad);
    }

    /// Sets the GUI inset: the top `points` of the screen belong to the Player's own controls
    /// (the touch overlay's menu button, spec 7.2), so every ScreenGui is laid out in the screen
    /// below it, like Roblox's default `IgnoreGuiInset = false`. Zero when nothing is there.
    pub fn set_gui_inset(&mut self, points: f32) {
        self.gui_inset = if points.is_finite() {
            points.max(0.0)
        } else {
            0.0
        };
    }

    /// The GUI inset in points.
    pub fn gui_inset(&self) -> f32 {
        self.gui_inset
    }

    /// Draws the local player's GUI with `painter` (see [`crate::gui`]), buttons shaded by
    /// hover and press.
    pub fn paint_gui(&self, painter: &egui::Painter) {
        if let Ok(world) = self.world.try_borrow() {
            let input = &self.gui_input;
            crate::gui::paint_shaded(&world, painter, self.gui_inset, &|id| input.shade(id));
        }
    }

    /// Gives a pointer event to the GUI before the game's input: the left mouse button and
    /// position, or a finger, at `at` GUI points (window pixels over the scale factor) on a
    /// `screen` of width x height points. Returns true when the GUI took it; the caller must
    /// then not pass it (nor that press's moves and release) to the [`InputRuntime`], so a
    /// click on a button never also drives the camera or a jump. The events reach the client
    /// scripts in the next [`Game::frame`]'s first tick. Does nothing (false) while stopped.
    pub fn gui_pointer(
        &mut self,
        pointer: PointerId,
        phase: PointerPhase,
        at: [f32; 2],
        screen: [f32; 2],
    ) -> bool {
        if !self.running {
            return false;
        }
        let Ok(world) = self.world.try_borrow() else {
            return false;
        };
        self.gui_input
            .pointer(&world, screen, self.gui_inset, pointer, phase, at)
    }

    /// The mouse left the window: the GUI elements under it get `MouseLeave`.
    pub fn gui_mouse_left(&mut self, at: [f32; 2]) {
        if self.running {
            self.gui_input.mouse_left(at);
        }
    }

    /// One fixed simulation tick. Returns false, having done nothing, when the world is borrowed.
    fn tick(&mut self, input: &FrameInput) -> bool {
        let respawned;
        let move_input = MoveInput {
            dir: self.camera.move_direction(input.movement),
            jump: input.jump,
            run: input.run,
        };
        {
            let Ok(mut world) = self.world.try_borrow_mut() else {
                return false;
            };
            if let Some(character) = self.character {
                self.characters
                    .step(&mut world, &self.physics, character, move_input, TICK);
                // The body animation advances with the simulation, so the touch volumes follow
                // the limbs on any host, headless or not.
                self.bodies
                    .animate(&mut self.characters, &world, character, TICK as f32);
            }
            self.physics.step(&mut world, TICK);
            respawned = respawn_if_gone(
                &mut world,
                &self.characters,
                self.player,
                &mut self.character_gone,
            );
        }
        if let (Some(player), Some(character)) = (self.player, respawned) {
            self.character = Some(character);
            self.sync_physics_now();
            self.scripts.character_added(player, character);
        }
        self.feed_events();
        self.time += TICK;
        self.scripts.step(self.time, TICK);
        self.sync_physics_if_changed();
        true
    }

    /// Hands this tick's character and physics events to the script host.
    fn feed_events(&mut self) {
        for event in self.characters.drain_events() {
            match event {
                // Both sides fire, as for part-part touches: `part.Touched(limb)` and
                // `limb.Touched(part)`.
                CharacterEvent::Touched { part, hit, .. } => {
                    self.scripts.touched(part, hit);
                    self.scripts.touched(hit, part);
                }
                CharacterEvent::TouchEnded { part, hit, .. } => {
                    self.scripts.touch_ended(part, hit);
                    self.scripts.touch_ended(hit, part);
                }
                CharacterEvent::Died { humanoid, .. } => self.scripts.humanoid_died(humanoid),
                CharacterEvent::Respawned {
                    player, character, ..
                } => {
                    // Checked when it fires: never announce a Model that is already gone.
                    let alive = self.world.try_borrow().is_ok_and(|w| w.is_alive(character));
                    if !alive {
                        continue;
                    }
                    if Some(player) == self.player {
                        self.character = Some(character);
                    }
                    self.sync_physics_now();
                    self.scripts.character_added(player, character);
                }
                CharacterEvent::CheckpointReached { .. } => {}
            }
        }
        for (a, b) in self.physics.drain_touched() {
            self.scripts.touched(a, b);
            self.scripts.touched(b, a);
        }
    }

    /// Re-syncs changed part signatures and character membership; pose-only changes need no sync.
    fn sync_physics_if_changed(&mut self) {
        if let Ok(world) = self.world.try_borrow() {
            self.physics_sync
                .synchronize(&mut self.physics, &world, false);
        }
    }

    /// Syncs the physics with the world right away.
    fn sync_physics_now(&mut self) {
        let Ok(world) = self.world.try_borrow() else {
            return;
        };
        self.physics_sync
            .synchronize(&mut self.physics, &world, true);
    }

    /// Moves and animates the local character's avatar; a new character (after a respawn) gets a
    /// fresh one.
    fn update_avatar(&mut self, dt: f32) {
        let (Some(character), Some((root, yaw))) = (self.character, self.root_pose()) else {
            self.avatar = None;
            return;
        };
        let root = GVec3::new(root.x as f32, root.y as f32, root.z as f32);
        let yaw = (yaw as f32).to_radians();
        if self.avatar.as_ref().map(|(c, _)| *c) != Some(character) {
            self.avatar = default_model().map(|m| (character, Avatar::new(m, root, yaw)));
        }
        let Some((_, avatar)) = self.avatar.as_mut() else {
            return;
        };
        let input = locomotion_input(&self.characters, character);
        avatar.update(root, yaw, input, dt);
    }

    /// The character's `HumanoidRootPart` position and facing (`Orientation.Y`, degrees).
    fn root_pose(&self) -> Option<(Vec3, f64)> {
        let world = self.world.try_borrow().ok()?;
        let root = world.find_first_child(self.character?, ROOT_PART)?;
        match (world.get(root, "Position"), world.get(root, "Orientation")) {
            (Ok(Value::Vector3(p)), Ok(Value::Vector3(o))) => Some((p, o.y)),
            _ => None,
        }
    }

    /// The local character's `HumanoidRootPart` position, if it has one.
    pub fn root_position(&self) -> Option<Vec3> {
        let world = self.world.try_borrow().ok()?;
        let root = world.find_first_child(self.character?, ROOT_PART)?;
        match world.get(root, "Position") {
            Ok(Value::Vector3(v)) => Some(v),
            _ => None,
        }
    }

    /// The character's parts (the camera ray ignores them).
    fn character_parts(&self) -> Vec<InstanceId> {
        let (Some(model), Ok(world)) = (self.character, self.world.try_borrow()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut stack = vec![model];
        while let Some(id) = stack.pop() {
            if world.is_a(id, "BasePart") {
                out.push(id);
            }
            stack.extend_from_slice(world.children(id));
        }
        out
    }
}

/// The one respawn timer for a character that is gone without dying (#273): a Model a script
/// destroyed, took out of the Workspace or stripped of its `HumanoidRootPart`. `gone` counts
/// the seconds; after [`RESPAWN_DELAY`] a new character spawns.
///
/// The player's `Character` property is read each tick (never a cached id), so a respawn that
/// already happened (the controller's after a death, or a script's) cancels this timer, and a
/// dead character is left to the controller's own timer: one character ever spawns.
fn respawn_if_gone(
    world: &mut World,
    controller: &CharacterController,
    player: Option<InstanceId>,
    gone: &mut f64,
) -> Option<InstanceId> {
    let player = player.filter(|&p| world.is_alive(p))?;
    let character = match world.get(player, "Character") {
        Ok(Value::Ref(c)) => c,
        _ => None,
    };
    let present = character.is_some_and(|c| {
        world.is_alive(c)
            && in_workspace(world, c)
            && world.find_first_child(c, ROOT_PART).is_some()
    });
    // The controller respawns a dead character it can still step (Humanoid and root present).
    let dying = character.is_some_and(|c| {
        world.is_alive(c)
            && controller.state(c) == Some(HumanoidState::Dead)
            && world.find_first_child(c, "Humanoid").is_some()
            && world.find_first_child(c, ROOT_PART).is_some()
    });
    if present || dying {
        *gone = 0.0;
        return None;
    }
    *gone += TICK;
    if *gone < RESPAWN_DELAY {
        return None;
    }
    *gone = 0.0;
    spawn_character(world, player).ok()
}

/// True when `id` is a descendant of the Workspace.
fn in_workspace(world: &World, id: InstanceId) -> bool {
    let Some(ws) = world.root("Workspace") else {
        return false;
    };
    let mut at = world.parent(id);
    while let Some(p) = at {
        if p == ws {
            return true;
        }
        at = world.parent(p);
    }
    false
}

/// Copies every child of `StarterGui` into the player's `PlayerGui` and returns the copies.
fn copy_starter_gui(world: &mut World, player: InstanceId) -> Vec<InstanceId> {
    let mut copies = Vec::new();
    let Some(starter) = world.root("StarterGui") else {
        return copies;
    };
    let Some(gui) = world
        .children(player)
        .iter()
        .copied()
        .find(|&c| world.is_a(c, "PlayerGui"))
    else {
        return copies;
    };
    for child in world.children(starter).to_vec() {
        if let Ok(copy) = world.clone_subtree(child) {
            if world.set_parent(copy, Some(gui)).is_ok() {
                copies.push(copy);
            } else {
                world.destroy(copy);
            }
        }
    }
    copies
}

/// The runtime action of an `InputAction` instance.
fn rt_action(rt: &InputRuntime, instance: InstanceId) -> Option<ActionId> {
    rt.action_for_instance(instance)
}

fn input_value(v: ActionValue) -> InputValue {
    match v {
        ActionValue::Bool(b) => InputValue::Bool(b),
        ActionValue::Axis1(x) => InputValue::Axis1(x),
        ActionValue::Axis2(x) => InputValue::Axis2(x),
        ActionValue::Axis3(x) => InputValue::Axis3(x),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locomotion::state;

    struct Silent;
    impl OutputSink for Silent {
        fn print(&self, _: &str) {}
        fn warn(&self, _: &str) {}
        fn error(&self, _: &str) {}
    }

    /// A started game on a flat baseplate whose top is at y = 0.
    fn game() -> Game {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let floor = w.create("Part", Some(ws)).unwrap();
        w.set(floor, "Size", Value::Vector3(Vec3::new(400.0, 1.0, 400.0)))
            .unwrap();
        w.set(floor, "Position", Value::Vector3(Vec3::new(0.0, -0.5, 0.0)))
            .unwrap();
        w.set(floor, "Anchored", Value::Bool(true)).unwrap();
        let mut g = Game::from_world("test".into(), w, Rc::new(Silent));
        g.start().unwrap();
        g
    }

    fn run(g: &mut Game, input: FrameInput, seconds: f64) {
        for _ in 0..(seconds / TICK).round() as usize {
            g.frame(&input, TICK);
        }
    }

    fn avatar(g: &Game) -> &Avatar {
        &g.avatar.as_ref().expect("the character has an avatar").1
    }

    #[test]
    fn the_avatar_stands_on_the_floor_and_follows_the_character() {
        let mut g = game();
        run(&mut g, FrameInput::default(), 0.5);
        let a = avatar(&g);
        assert!(a.feet().y.abs() < 0.05, "feet at {}", a.feet().y);
        assert_eq!(a.locomotion().state(), state::GROUND);
        // Walk forward (-Z): the avatar moves with the root.
        let forward = FrameInput {
            movement: [0.0, 1.0],
            ..FrameInput::default()
        };
        run(&mut g, forward, 1.0);
        let root = g.root_position().unwrap();
        let feet = avatar(&g).feet();
        assert!((f64::from(feet.z) - root.z).abs() < 1e-3);
        assert!(root.z < -10.0, "walked {root:?}");
    }

    #[test]
    fn the_avatar_turns_smoothly_toward_the_move_direction() {
        let mut g = game();
        run(&mut g, FrameInput::default(), 0.2);
        assert!(avatar(&g).yaw().abs() < 1e-4);
        // Walk right (+X): the physics faces yaw -90 degrees at once; the avatar turns over a
        // few frames.
        let right = FrameInput {
            movement: [1.0, 0.0],
            ..FrameInput::default()
        };
        g.frame(&right, TICK);
        let turned = |g: &Game| {
            let y = avatar(g).yaw();
            (y - std::f32::consts::TAU).abs().min(y.abs())
        };
        let first = turned(&g);
        assert!(
            first > 0.05 && first < 1.0,
            "first frame turned {first} rad"
        );
        run(&mut g, right, 0.3);
        assert!(
            (turned(&g) - std::f32::consts::FRAC_PI_2).abs() < 0.02,
            "turned {}",
            turned(&g)
        );
    }

    #[test]
    fn jumping_plays_jump_then_lands() {
        let mut g = game();
        run(&mut g, FrameInput::default(), 0.3);
        let jump = FrameInput {
            jump: true,
            ..FrameInput::default()
        };
        g.frame(&jump, TICK);
        assert_eq!(avatar(&g).locomotion().state(), state::JUMP);
        // Apex after 0.25 s, back down after about 0.5 s.
        let mut seen: Vec<String> = Vec::new();
        for _ in 0..60 {
            g.frame(&FrameInput::default(), TICK);
            let s = avatar(&g).locomotion().state().to_owned();
            if seen.last() != Some(&s) {
                seen.push(s);
            }
        }
        assert_eq!(seen, [state::JUMP, state::FALL, state::LAND]);
        run(&mut g, FrameInput::default(), 0.6);
        assert_eq!(avatar(&g).locomotion().state(), state::GROUND);
    }

    /// #273: a Died handler that takes the body out of the Workspace (without destroying it)
    /// gets exactly one new character, and `Player.Character` is that one.
    #[test]
    fn a_body_removed_on_death_respawns_once() {
        let mut g = game();
        run(&mut g, FrameInput::default(), 0.2);
        let player = g.player().unwrap();
        let old = g.character().unwrap();
        {
            let mut w = g.world.borrow_mut();
            let humanoid = w.find_first_child(old, "Humanoid").unwrap();
            w.set(humanoid, "Health", Value::Number(0.0)).unwrap();
        }
        g.frame(&FrameInput::default(), TICK);
        // What the Died handler does.
        g.world.borrow_mut().set_parent(old, None).unwrap();
        run(&mut g, FrameInput::default(), RESPAWN_DELAY + 1.0);
        let w = g.world.borrow();
        let ws = w.root("Workspace").unwrap();
        let models: Vec<_> = w
            .children(ws)
            .iter()
            .copied()
            .filter(|&c| w.is_a(c, "Model"))
            .collect();
        assert_eq!(models.len(), 1, "one character: {models:?}");
        assert_eq!(g.character(), Some(models[0]));
        assert_eq!(
            w.get(player, "Character").unwrap(),
            Value::Ref(Some(models[0]))
        );
    }

    fn screen_guis(g: &Game) -> usize {
        let w = g.world.borrow();
        let Some(gui) = crate::gui::local_player_gui(&w) else {
            return 0;
        };
        w.children(gui)
            .iter()
            .filter(|&&c| w.is_a(c, "ScreenGui"))
            .count()
    }

    /// #273: stop then start again gives one copy of the StarterGui, not two.
    #[test]
    fn a_restart_does_not_duplicate_the_hud() {
        let mut w = World::new_with_services();
        let starter = w.root("StarterGui").unwrap();
        let hud = w.create("ScreenGui", Some(starter)).unwrap();
        w.set(hud, "Name", Value::String("HUD".into())).unwrap();
        let mut g = Game::from_world("test".into(), w, Rc::new(Silent));
        g.start().unwrap();
        assert_eq!(screen_guis(&g), 1);
        g.stop();
        g.start().unwrap();
        assert_eq!(screen_guis(&g), 1, "one HUD after a restart");
        // A GUI a script made at run time is not ours to remove.
        {
            let mut w = g.world.borrow_mut();
            let gui = crate::gui::local_player_gui(&w).unwrap();
            w.create("ScreenGui", Some(gui)).unwrap();
        }
        g.stop();
        g.start().unwrap();
        assert_eq!(screen_guis(&g), 2);
    }

    /// #273: a script that destroys the character gets a new one after the respawn delay.
    #[test]
    fn a_destroyed_character_respawns_after_the_delay() {
        let mut g = game();
        run(&mut g, FrameInput::default(), 0.2);
        let old = g.character().unwrap();
        g.world.borrow_mut().destroy(old);
        run(&mut g, FrameInput::default(), RESPAWN_DELAY - 0.5);
        assert_eq!(g.character(), Some(old), "not before the delay");
        run(&mut g, FrameInput::default(), 1.0);
        let new = g.character().unwrap();
        assert_ne!(new, old);
        assert!(g.world.borrow().is_alive(new));
        assert!(g.root_position().is_some());
        let player = g.player().unwrap();
        assert_eq!(
            g.world.borrow().get(player, "Character").unwrap(),
            Value::Ref(Some(new))
        );
        // Removed from the Workspace without being destroyed counts too.
        g.world.borrow_mut().set_parent(new, None).unwrap();
        run(&mut g, FrameInput::default(), RESPAWN_DELAY + 0.5);
        assert_ne!(g.character(), Some(new));
    }
}
