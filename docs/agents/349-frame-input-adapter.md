# #349 Presentation FrameInput adapter and quantization regressions

## Current status

Implementation source `21aedd518087441f6c5b42e2baf839eddd8b327e`, on pinned main `d34c50f`.
Historical test-only `f605294` produced the expected compile-only missing-method failure: Cargo 101, seven
E0599 errors and no runtime test execution. The branch was restored before green checks. Four real adapter
unit tests and all 11 pure LocalInput tests then passed. No Lua test ran in either focused command.

Workspace prebuild, strict all-target Clippy, licenses and formatting passed. The final serial workspace suite
also passed: 1,540 tests, zero failures and seven existing ignored tests. The actual timing window was released. All output uses the dedicated `lane114/349-input` target; old journal,
benchmark and release artifacts remain untouched, with no compiler artifact copying. No Game/driver/wire/trig
or dependency change. No CPU feature-graph, performance or live-session acceptance claim is made. PR publication and CI are next.

## Goal and ownership

Scoped child of #340, authorized by the multiplayer conductor after read-only research. #315 is closed, so this
adapter/test subset is unblocked. #336 retains Game/Simulation/NetDriver integration; the lead A6 lane retains
the wire -128 decision/codec. Cross-platform sin/cos tables and bit-identical physics are outside this task.

Branch `studio/349-frame-input-adapter`, fresh-main base `d34c50fcfd9b3e768013526ea7b8ea27bb3e25e9`.
Worktree `metallic-studio-worktrees/349-frame-input-adapter`; owned Cargo lane114 will be used only after an
explicit resource grant. The managed worktree tool's nested-source restriction is unchanged; the repository's
prescribed git-worktree fallback was used. PR #338 is left clean and unchanged.

## Source findings

`InputRuntime::binding_value` normalizes digital composites, then applies radial/rescaled deadzones, response
curves and binding scales. `frame_input` reads those evaluated action values and held flags; it does not expose
raw stick/key state. The default thumbstick deadzone is 0.18. The adapter must call `from_components` exactly
once on FrameInput.movement/jump/run and supplied absolute camera yaw, without another deadzone/evaluation.
Orbit and zoom remain presentation-only fields. Game's current raw FrameInput tick path stays unchanged.

The adapter will live in `input/local_input.rs`, leaving the pure root `local_input.rs` free of FrameInput.
PR #309's actual feature is `presentation`, which gates the whole input module, while LocalInput remains
unconditional. Nested adapter unit tests inherit that module boundary rather than referencing a feature absent
from current main. After #309 lands, the CPU-only graph therefore does not need this adapter.

## Test-first definitions (before execution)

Added four adapter unit tests before implementation, plus six pure regressions: every canonical axis code,
every yaw code, exact/adjacent half-step floats, yaw seam, non-finite move_z and 10,000 seeded float-bit patterns.
The adapter tests currently refer to the missing method; their baseline compile failure has NOT been executed.
No Cargo command has run: heavy work is held during the conductor's actual suite and composition compile.

Next: obtain a focused compile slot, observe the missing-method baseline without misclassifying fixture errors,
implement the thin adapter, update API/architecture docs and run focused then required full gates. Do not claim
an executed red, green or CPU feature-graph result before it actually runs. No dependency or visual UI change.


## Implementation checkpoint

`input/local_input.rs` now defines the inherent `LocalInput::from_frame` method and delegates directly to
`from_components(input.movement, input.jump, input.run, camera_yaw)`. No other processing runs. The four nested
unit tests exercise processed keyboard diagonals/held flags, raw-stick evaluation with one existing deadzone,
absolute yaw with ignored orbit/zoom, and non-finite command components. They live under the input module so
PR #309's presentation gate will also gate the adapter tests. Root local_input remains independent of FrameInput.

The six additional pure tests cover every canonical axis, all 65,536 yaw codes, representable half-step neighbors,
turn seams, non-finite second-axis input and bounded seeded float-bit invariants. These are test definitions,
not observed results yet. No dependency, global manifest, Game tick, Simulation, driver or codec file changed.


## Executed focused red and green

With the granted non-Lua compile slot, the owned worktree temporarily checked out test-only `f605294` and ran
`test --locked -p metallic_runtime --lib --no-run -j 4 input::local_input::tests` through cargo_lane. It returned
Cargo 101 with exactly the E0599 missing `LocalInput::from_frame` category (seven call sites); no test harness ran.
The command's finally block restored branch head `21aedd5`. `baseline-adapter-build.log` and
`baseline-result.json` retain source IDs, the raw failure and checksum. This red is a compiler failure, not a
runtime/behavioral failure or a malformed test fixture.

At restored `21aedd5`, the filtered adapter unit command passed all four cases (10.64s build, 0.01s harness):
keyboard composite/flags, one analog deadzone, absolute camera yaw with ignored orbit/zoom, and independently
invalid components. The pure `--test local_input` command then passed all 11 cases (19.44s build, 0.04s harness),
including the six new exhaustive/boundary/property-style cases. Those durations are command observations, not
latency benchmarks. Logs: `adapter-tests.log`, `quantization-tests.log` under
`integration-evidence/349-frame-input-adapter`. No Lua or window test executed during these focused checks.

Root's independent read-only source review at `21aedd5` found no actionable concern in movement/held-flag
semantics, one-pass delegation, no orphan CPU import, unchanged Game behavior and test expectations. The review
ran no tests. The same dedicated target is now prebuilding full workspace binaries; actual Lua execution is
still reserved for its separately granted window. No threshold or safety budget is changed.


## Full required gates

At unchanged implementation `21aedd5`/main `d34c50f`, the guarded full workspace prebuild passed in 6m00s.
Strict `clippy --workspace --all-targets --locked -j 4 -- -D warnings` passed in 2m34s. Licenses (`licenses ok`),
formatting and whitespace checks passed. Logs: `workspace-prebuild.log`, `clippy.log`, `licenses.log` in the same
retained integration evidence directory. The dedicated output target remained `lane114/349-input`; neither the
old performance outputs nor the offered warm lane121 were used or copied.

After the conductor released the earlier actual suites, the guarded full workspace command
`test --workspace --locked -j 4 -- --test-threads=1` exited zero: **1,540 passed, zero failed, seven existing ignored
tests across 119 summaries**. It included both the new adapter and pure cases alongside all default Lua and
Parkour checks. No failure was retried or excluded, no budget changed, and this task added no ignore. The actual
window was released immediately after verified exit/totals. `workspace-tests.log` and `workspace-result.json`
retain the command result and checksum. No CPU-only feature result is inferred before #309's gate lands.

Commit the final validation prose, push only this branch, open/attach one PR that closes #349 (not #340), and
set #349 to review. Check current-head CI and describe later main integration separately if required. Source
review remains the independent `21aedd5` review; the final commit adds documentation only. No own merge,
force push, visual UI, protocol change or live multiplayer/Team Create acceptance.
