# Issue 315: canonical quantized local movement

Status: implemented; focused tests and source review passed; mandatory workspace/Clippy gates pending.
Branch: studio/315-canonical-local-input, base e20f082.

## Scope and implementation

Independent remaining M2a A3 input foundation, authorized by the owner's continued multiplayer request.
No changes to the current Game input path, ScriptHost, transport/wire or character-state lanes.

- Pure public LocalInput follows the planned axis/button/yaw field shape. It is not a Rust-layout wire packet.
- from_components rounds saturated finite axes to -127..127, wraps/rounds yaw to 65,536 steps and zeroes each
  non-finite local component independently. Held buttons remain explicit.
- is_canonical identifies the reserved -128 axes for future protocol rejection. to_move_input still handles
  every raw struct value safely, including those axes, with finite bounded horizontal movement and analog magnitude.
- CameraController delegates to an extracted move_direction with its original operations and caller magnitude.
  The quantized conversion shortens diagonals separately; the legacy camera/Game behavior is preserved.
- Tests cover signed rounding, saturation, non-finite input, turn seams, flags, analog speed and all 65,536 raw
  axis pairs at five yaw samples (327,680 conversions). Existing camera direction coverage includes a non-unit wish.

FrameInput adapters and Game/Simulation network-role wiring remain later integration. The pure module does not
depend on presentation and is intended to compose with PR #309's CPU-only feature boundary. This does not close
the complete A3, provide player authentication, wire messages, prediction or a live multiplayer session.

## Checks and next

Source written; format/whitespace checks passed and cargo deny check licenses reports licenses ok.
Independent source review by the edit-kernel worker found no actionable issue in quantization, decode bounds, yaw
seams or the exact legacy camera extraction; that review did not run tests. The first focused runtime lib/input
prebuild passed in 1m16s in guarded lane122 after an owner transition. The five input cases passed (327,680
raw-axis/yaw conversions included), followed by the existing camera direction case: one passed, 78 filtered out.
Logs: <TEMP>/metallic-315-focus-build.log, metallic-315-focus-tests.log and metallic-315-camera-tests.log.
No performance pass is claimed.
No dependencies, root manifest or lockfile changes. Required workspace tests and strict all-target Clippy remain pending.
Use an owned guarded build lane with four jobs; coordinate actual Lua suites with other workers. After initial
focus checks, get independent source review, finish mandatory gates, publish one PR, then test private composition
with the existing Simulation foundation so this module stays available without presentation.
