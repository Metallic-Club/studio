# #349 Presentation FrameInput adapter and quantization regressions

## Current status

Test-only checkpoint `f60529420218a92788eaacd33ddae9d664ca0b14` retains the pre-implementation tests for a
reproducible missing-method baseline. A thin adapter and API/architecture docs are now implemented in the working
tree, but no Rust build or test has run. Cargo remains held. The first focused grant can check the historical
test-only baseline, return to the implementation and validate both adapter and pure quantization cases without
claiming an unexecuted result. No Game behavior, wire rule, trigonometry or dependency changed.

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

## Test-first checkpoint

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
