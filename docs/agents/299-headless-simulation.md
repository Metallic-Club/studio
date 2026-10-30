# #299 Shared graphics-free Simulation foundation

Status: CPU graph/prebuild passed; first CPU execution 17 passed, one fixture under investigation.

Branch `studio/299-headless-simulation`, initial base
`fbaa17d03d4321b300e6f3ce6d17dc38d5c36223`. User-prioritized narrow A3 foundation;
root worker coordinator approved scope. No M2 acceptance/unblock claim.

## API / ownership

`Simulation` owns CPU World/physics/characters/ScriptHost, fixed-step accumulator,
clock and runtime membership cache. Public `from_world`/`open`/`load` select server-only
scripts. Game delegates through private Combined construction; its existing local
Player/StarterGui startup, camera mapping, avatar/GUI/render and input bridges stay.
There is one tick/event/sync implementation. `advance` retains existing delta clamp,
residual and borrow retry semantics; `tick_once` is explicit fixed-step execution.
It does not consume buffered fractional frame time. `stop` retains World state.

`PlayerInput` is trusted in-process input keyed by a Player InstanceId in this World,
not a backend account or wire ID. Server mode visits each Player once, resolves its
current Character per tick and defaults absent samples to idle. The last repeated
input sample wins. Combined mode keeps stepping only its original local character.
No player admission, Client role, NetDriver, transport or replication is added.

Default `presentation` feature keeps all prior client exports/targets. Optional
render/animation/wgpu/egui dependencies and presentation-only targets are gated.
The finite `metallic-sim` binary in the existing package has no listener or joinability.
It runs trusted projects for 1..=1,000,000 ticks (default 600), stops cleanly and returns
nonzero for argument, loading/start or reported script errors. It is not a wall-clock
watchdog, filesystem sandbox, deployed match server or capacity measurement.

Only the runtime manifest changes; no root manifest/Cargo.lock change observed. Core
classes/hash and detailed journal files remain with the A2/A1 workers.

## Tests and evidence so far

- `python -I tools/test_simulation_graph.py`: two policy cases passed, including
  transitive graphics rejection and incomplete/unrecognized output refusal.
- `python -I tools/check_simulation_graph.py`: actual normal dependency tree reports
  72 packages and none of renderer/wgpu/winit/egui/gilrs. Graph checked with
  `cargo tree --locked -p metallic_runtime --no-default-features --edges normal`.
- Guarded lane121 CPU prebuild passed in 1m19s; logs identify this worktree's sources.
  Prior local package artifacts were cleaned on lane ownership transition.
- First actual serial CPU run: 17 passed, one failed, zero ignored. Physics/cache,
  server-only script/Heartbeat/wait, fixed rates, invalid/clamped/residual/borrowed
  time, real touch/membership controls, stop/restart, runner CLI/process cases passed.
- The new two-player trajectory fixture failed its expected 16-stud assertion. Both
  characters are currently spawned at the same point. No product cause/fix is claimed;
  inspect actual coordinates and matched fixture controls before changing production.
- The default-feature Game/Simulation trajectory comparison has not run yet.

Saved local logs: `metallic-299-cpu-prebuild.log`, `metallic-299-cpu-tests.log`.
No unchanged-main red claim is made for this new API extraction. Full required gates,
no-default strict Clippy, default compatibility and new Linux CPU CI remain pending.

## Next

Core A1 owns a compile-quiet paired benchmark window after the first CPU run. Hold
Rust builds until it releases; source review/docs are independent. Diagnose the
trajectory fixture, run focused CPU/default behavior, then complete the full workspace,
Clippy/licenses and dependency checks on a reviewed pinned main base. Update this note,
open/attach one PR and inspect CI. No own-PR merge or remote machine changes.
