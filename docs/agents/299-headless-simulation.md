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

The compatibility source is pinned at merge commit
`3d67ed509b439acf8499d107e883132c67d24a6c` with parents 893ff3e and d1eecce.
No later main changes are included. Fresh guarded lane121 checks use four build
jobs; actual Lua suites run serially in separately coordinated windows.

- CPU-only prebuild passed in 28.74s; default runtime prebuild passed in 40.61s.
- Full workspace no-run prebuild passed in 5m21s. Strict workspace all-target
  Clippy passed in 1m11s.
- `cargo test --workspace --locked -j 4 -- --test-threads=1` exited zero:
  1,234 passed, zero failed, six existing ignored tests across 112 result groups,
  including doc tests. Both complete Parkour routes, the seven Simulation cases,
  both runner cases, four input allocation cases, five input script cases and six
  physics membership cases passed. The six ignores remain three window-opening
  tests, two manual property-getter probes and one manual real-budget pattern probe.
- After the default workspace run, the CPU configuration was explicitly refreshed
  with a no-run check (6.13s), then all 18 selected library/CLI/runner/Simulation
  cases passed, with zero failures or ignores. The managed actual Lua window was
  exclusive; no concurrent managed benchmark or Lua suite ran.
- Both Python graph-policy cases passed. The actual CPU-only normal/build graph
  contains 86 packages and excludes the renderer, wgpu, winit, egui and gilrs.
- CPU-only strict all-target Clippy passed in 10.19s. Final license, format and
  whitespace checks passed. The root worker coordinator reviewed the automatic
  merge/source preservation without an actionable blocker.

Saved compatibility logs in the local temporary directory:
`metallic-299-compat-cpu-prebuild.log`,
`metallic-299-compat-default-prebuild.log`,
`metallic-299-compat-workspace-prebuild.log`,
`metallic-299-compat-clippy-final.log`,
`metallic-299-compat-workspace-final.log`,
`metallic-299-compat-cpu-refresh.log`,
`metallic-299-compat-cpu-final.log`,
`metallic-299-compat-cpu-clippy-final.log` and
`metallic-299-compat-licenses-final.log`.
The append-only publication changes this evidence note after the tested merge;
production sources remain exactly 3d67ed5. Old 893ff3e results are historical;
the checks above validate the merged source. Exact-head remote CI is checked
after publication. No own-PR merge or remote machine changes were performed.

## Second compatibility refresh: touch controls and rendering

After the append-only publication at eaa4c0b, GitHub reported a new conflict as
main gained touch controls, HDR/post processing, lighting, input actions and A6
wire formats. The root worker coordinator requested a normal merge of exact
`b4909b582241cd1f5c946aa69a7e91021a8271d4`. The 1,234 default/18 CPU results above
remain valid only for the preceding 3d67ed5/d1eecce composition.

The only textual conflict is Game's script import list: retain the extracted
imports and add TouchButtonInfo. Both new touch/device setter APIs forward to
the ScriptHost held by Simulation. The new touch-controls integration target is
gated on presentation, matching the other Game-only integration targets. Upstream
rendering, touch/input, player, scripting, A6 and manifest additions are preserved.
The CPU kernel itself is unchanged. No new capability or acceptance is claimed.

Before gates, the coordinator also requested the exact additive main
`720383017d8efb297e12d236558b64f842567018` union, which contains the merged
LocalInput foundation and Arrange follow-up. The sole conflict in runtime lib.rs
keeps LocalInput and its re-export unconditional, with the presentation attribute
remaining only on locomotion. No later main changes are included.

The tested source is `492d62a26bb56219b0b23f2fe7da21d92e4d3ff3`, tree
`6b4c65359f1e9f0e8177e85256441908db0f8740`, with main pinned at 7203830.
Both compatibility merges preserve the published ancestors; no history was
rebased or force-pushed. The task-range whitespace diff is checked against this
pin. The incoming upstream lighting document's existing trailing blank line is
left unchanged.

Fresh guarded lane121 checks with four build jobs:

- CPU-only no-run prebuild passed in 24.42s; full default workspace no-run
  prebuild passed in 2m08s.
