# CPU simulation and the default Game

`metallic_runtime::Simulation` owns the World, physics, character controller, scripts,
physics-membership cache, CPU skeletal body animation and simulation clock. Public constructors select server-only
scripts: no client VM, synthetic player, PlayerGui or synthetic PlayerAdded. Existing
World instances remain intact. Default `Game` uses the same kernel in Combined mode
and retains its local player, camera-relative controls, avatar, GUI and rendering.

The default `presentation` feature keeps the existing Game/input/camera exports.
Build without default features for the CPU library and finite trusted-project tool:

```sh
cargo run -p metallic_runtime --no-default-features --bin metallic-sim -- path/to/trusted-project --ticks 600
python -I tools/check_simulation_graph.py
```

The tool loads `metallic.toml` and its main scene (or `--scene FILE`), starts server
scripts, runs 1..=1,000,000 explicit ticks (default 600), stops and reports simulated
time. It runs as fast as the workload allows; this is not paced real-time hosting or
a capacity measurement. Invalid arguments return exit 2; loading/start or reported
script errors return exit 1. `--help` lists the small command surface.

This foundation supplies no listener, player admission, network driver, replication,
Client role, Team Create session or fleet registration. It does not declare M2 ready
or accepted. Project paths/code are trusted; this runner is not a filesystem sandbox
or wall-clock watchdog for hostile projects. Later process/transport/isolation work
remains separate.

## Scheduling and lifecycle

- `Simulation::from_world(name, world, output)` constructs a stopped server simulation.
  `open(dir, scene, output)` loads a trusted project; `load(dir)` uses terminal output.
- `start()` starts scripts and synchronizes physics. Repeated start while running is a
  no-op. `stop()` clears script tasks/connections and retains the World/controller
  state; the next successful start resets the simulation clock, counter and fractional
  elapsed-time accumulator. Construct a new Simulation for a fresh match/world.
- `advance(dt, inputs)` returns completed fixed ticks at 60 Hz. It preserves Game's
  existing accumulator: negative/non-finite elapsed time is zero, and a long positive
  delta is capped at 0.25 seconds. Excess stall time is not simulated. A World borrow
  that blocks stepping retains the bounded pending time for retry, with no partial
  character/physics/script tick.
- `tick_once(inputs)` executes one exact fixed tick while running, or returns false
  while stopped/borrowed. It does not consume fractional time already buffered by
  `advance`; callers choosing explicit ticks normally use that method consistently.
- Each tick preserves the existing order: character controller, CPU body animation, physics, character/
  touch events, clock advancement, script Heartbeat/waits, then physics synchronization
  for signature/membership changes. World borrows are released before running scripts.
- Game keeps input-action delivery and Sink/Pass handling before movement, and camera/
  avatar updates after the shared advancement. CPU body animation drives animated limb
  touch volumes for every stepped character, using renderer-free animation/asset crates.
  No drawing or GUI work executes in the server kernel. Combined restart replaces only
  its prior StarterGui copies and retains runtime-created GUI; missing local characters
  use the upstream respawn delay. Server mode applies the same gone-without-dying respawn
  timer to every Player that has had a Character (destroyed, out of the Workspace, no
  root part, or `Character` cleared), in `Players` child order, so it matches Game; a Player
  that never had a Character gets none from here (admission and first spawn are #336).
  The test `game_and_server_simulation_keep_identical_world_hashes` runs Game and a server
  Simulation side by side and compares a hash of the whole World every tick.
- Pointer hit testing, shading, GUI inset and input capture stay in Game. GUI events are
  forwarded to its persistent Combined ScriptHost before simulation advancement; they
  remain queued until a fixed tick executes. Stop/start resets pointer capture.

## Trusted player inputs

`PlayerInput { player, input: MoveInput { dir, jump, run } }` uses a Player InstanceId
from this Simulation's World. It is neither a backend user ID nor a wire identity or
authorization decision. Movement is in world space; Game converts its camera-relative
input before delegating. Missing samples mean idle, repeated samples use the last,
and every valid Player is visited once per server tick. Its current `Character`
reference is read each tick, so respawning does not require changing the Player key.
The caller supplies finite movement components with horizontal length at most one;
this API forwards `MoveInput` to the existing controller and does not validate hostile
commands. The authored World must give each Player a distinct live Character reference.
Sharing one character between Players would step it more than once per tick. The caller
owns these world/input invariants and any future network/authentication boundary; a
future admission layer must establish them before constructing `PlayerInput`.

The CPU graph checker examines the actual normal and build dependency tree, rejects graphics
packages and refuses incomplete/unrecognized output. Linux CI builds/tests the CPU
configuration without GUI system libraries; normal workspace gates retain default
Combined/Parkour/input/avatar coverage. These functional and graph checks do not
establish players per core, frame-time targets or production hosting readiness.
