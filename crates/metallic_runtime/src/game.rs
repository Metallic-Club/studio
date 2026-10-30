//! [`Game`]: one running game and its frame loop.
use crate::RuntimeError;
use crate::camera_controller::{CameraController, FOCUS_HEIGHT};
use crate::input::FrameInput;
use crate::project::ProjectConfig;
use metallic_core::{ChangeCursor, Changes, InstanceId, Value, Vec3, World, load_scene};
pub use metallic_physics::RUN_MULTIPLIER;
use metallic_physics::{
    CharacterController, CharacterEvent, HumanoidState, MoveInput, PhysicsWorld, spawn_character,
};
use metallic_profiling::{
    BudgetReport, BudgetTable, Frame, FrameProfiler, FrameReport, Subsystem, SystemClock,
};
use metallic_render::{Camera, Renderer};
use metallic_script::{LOCAL_PLAYER_NAME, OutputSink, ScriptHost};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

/// Simulation ticks per second (physics, characters and scripts).
pub const TICK_RATE: f64 = 60.0;
/// Length of one simulation tick in seconds.
pub const TICK: f64 = 1.0 / TICK_RATE;
/// Longest frame time [`Game::frame`] simulates; a longer stall (a dragged window, a breakpoint)
/// is dropped instead of being caught up in a burst.
pub const MAX_FRAME_DT: f64 = 0.25;
/// Maximum retained CPU scope samples in one opt-in simulation profile; no history is retained.
pub const SIMULATION_PROFILE_CAPACITY: usize = 128;
type SimulationFrame<'p> = Frame<'p, SystemClock, SIMULATION_PROFILE_CAPACITY>;
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

/// The physics-relevant shape of a part; when it changes the physics must re-sync. These are the
/// properties `PhysicsWorld::sync_from` reads besides the pose (it does not read `CanTouch` or
/// `CanQuery` in M1).
#[derive(Debug, Clone, PartialEq)]
struct PartShape {
    size: Value,
    shape: Value,
    material: Value,
    anchored: Value,
    can_collide: Value,
}

impl PartShape {
    fn read(world: &World, id: InstanceId) -> PartShape {
        let get = |p: &str| world.get(id, p).unwrap_or(Value::Nil);
        PartShape {
            size: get("Size"),
            shape: get("Shape"),
            material: get("Material"),
            anchored: get("Anchored"),
            can_collide: get("CanCollide"),
        }
    }
}

fn under_workspace(world: &World, id: InstanceId) -> bool {
    let mut cur = world.parent(id);
    while let Some(p) = cur {
        if world.class_name(p) == Some("Workspace") {
            return true;
        }
        cur = world.parent(p);
    }
    false
}

