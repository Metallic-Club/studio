# 252 Player: automatic touch controls and device emulation

Goal: spec `docs/design/input-actions.md` sections 7 and 10.3 (issue #252): thumbstick, look pad and buttons drawn by the
Player from `ShowOnTouch` actions, the touch input path through `InputRuntime`, and `--emulate` for desktop testing.

## Done
- `metallic_runtime::input::touch` (own module, so #254 can work on the Controls page beside it):
  - `TouchSet`: stick / look pad by `TouchSlot`, role and context priority; Bool buttons sorted by `TouchOrder` (ties:
    context rank, tree order), at most 6, plus a menu button for a Bool action with role `Menu`; the rest in `left_out`.
    `TouchLabel` > `DisplayName` > name; `TouchImage`; `TouchLayout = Custom` (`TouchPosition` = top-left corner, scale of
    the safe area plus dp; `TouchSize` = diameter, at least 48 dp).
  - `InputRuntime::load` adds implicit bindings: `TouchStick` (dead zone 0.15), `TouchLook` (Scale 0.01 rad/dp,
    Vector2Scale (1, -1)), `TouchButton1`..`TouchButton6` and `TouchButton7` (menu). New controls in `Control`;
    `TouchLook` is now a delta (dp moved this frame). `touch_set()`, `touch_generation()`, `set_forced_device()`.
  - `RawEvent.cancel` / `RawEvent::cancel`: release without `Triggered` (finger slid more than 2 radii off a button,
    or the OS cancelled the touch).
  - `layout(&DeviceProfile, &TouchSet) -> TouchLayout` (`rects()` for the spec's `Vec<Rect>`): spec 7.2 numbers; when a
    ring slot breaks a rule (narrow portrait, Large controls) the button takes the first spare slot that keeps every
    rule (more ring-2 angles, then a third ring), else it goes to `unplaced`.
  - `Stick` (floating: base under the thumb, follows past R; fixed: knob stops at R), `TouchControls` (finger capture,
    10 fingers, 8 dp dead ring around buttons, look pad on the right half, pointer touches as `MousePosition` px +
    `MouseButton1` when there is no look action, menu acts on release), `paint` (egui, token colours checked by a test).
- `ScriptedSource`: finger lines `frame touchN Down|Move|Up|Cancel pos x y` (dp), `poll_fingers`, `has_fingers`.
- `metallic_script`: `ContextActionService:GetButton` returns a read-only `TouchButton` (`Name`, `ClassName`, `Text`,
  `Image`, `Visible`, `AbsolutePosition`, `AbsoluteSize`, `Position`, `Size`); host API `ScriptHost::set_touch_buttons`,
  `TouchButtonInfo`; `Enum.KeyCode` gains `TouchButton1`..`7`. `Game::set_touch_buttons`, `Game::set_input_devices`.
- Player: `touch_overlay.rs`; `--emulate phone|tablet|small-phone[-portrait]` (window in the device's shape, letterboxed,
  safe-area guide, left mouse = finger, right click pins/lifts a second finger, last device forced to Touch,
  `TouchEnabled` true), real `WindowEvent::Touch` (px per dp = 0.6 x scale factor, zero insets on desktop), 140 ms fade
  (`--reduce-motion`: instant), `TouchImage` PNGs inside the project, draw-time log at the end of an emulated run.
  Headless `--emulate` routes finger lines of `--input-script` through the same controls.
- Fixed on the way: `Overlay::draw` dropped egui's `TexturesDelta` without clearing it, which panics in debug builds
  (`debug_assert` in epaint 0.37), so a debug windowed Player panicked on its first frame.

## Tests
- `src/input/touch/tests.rs`: mockup positions (phone), tablet sizes, the rules over 3 presets x 2 orientations x 1-6
  buttons x Standard/Large, odd screens, custom placement, stick maths, dead zone through the runtime, routing (press on
  down, cancel by sliding off, two fingers, dead ring, look pad, pointer, menu on release, 10 fingers, relayout), paint.
- `tests/touch_controls.rs`: a finger on a scene `InputAction`'s button fires `Pressed`/`Triggered`/`Released` in a
  LocalScript; `GetButton` of a bound action and its handler `Begin`/`End` from a finger.
- `tests/input_alloc.rs`: the touch path allocates nothing per frame.
- `tests/parkour_route.rs`: `the_course_can_be_finished_with_touch_controls` (stick, Jump, Run by fingers on the phone
  layout; the recorded finger script replays to the same HUD).
- `metallic_script/tests/input.rs`: `get_button_reads_the_overlay_placement`.

## Open questions
- "At most 6 buttons": the spec text and the issue say 6 buttons; the mockup draws the primary plus S1 to S6 (7). Built
  as 6 in all (primary + S1..S5); S6's slot is a spare. Raising `MAX_BUTTONS` to 7 is one constant.
- The mockup's tablet primary sits 28 dp from the corner; the spec text says 24. Built as the spec says.
- GUI hit-testing (a touch that lands on a GUI button) is not built: the Player GUI has no interactive buttons yet.
- Stick mode (Fixed/Floating) and the opacity slider are fields (`DeviceProfile::stick_mode`, `OverlayStyle::opacity`)
  for #254's settings page; no UI here.
- Safe-area insets: winit 0.30 has no safe-area API, so desktop uses zero (emulation uses the presets).

## Commands that matter
```
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=<shared target> CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
cargo test -p metallic_runtime -p metallic_script -p metallic_player -j 2
cargo run -p metallic_player -- templates/parkour --emulate phone
cargo run -p metallic_player -- templates/parkour --emulate phone --input-script <file> --frames 70 --capture 66:<png>
```
Finger script lines: `20 touch0 Down pos 170 290`, `28 touch0 Move pos 215 245`, `60 touch1 Down pos 736 310`.

## Verified
- fmt; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace` (no failures); `cargo deny check licenses` (ok).
- Windowed `--emulate phone` and `--emulate small-phone-portrait` on Parkour, screenshots in `252-shots/` (looked at:
  held stick with violet knob, Run and Jump with labels, safe-area guide; idle hint ring).
- Overlay draw time, debug build, RTX 3060 Ti (no Mid-tier machine here), 600 frames with a held stick:
  mean 0.051 ms, p50 0.035 ms, p99 0.23 ms, max 3.6 ms (the first frame lays out the label fonts).