- Strict full-workspace all-target Clippy passed in 56.18s; CPU-only strict
  all-target Clippy passed in 9.44s.
- Full default workspace serial suite exited zero: 1,463 passed, zero failed,
  six existing manual ignores across 119 result groups including doc tests.
  All three complete Parkour routes passed, including the new touch route.
  HDR/post-chain (seven), lighting presets (four), touch integration (three),
  LocalInput (five), Simulation (seven) and runner (two) cases passed.
- After an explicit CPU no-run refresh, its actual serial library/CLI/runner/
  Simulation suite passed all 18 cases, with zero failures or ignores. This
  selection matches the original CPU CI target; LocalInput's separate five
  integration cases also passed in the default workspace run above.

Actual Lua suites used coordinated exclusive windows. The full log preserves the
results; no test failure or performance claim occurred in this refresh. Logs in
the local temporary directory use prefix `metallic-299-720-`: `cpu-prebuild.log`,
`workspace-prebuild.log`, `clippy-final.log`, `cpu-clippy-final.log`,
`workspace-final.log`, `cpu-refresh.log` and `cpu-final.log`.
Both graph policy cases and the actual 86-package CPU normal/build graph passed;
no presentation dependency was found. Final license, format and pinned-range
whitespace checks passed; license output is `licenses-final.log` under the same
prefix. The final evidence-only append commit leaves production sources exactly
492d62a. Exact-head remote CI is inspected after publication; earlier results
remain separately identified above. No remote machine changes or own-PR merge
were performed.

## GUI inset compatibility refresh

The 1,463 default/18 CPU results above describe only 492d62a/7203830. GitHub's
new conflict was reproduced by object-only analysis against exact main
`5c84cf7566901c73bd9de3621ab6ad384add8030`: touch follow-up #339 introduces a
GUI inset before the later README update. This is a substantive upstream change,
not a documentation-only refresh or a stale GitHub status.

The coordinator requested a normal merge of that exact base. Preserve the new
gui_inset field, initializer, setter/getter and all upstream behavior; resolve the
sole Game::paint_gui conflict with Simulation's World borrow and the new third
gui_inset argument. No CPU-kernel change is needed. The renderer/script/physics/
net/player changes remain upstream-identical. No newer base is included.

The final tested source is `632b47ea4b083ce9121b820d075af271ea333a87`, tree
`cb4273f8d442791b7bda111fae08ef646606942a`, pinned at exact main 5c84cf7.
studio_core independently reviewed the compatibility scope without an actionable
concern: Game blob d9bd296, Simulation 1d8c6b7, lib 083049e and touch test dfc5477.
No source changed after that review or during the following gates.

- Full workspace no-run passed in 1m13s; CPU-only no-run passed in 14.03s.
- Strict full-workspace all-target Clippy passed in 26.95s; CPU-only all-target
  Clippy passed in 10.97s.
- `cargo test --workspace --locked -j 4 -- --test-threads=1` exited zero:
  1,539 passed, zero failed, seven upstream ignores across 122 result groups,
  including doc tests. All three complete Parkour routes passed. Compared with
  the prior pin, the additional ignore is an upstream Studio actions documentation
  example. This task introduces no ignored test.
- After an explicit CPU no-run refresh, all 18 selected library/CLI/runner/
  Simulation cases passed with zero failures or ignores. Actual Lua suites used
  coordinated exclusive windows; other managed compile-only work may run in the
  separately assigned build slot. No performance claim follows from these tests.
- Both graph-policy cases passed; the actual CPU-only normal/build graph remains
  86 packages without presentation dependencies. Final license, format and
  task-range whitespace checks against 5c84cf7 passed.

Saved logs use local temporary prefix `metallic-299-5c-`: `workspace-prebuild.log`,
`cpu-prebuild.log`, `clippy-final.log`, `cpu-clippy-final.log`,
`workspace-final.log`, `cpu-refresh.log`, `cpu-final.log` and
`licenses-final.log`. Earlier pins/results above remain historical evidence.
The final evidence-only append commit preserves production source 632b47e.
Exact-head CI is inspected after publication; no own-PR merge or remote-machine
action was performed.