/// Every BasePart under Workspace with its shape.
fn workspace_parts(world: &World) -> HashMap<InstanceId, PartShape> {
    let mut out = HashMap::new();
    let mut stack: Vec<InstanceId> = world.root("Workspace").into_iter().collect();
    while let Some(id) = stack.pop() {
        if world.is_a(id, "BasePart") {
            out.insert(id, PartShape::read(world, id));
        }
        stack.extend_from_slice(world.children(id));
    }
    out
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
    /// The world's change journal position when the physics was last checked.
    physics_cursor: Option<ChangeCursor>,
    /// Shapes of the parts the physics was last synced with.
    synced_parts: HashMap<InstanceId, PartShape>,
    running: bool,
    cpu_profiling_enabled: bool,
    last_simulation_profile: Option<FrameReport<SIMULATION_PROFILE_CAPACITY>>,
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
            physics_cursor: None,
            synced_parts: HashMap::new(),
            running: false,
            cpu_profiling_enabled: false,
            last_simulation_profile: None,
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

    /// Whether CPU simulation profiling is enabled. New games default to disabled.
    pub fn cpu_profiling_enabled(&self) -> bool {
        self.cpu_profiling_enabled
    }

    /// Opt into bounded CPU simulation profiles, without changing simulation inputs or ticks.
    ///
    /// Changing this flag clears the previous report. Disabled recording performs no clock reads
    /// or collector initialization. This flag does not enable an external profiling backend.
    pub fn set_cpu_profiling_enabled(&mut self, enabled: bool) {
        if self.cpu_profiling_enabled != enabled {
            self.cpu_profiling_enabled = enabled;
            self.last_simulation_profile = None;
        }
    }

    /// Latest completed running [`Game::frame`] CPU interval, if profiling is enabled.
    ///
    /// This interval excludes separate rendering, GUI and app input/window processing. Their
    /// timings remain unavailable; this is not an entire displayed-frame or GPU profile. Reports
    /// are cleared on start, stop, flag changes and before each new frame. Reloading creates a
    /// fresh disabled game. A zero-tick frame can measure camera work without script/physics data.
    pub fn latest_simulation_profile(&self) -> Option<&FrameReport<SIMULATION_PROFILE_CAPACITY>> {
        self.last_simulation_profile.as_ref()
    }

    /// Assess the latest simulation interval with a caller-selected CPU policy.
    ///
    /// Use [`BudgetReport::overall_state`] to retain missing required stages. In particular a
    /// simulation-only interval cannot pass a policy requiring render/GUI timings it did not record.
    pub fn latest_simulation_budget(&self, table: &BudgetTable) -> Option<BudgetReport> {
        self.latest_simulation_profile()
            .map(|report| report.check_budgets(table))
    }

    /// Starts the game: runs the scripts (which creates the local player `Player1`), copies the
    /// contents of `StarterGui` into the player's `PlayerGui` (once; M1 has no ResetOnSpawn),
    /// spawns the character on the first `SpawnLocation` and fires `CharacterAdded`.
    /// Calling it again while running does nothing.
    ///
    /// Errors: [`RuntimeError::Start`] if the script VMs or the character cannot be created.
    pub fn start(&mut self) -> Result<(), RuntimeError> {
        if self.running {
            return Ok(());
        }
        self.last_simulation_profile = None;
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
        copy_starter_gui(&mut world, player);
        let model = spawn_character(&mut world, player)
            .map_err(|e| RuntimeError::Start(format!("cannot spawn the character: {e}")))?;
        Ok((player, model))
    }

    /// Stops the scripts. The world keeps its last state; the game can be dropped afterwards.
    pub fn stop(&mut self) {
        self.last_simulation_profile = None;
        self.scripts.stop();
        self.running = false;
    }

    /// Advances the game by a frame of `dt` seconds and returns the camera to draw it with.
    ///
    /// Camera orbit and zoom apply every frame. The simulation runs in fixed ticks of [`TICK`]
    /// (an accumulator carries the remainder; `dt` is clamped to [`MAX_FRAME_DT`], a negative or
    /// non-finite `dt` counts as 0). Each tick, in order:
    /// 1. character controller (movement from `input`, death, respawn, checkpoints, touches);
    /// 2. physics step (movers, loose parts);
    /// 3. events fed to the scripts: part-part touches both ways, character touches as
    ///    `part.Touched(HumanoidRootPart)`, `Humanoid.Died`, `CharacterAdded` on respawn;
    /// 4. script step (inputs, `RunService.Heartbeat`, resumed waits);
    /// 5. physics re-sync when scripts created, destroyed or reshaped parts (change journal).
    ///
    /// Then the camera follows the character. Draw with [`Game::render`] and
    /// [`crate::gui::paint`]. Opt-in CPU profiles measure only this simulation/camera interval;
    /// rendering, GUI and the host's full-frame boundary are deliberately separate.
    pub fn frame(&mut self, input: &FrameInput, dt: f64) -> FrameOutput {
        self.last_simulation_profile = None;
        let mut profiler = (self.cpu_profiling_enabled && self.running)
            .then(FrameProfiler::<SystemClock, SIMULATION_PROFILE_CAPACITY>::default);
        let profile = profiler.as_mut().map(FrameProfiler::begin_frame);
        let dt = {
            let _scope = profile
                .as_ref()
                .map(|frame| frame.scope(Subsystem::Input, "runtime.input"));
            metallic_profiling::backend_scope!("runtime.input");
            let dt = if dt.is_finite() {
                dt.clamp(0.0, MAX_FRAME_DT)
            } else {
                0.0
            };
            self.camera.apply_input(input);
            dt
        };
        let mut ticks = 0;
        if self.running {
            // Capped so time kept by skipped ticks (see below) never piles up into a burst.
            self.accumulator = (self.accumulator + dt).min(MAX_FRAME_DT + TICK);
            while self.accumulator + 1e-9 >= TICK {
                if !self.tick(input, profile.as_ref()) {
                    // Someone holds a World borrow across frames (a bug in the caller): keep the
                    // time and try again next frame instead of dropping the tick.
                    break;
                }
                self.accumulator -= TICK;
                ticks += 1;
            }
        }
        let output = {
            let _scope = profile
                .as_ref()
                .map(|frame| frame.scope(Subsystem::Input, "runtime.camera"));
            metallic_profiling::backend_scope!("runtime.camera");
            let state = self.character.and_then(|c| self.characters.state(c));
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
        };
        self.last_simulation_profile = profile.map(Frame::finish_interval);
        output
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
        metallic_profiling::backend_scope!("runtime.render");
        if let Ok(world) = self.world.try_borrow() {
            renderer.sync(&world);
        }
        renderer.render(target, camera, size);
    }

    /// Draws the local player's GUI with `painter` (see [`crate::gui`]).
    pub fn paint_gui(&self, painter: &egui::Painter) {
        metallic_profiling::backend_scope!("runtime.gui");
        if let Ok(world) = self.world.try_borrow() {
            crate::gui::paint(&world, painter);
        }
    }

    /// One fixed simulation tick. Returns false, having done nothing, when the world is borrowed.
    fn tick(&mut self, input: &FrameInput, profile: Option<&SimulationFrame<'_>>) -> bool {
        let move_input = {
            let _scope = profile.map(|frame| frame.scope(Subsystem::Input, "runtime.move_input"));
            metallic_profiling::backend_scope!("runtime.move_input");
            MoveInput {
                dir: self.camera.move_direction(input.movement),
                jump: input.jump,
                run: input.run,
            }
        };
        {
            let _scope = profile.map(|frame| frame.scope(Subsystem::Physics, "runtime.physics"));
            metallic_profiling::backend_scope!("runtime.physics");
            let Ok(mut world) = self.world.try_borrow_mut() else {
                return false;
            };
            if let Some(character) = self.character {
                self.characters
                    .step(&mut world, &self.physics, character, move_input, TICK);
            }
            self.physics.step(&mut world, TICK);
        }
        {
            let _scope = profile.map(|frame| frame.scope(Subsystem::Scripts, "runtime.scripts"));
            metallic_profiling::backend_scope!("runtime.scripts");
            self.feed_events(profile);
            self.time += TICK;
            self.scripts.step(self.time, TICK);
        }
        {
            let _scope =
                profile.map(|frame| frame.scope(Subsystem::Physics, "runtime.physics_sync"));
            metallic_profiling::backend_scope!("runtime.physics_sync");
            self.sync_physics_if_changed();
        }
        true
    }

    /// Hands this tick's character and physics events to the script host.
    fn feed_events(&mut self, profile: Option<&SimulationFrame<'_>>) {
        for event in self.characters.drain_events() {
            match event {
                CharacterEvent::Touched { character, part } => {
                    let hit = self
                        .world
                        .try_borrow()
                        .ok()
                        .and_then(|w| w.find_first_child(character, ROOT_PART));
                    if let Some(hit) = hit {
                        self.scripts.touched(part, hit);
                    }
                }
                CharacterEvent::Died { humanoid, .. } => self.scripts.humanoid_died(humanoid),
                CharacterEvent::Respawned {
                    player, character, ..
                } => {
                    if Some(player) == self.player {
                        self.character = Some(character);
                    }
                    {
                        let _scope = profile
                            .map(|frame| frame.scope(Subsystem::Physics, "runtime.respawn_sync"));
                        metallic_profiling::backend_scope!("runtime.respawn_sync");
                        self.sync_physics_now();
                    }
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

    /// Re-syncs the physics when a part under Workspace appeared, disappeared or changed shape
    /// since the last check. Moving parts needs no sync.
    fn sync_physics_if_changed(&mut self) {
        let changed = {
            let Ok(world) = self.world.try_borrow() else {
                return;
            };
            let Some(cursor) = self.physics_cursor else {
                drop(world);
                self.sync_physics_now();
                return;
            };
            let (changes, now) = world.changes_since(cursor);
            self.physics_cursor = Some(now);
            match changes {
                Changes::Overflow => true,
                Changes::Ids(ids) => ids.iter().any(|&id| {
                    let shape = (world.is_a(id, "BasePart") && under_workspace(&world, id))
                        .then(|| PartShape::read(&world, id));
                    shape.as_ref() != self.synced_parts.get(&id)
                }),
            }
        };
        if changed {
            self.sync_physics_now();
        }
    }

    /// Syncs the physics with the world right away.
    fn sync_physics_now(&mut self) {
        let Ok(world) = self.world.try_borrow() else {
            return;
        };
        self.physics.sync_from(&world);
        self.synced_parts = workspace_parts(&world);
        self.physics_cursor = Some(world.change_cursor());
    }

    /// The character's `HumanoidRootPart` position.
    fn root_position(&self) -> Option<Vec3> {
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

/// Copies every child of `StarterGui` into the player's `PlayerGui`.
fn copy_starter_gui(world: &mut World, player: InstanceId) {
    let Some(starter) = world.root("StarterGui") else {
        return;
    };
    let Some(gui) = world
        .children(player)
        .iter()
        .copied()
        .find(|&c| world.is_a(c, "PlayerGui"))
    else {
        return;
    };
    for child in world.children(starter).to_vec() {
        if let Ok(copy) = world.clone_subtree(child)
            && world.set_parent(copy, Some(gui)).is_err()
        {
            world.destroy(copy);
        }
    }
}
