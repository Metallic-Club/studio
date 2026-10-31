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
- Stick mode and opacity are saved in the player's overrides (`"touch": {"stick", "opacity"}`); the Touch tab came
  in #333.
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

## Review fixes (#325, round 1)
- A new touch set (SetTitle, SetImage, SetPosition, BindAction, a TouchLabel edit) no longer releases every finger.
  `InputRuntime::load` moves held `TouchButtonN` values to the action's new control by path, and a held button whose
  action lost its button ends as a cancel. `TouchControls::sync` re-maps finger captures the same way. Slide-off is
  measured from where the button was when the finger landed.
- Up/Cancel with a non-finite position (or time) still lifts; a Down for a held id cancels the old touch first;
  pointer fingers share `MouseButton1` by count (up with the last one, cancel on Cancel).
- Tests: title change keeps held fingers (unit and Luau e2e), a held button follows its action to another slot without
  pressing the action that takes its old control, a removed button cancels, stick finger over a button, Cancel on the
  stick, focus loss, non-finite Up, reused id, two pointer fingers.

## Merge with #323 (Controls page)
- Glyphs for `TouchButton1`..`7` (touch art); `glyph.rs`'s `unreachable!` for an empty slot now returns the unbound
  glyph. `Control::is_touch` drives `control_device()` and the glyph folder.
- The built-in `Metallic.System/Menu` has `ShowOnTouch`: phones get the menu button, and tapping it opens the Player
  menu (test). While the menu is open, fingers (and the emulated mouse) work it like a mouse.
- `PlayerOverrides::touch` (`TouchSettings`: stick mode, opacity 0.25 to 1) is read, written and reset with the other
  overrides; `TouchOverlay::update` applies it each frame (test).
- The in-play controls hint of #323 hides while the last device is touch (the overlay labels its own buttons).
- Open: the menu button sits top centre as spec 7.2 says, which covers Parkour's top-centre banner a little
  (`252-shots/phone-held.png`); the banner or the button's place may need a change.
