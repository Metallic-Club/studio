# #299 Shared graphics-free Simulation foundation

Status: CPU graph and all 18 CPU cases passed; default-feature/full gates pending.

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
- The initial two-player fixture mixed overlapping spawns into its input contract.
  Diagnostic positions were moved=(15.6441,2.9862,-1.3818) and
  idle=(-1.2533,3.0,0.0006), so it included collision resolution. Distinct explicit
  respawn points isolated the players; movement then passed, while idle at x=100
  showed about 0.000088 studs of contact drift. The original 1e-8 exact-pose assertion
  was not physically justified. A documented 1e-3 horizontal drift bound checks idle
  behavior without demanding bit-exact floating-point collision correction.
- With only fixture setup/assertion changes, the complete CPU rerun passed all 18
  cases (zero failed/ignored), including the duplicate-input/default-idle/respawn
  contract. No simulation defect or production fix is claimed from those initial
  fixture failures. Original diagnostic output is retained separately.
- The default-feature Game/Simulation trajectory comparison has not run yet.
- The default-feature workspace prebuild passed in 7m45s on fbaa17d. It reported
  unused TICK/TICK_RATE imports after the extraction; these are removed from the
  production import and TICK remains imported only by the existing unit tests.
- The graph checker now includes normal and build edges. Its actual 83-package
  graph excludes the forbidden presentation crates; both Python policy cases pass.
- Independent read-only review of issue 293 at 458e118 found no actionable issue
  in canonical hash encoding, property-index ordering or ancestor/reference filtering.
  No extra build or repository acceptance is claimed from that review.

Saved local logs: `metallic-299-cpu-prebuild.log`, `metallic-299-cpu-tests.log`,
`metallic-299-trajectory-diagnostic.log`, `metallic-299-trajectory-control.log`,
`metallic-299-trajectory-isolated.log`, `metallic-299-cpu-final.log`.
No unchanged-main red claim is made for this new API extraction. Full required gates,
no-default strict Clippy, default compatibility and new Linux CPU CI remain pending.

## Next

Refresh once to main ebf359a: the exact diff from fbaa17d contains the M2a plan and
release Python tooling/tests only, with no runtime/script/physics or manifest changes.
Hold actual timing-sensitive tests until the root worker coordinator releases the window.
Root independently inspected the Simulation/Game delegation and checker without a
confirmed blocker; the default/Parkour gates remain essential. Run focused default behavior and complete the full workspace,
Clippy/licenses and dependency checks on a reviewed pinned main base. Update this note,
open/attach one PR and inspect CI. No own-PR merge or remote machine changes.
