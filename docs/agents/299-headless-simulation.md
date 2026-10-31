# #299 Shared graphics-free Simulation foundation

Status: PR 309 published; original gates/CI passed at 893ff3e. Compatibility normal
merge of pinned d1eecce is source-audited; refreshed validation is pending.

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
- Final default-feature workspace command on 63d526b (base ebf359a) exited zero:
  1,004 passed, zero failed, six existing tests ignored across 105 harness
  and doc-test groups. The seven Simulation cases include actual Game/server
  trajectory parity. Six Parkour cases and both complete keyboard/gamepad route
  cases passed; runtime input allocation (three), input scripts (five), membership
  (six), runner process (two) and binary CLI (one) cases also passed.
  The ignored tests are three existing window tests, two property-getter diagnostic
  probes and one real-budget pattern acceptance probe. No new test is ignored.
- The default-feature workspace prebuild passed in 7m45s on fbaa17d. It reported
  unused TICK/TICK_RATE imports after the extraction; these are removed from the
  production import and TICK remains imported only by the existing unit tests.
- The graph checker now includes normal and build edges. Its actual 83-package
  graph excludes the forbidden presentation crates; both Python policy cases pass.
- Final CPU-only suite on 63d526b/ebf359a passed all 18 cases, with zero failures
  or ignored tests: nine library, one CLI unit, two runner process and six Simulation
  cases. Its prebuild passed in 22.42s. C2's managed prebuild was active during this
  short actual run; no other actual Lua suite or benchmark was active.
- Strict `clippy --workspace --all-targets -j 4 -- -D warnings` passed in 3m40s;
  CPU-only all-target Clippy passed in 1m59s. License, format and whitespace checks
  passed. All Cargo builds/tests used the guarded lane121 with four build jobs.
- The root worker coordinator independently reviewed the source without an actionable
  blocker. Trusted finite/normalized input and distinct Character ownership preconditions
  are explicit in API/public docs; this layer does not validate network commands.
- Independent read-only review of issue 293 at 458e118 found no actionable issue
  in canonical hash encoding, property-index ordering or ancestor/reference filtering.
  No extra build or repository acceptance is claimed from that review.

Saved local logs: `metallic-299-cpu-prebuild.log`, `metallic-299-cpu-tests.log`,
`metallic-299-trajectory-diagnostic.log`, `metallic-299-trajectory-control.log`,
`metallic-299-trajectory-isolated.log`, `metallic-299-cpu-final.log`,
`metallic-299-default-prebuild.log`, `metallic-299-workspace-final.log`,
`metallic-299-clippy-final.log`, `metallic-299-cpu-clippy-final.log`,
`metallic-299-cpu-prebuild-final.log`, `metallic-299-cpu-pinned.log` and
`metallic-299-licenses-final.log`. No unchanged-main red claim is made for this new
API extraction. These logs/results describe the original pinned base, not the
substantive compatibility merge below.

## Original publication

Refreshed once to main ebf359a: the exact diff from fbaa17d contains the M2a plan and
release Python tooling/tests only, with no runtime/script/physics or manifest changes.
PR 309 was published/attached at `893ff3e0a06a80c7eec0b27349e6a3892a678e59`.
Attempt 1 executed no steps because GitHub reported an account payment/spending
condition. Attempt 2 was already running when a rerun was requested; no duplicate
or billing change was submitted. All required checks passed: Windows (6m07s),
Linux CPU tests/Clippy (1m24s), launcher Linux (1m18s), portability (12s), and
instructions (1m35s). These results are historical after the main ABI/features changed.

## Compatibility refresh: normal published-branch merge

Normally merged exact main `d1eeccee2c3d46ca61921baffa562af4bd9472e8` into the
published branch without rebasing, force-pushing or changing published ancestors.
There were no textual conflicts. The pre-note automatic merge tree was
`c77db66e63c5a5a29c26b948205296f8f1ece225`.

The source audit preserves upstream A4 state/replica physics, A5 transport,
boxed-CFrame/pivot behavior, controls/overrides/glyphs and Avatar V4. Core, physics,
net, player, avatar, locomotion, input and avatar-art files are byte-identical to
the pinned main. Game still delegates its sole CPU kernel to Simulation; automatic
merging retained vertical LocomotionInput and the new avatar turning/style behavior.
FrameOutput is retained, and HumanoidState is unconditionally re-exported for CPU
and presentation builds. The manifest retains new serde_json/tempfile dependencies
and presentation gates on graphics dependencies and presentation-only targets.

No stubs, copied unmerged branches, registry changes or feature removals were used.
The full runtime Role/NetDriver/client-Game work remains issue 336, blocked on the
separate foundations; this does not complete A3 or accept M2.

Slot two is explicitly granted for CPU-only and default runtime no-run prebuilds
and graph-policy checks, with four build jobs and guarded lane121 ownership.
Report each compile result; no actual Lua until a separately released window.
Old 893ff3e green results do not validate this merged tree. No own-PR merge or
remote machine changes.
