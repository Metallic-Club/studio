# Input actions: design spec

Date: 2026-10-30. Status: **design** (docs only; nothing here is implemented). Issue #180.

One binding model for keyboard and mouse, gamepad and touch. Games listen to actions, never to raw keys. This spec
builds on:

- `crates/metallic_physics/src/character.rs` (`MoveInput`, #184) and `crates/metallic_runtime/src/input.rs`
  (`Action`, `InputState`, `FrameInput`): what exists today (section 1).
- `docs/luau-api.md` and `docs/templates/README.md` (A5, A20): the script API and the planned `UserInputService`.
- `docs/design/building-tools.md` section 15: the Studio keymap (#129). Studio shortcuts and game input stay apart
  (section 10.4).
- `docs/design/learning-course.md` U5: gamepad and keyboard reach for every game UI.
- `docs/design/visual-language/`: tokens for the touch look. Mockup: `docs/design/input-actions/touch-layout.html`.

Labels: **Assistant's call** marks a decision the owner can overturn (all listed again in section 17). "Start value"
means a number the first implementation tunes with playtests. (Sourced: URL) cites a page read on 2026-10-30.
(Unverified) means I could not confirm it.

## 0. Decisions in short

1. **Roblox names for the data model, Metallic additions marked +.** `InputContext`, `InputAction` and
   `InputBinding` use Roblox's class and property names, so Roblox knowledge carries over. Triggers, dead zones,
   touch placement and rebinding are additions. Assistant's call.
2. **Actions are evaluated once per frame on the client, before the fixed ticks.** The result feeds the existing
   `FrameInput` unchanged, so `Game::frame`, `MoveInput` and the Parkour route bot keep working. Assistant's call.
3. **The touch screen is just another device.** The stick and buttons write virtual controls that bindings read, so
   a game never knows it is on a phone. One layout rule places every control from the safe area; creators move
   controls in Studio's device preview, players move them in Settings.
4. **Players can rebind and change every action, including Menu.** Overrides are a small file of differences, per
   game and per player, kept local first. The creator's bindings are never rewritten.
5. **Input is client-only.** Server scripts cannot read actions. A client script sends what the server needs through
   a `RemoteEvent` and the server validates it.
6. **Our own glyphs, drawn from scratch.** No console logos, colours or word marks (section 9.2).

## 1. What exists today

- `metallic_runtime::input`: `Action` (Forward, Back, Left, Right, Jump, Run) is a fixed enum. `InputState`
  collects keys, mouse drag and wheel, and one gamepad (left stick, right stick, A) and returns one `FrameInput` per
  frame: `movement` (x right, y forward, length at most 1), `jump`, `run`, `orbit` (radians this frame), `zoom`
  (studs this frame).
- Constants: `MOUSE_ORBIT_SPEED` 0.006 rad per px, `STICK_ORBIT_SPEED` 3.0 rad/s, `ZOOM_PER_NOTCH` 2.0 studs,
  `STICK_DEADZONE` 0.18 (radial, rescaled so the output starts from 0).
- `metallic_player::window`: winit keys map to `Action` in `action_for`; `poll_gamepad` reads the first connected
  gilrs pad and discards gilrs events (`while gilrs.next_event().is_some() {}`).
- `Game::tick` builds `MoveInput { dir: camera.move_direction(input.movement), jump, run }` and calls
  `CharacterController::step`. `MoveInput::dir` is world space, lengths above 1 count as 1, `run` multiplies
  `Humanoid.WalkSpeed` by 1.5 (`RUN_MULTIPLIER`), `jump` held jumps again on landing.
- Scripts: `docs/luau-api.md` has no input API. The template README plans `UserInputService` and
  `ContextActionService:BindAction` (A5, A20) and says "There is no client input" yet. Issue #80 (blocked) owns raw
  `UserInputService` events.
- Templates hard-code keys in their docs (section 12). Parkour needs only the default controls.
- Missing today: gamepad buttons other than A, keyboard rebinding, touch, a last-used device, any script access.

## 2. Research

| Source | What it does | What we take |
|---|---|---|
| Unity Input System | An action separates "the purpose of an input from the device controls which perform that input"; action maps group actions; actions are Value, Button or Pass-Through; phases Started, Performed, Canceled. (Sourced: https://docs.unity3d.com/Packages/com.unity.inputsystem@1.11/manual/Actions.html, https://docs.unity3d.com/Packages/com.unity.inputsystem@1.11/manual/RespondingToActions.html) | Value actions report the most actuated control; Bool is OR. Phases become our trigger states. |
| Unity composites and schemes | The 2D Vector composite joins four buttons into a Vector2; modes DigitalNormalized (default), Digital, Analog. Control schemes group bindings per device; overrides are non-destructive (`overridePath`) and saved as JSON apart from the asset. (Sourced: https://docs.unity3d.com/Packages/com.unity.inputsystem@1.11/manual/ActionBindings.html) | Composite = four keys to Axis2, normalised by default. Rebinding is a separate override file. |
| Unity interactions | Press, Hold, Tap, SlowTap, MultiTap with parameters (`duration`, `tapTime`, `tapDelay`, `tapCount`); defaults come from settings. (Sourced: https://docs.unity3d.com/Packages/com.unity.inputsystem@1.11/manual/Interactions.html) | Our trigger set. Numbers are our own start values. |
| Unreal Enhanced Input | Input Actions hold Bool, Axis1D, Axis2D or Axis3D values; Mapping Contexts hold the key mappings and are added or removed at runtime with a priority, highest wins on a conflict; Modifiers (dead zone, negate, swizzle) run before Triggers; Hold fires after a threshold, once or repeatedly; trigger states None, Started, Ongoing, Triggered, Completed, Canceled. (Sourced: https://dev.epicgames.com/documentation/en-us/unreal-engine/enhanced-input-in-unreal-engine) | Context priority; modifiers before triggers; value types. |
| Godot InputMap | Named actions with any number of events; `Input.is_action_pressed` for polling, `is_action_pressed` on events; per-action deadzone, default 0.2 in `add_action`; `action_add_event`, `action_erase_events` for runtime changes. (Sourced: https://docs.godotengine.org/en/stable/classes/class_inputmap.html, https://docs.godotengine.org/en/stable/tutorials/inputs/input_examples.html) | Runtime remap by replacing events on an action. |
| Roblox ContextActionService | `BindAction(name, fn, createTouchButton, ...inputTypes)`; `BindActionAtPriority` (higher first; without priority the most recent first); `SetTitle`, `SetPosition`, `SetImage`, `GetButton` (nothing on non-touch devices); handler returns `Sink` or `Pass`; no return means sink. (Sourced: https://create.roblox.com/docs/reference/engine/classes/ContextActionService) | Compatibility layer, section 5.2. |
| Roblox UserInputService | `InputBegan`, `InputChanged`, `InputEnded` with `gameProcessedEvent`; `GetLastInputType`, `LastInputTypeChanged`, `PreferredInput` (Touch, Gamepad, KeyboardAndMouse); `IsKeyDown`, `GetMouseDelta`, `TouchEnabled`, `KeyboardEnabled`, `GamepadEnabled`. (Sourced: https://create.roblox.com/docs/reference/engine/classes/UserInputService) | Compatibility layer, section 5.3. |
| Roblox Input Action System | `InputContext` has `Enabled`, `Priority`, `Sink`. `InputAction` has `Type`, `Enabled`, `DisplayName`, `PreferredBinding` (read-only), events `Pressed`, `Released`, `StateChanged(value)`, method `GetState()`. `Enum.InputActionType` is Bool, Direction1D, Direction2D, Direction3D, ViewportPosition. `InputBinding` has `KeyCode`, `Up`, `Down`, `Left`, `Right`, `Forward`, `Backward`, `PrimaryModifier`, `SecondaryModifier`, `Scale`, `Vector2Scale`, `Vector3Scale`, `ResponseCurve`, `ClampMagnitudeToOne`, `PressedThreshold`, `ReleasedThreshold`, `UIButton`, `PointerIndex`, `DisplayName`, `DisplayImage`. (Sourced: https://create.roblox.com/docs/reference/engine/classes/InputAction, https://create.roblox.com/docs/reference/engine/classes/InputContext, https://create.roblox.com/docs/reference/engine/classes/InputBinding, https://create.roblox.com/docs/reference/engine/enums/InputActionType) The pages list names and types only. What `Sink` does, the defaults, and what `ResponseCurve` computes are (Unverified); section 3 defines Metallic's own meaning. | Names and shape of the model. |
| Steam Input | Action sets switch a mode (menu, walking, driving); action set layers add to the active set; digital and analog actions are looked up by name; glyphs come as PNG or SVG per action origin and style; the device type is a fixed enum. (Sourced: https://partner.steamgames.com/doc/api/ISteamInput) | Contexts as sets and layers; glyph per origin and style. |
| Xbox Accessibility Guideline 107 | Remap all controls in-game, including Esc on PC, and invert each stick axis; remap actions, not just buttons; toggles and "auto" holds for long holds; avoid holds over 2 to 3 seconds, rapid repeats and simultaneous presses; activate on up, with a way to cancel, unless the down event is essential; sensitivity adjustable by at least 50% of default; mobile: adjustable size, spacing and position of touch targets, presets, minimum 15 mm (phone) and 24 mm (tablet), inactive space around targets, portrait and landscape; the customisation itself must be accessible. (Sourced: https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/107) | Sections 7, 8 and 11. |
| Touch target minimum | 48 x 48 dp with at least 8 dp between targets. (Sourced: https://support.google.com/accessibility/android/answer/7101858) Apple's 44 x 44 pt is widely quoted; the Apple page I fetched returned no text, so (Unverified). | 48 dp hard floor. |
| Android insets | Display cutout insets and system gesture insets (bottom inset covers the home gesture, side insets the back gesture on Android 10+) tell an app where not to put controls. (Sourced: https://developer.android.com/develop/ui/views/layout/edge-to-edge) | Safe-area rule, section 7.4. |
| Virtual sticks | Fixed sticks are a common cause of a bad feel; a stick that spawns where the thumb lands and re-centres reduces "losing the stick"; it needs a restricted zone or any touch spawns one; a dead zone stops drift from a resting thumb and should be rescaled so output does not jump from zero. (Sourced, a developer blog, not a study: https://bugnet.io/blog/how-to-fix-mobile-virtual-joystick-feeling-bad) | Floating stick in a zone, rescaled dead zone. Numbers are start values. |
| gilrs 0.11 | `Button` has South, East, North, West, C, Z, LeftTrigger, RightTrigger, LeftTrigger2, RightTrigger2, Select, Start, Mode, LeftThumb, RightThumb, DPadUp/Down/Left/Right, Unknown; button values are 0.0 to 1.0. (Sourced: https://docs.rs/gilrs/0.11.0/gilrs/ev/enum.Button.html) | Section 4.2 mapping table. |

## 3. Data model

### 3.1 Instances

```
ReplicatedStorage
  Input                       Folder (convention; any location under ReplicatedStorage or StarterPlayer works)
    CharacterControls         InputContext   Priority 0
      Move                    InputAction    Type Direction2D
        (InputBinding children: keyboard, arrows, stick, d-pad)
      Jump, Run, Look, Zoom
    Gameplay                  InputContext   a game's own actions
```

Rules:

- The client collects every enabled `InputContext` that is a descendant of `ReplicatedStorage`, `StarterPlayer` or
  `Players.LocalPlayer`. Input classes are inert on the server (section 5.5).
- An `InputAction` belongs to its parent context; `InputBinding`s are children of the action. Names are unique among
  siblings. The **path** `Context/Action` (for example `CharacterControls/Jump`) is the stable key in the player's
  override file (section 8.4). Renaming an action orphans players' overrides, so Studio warns on rename.
- Instances live in the scene file like any other. Studio's Input Actions panel (section 10) edits them.

### 3.2 InputContext

| Property | Type | Default | Meaning |
|---|---|---|---|
| `Enabled` | bool | true | Disabled contexts are skipped. |
| `Priority` | number | 0 | Higher is asked first (section 3.7). |
| `Sink` | bool | false | When true, a control that one of this context's enabled bindings uses is not offered to lower contexts. |
| `RebindGroup` + | string | "Game" | Actions in contexts of the same group are checked against each other for conflicts (section 8.3). |

### 3.3 InputAction

Roblox properties and events (Sourced above): `Type`, `Enabled`, `DisplayName`, `PreferredBinding` (read-only; the
binding that applies after the player's override), `Pressed`, `Released`, `StateChanged(value)`, `GetState()`.

| Property | Type | Default | Meaning |
|---|---|---|---|
| `Type` | `Enum.InputActionType` | Bool | Value type, below. |
| `Trigger` + | `Enum.InputTrigger` | Press | When `Triggered` fires (section 3.5). Bool only. |
| `HoldTime` + | number (ms) | 500 | Hold trigger. |
| `TapTime` + | number (ms) | 200 | Tap and DoubleTap: longest press that still counts. |
| `DoubleTapWindow` + | number (ms) | 300 | Longest gap between two taps. |
| `RepeatDelay` + | number (ms) | 400 | Repeat: wait before the first repeat. |
| `RepeatRate` + | number (ms) | 100 | Repeat: gap between repeats (10 per second). |
| `Role` + | `Enum.InputRole` | None | Move, Look, Jump, Run, Primary, Secondary, Interact, Menu, Zoom. Presets and ProximityPrompt use it (sections 9.3, 11). |
| `Rebindable` + | bool | true | Shows in the player's Controls page. |
| `Required` + | bool | false | The player cannot leave it without a binding on a device that has any (section 8.3). |
| `CanToggle` + | bool | true | A held Bool action may become a toggle for the player (section 11.1). |
| `ShowOnTouch` + | bool | false | Shows an on-screen control (section 7). |
| `TouchOrder` + | number | 0 | Lower comes first: 0 is the primary button, then ring slots in order. |
| `TouchSlot` + | `Enum.TouchSlot` | Auto | Direction2D only: `Stick` (left thumbstick) or `Look` (look pad). Auto gives the first such action the stick and the second the look pad. |
| `TouchLabel` +, `TouchImage` + | string | "" | Text or image on the button. Empty: `DisplayName` and the action's icon. |
| `TouchLayout` +, `TouchPosition` +, `TouchSize` + | enum, UDim2, number (dp) | Auto | `Custom` places the control by `TouchPosition` (Scale of the safe area, Offset in dp) and `TouchSize`. |

Events and methods added: `Triggered` + (section 3.5), `GetGlyph(device?)` +, `GetGlyphs(device?)` +,
`GlyphChanged` + (section 9), `GetHoldProgress()` + (0 to 1, for a progress ring).

**Value types.** The issue's words map to Roblox's enum:

| Issue word | `Enum.InputActionType` | Luau value | Used for |
|---|---|---|---|
| Bool | `Bool` | boolean | Jump, Fire, Interact |
| Axis1 | `Direction1D` | number, -1 to 1 (or any range after `Scale`) | Zoom, trigger pull, steer |
| Axis2 | `Direction2D` | Vector2 (x right, y up or forward) | Move, Look, Pan |
| Direction | `Direction3D` | Vector3 | Fly, swim |
| (extra) | `ViewportPosition` | Vector2 in pixels | pointer, touch point (build tools, picking) |

`Vector2` is not a Luau datatype yet (`docs/luau-api.md` lists `Vector3`, `Color3`, `UDim2`). Phase 2 adds it with
`Vector2.new`, `X`, `Y`, `Magnitude`, `Unit`, `+ - *` and lets `RemoteEvent` carry it (the allowed list in
`docs/luau-api.md` gains Vector2). The 2D design (A23) needs the same type.

### 3.4 InputBinding

Roblox properties are kept as listed in section 2. Meaning and defaults for Metallic:

| Property | Meaning |
|---|---|
| `KeyCode` | One control. Roblox names: `Space`, `W`, `LeftShift`, `ButtonA`, `ButtonL1`, `Thumbstick1`, `DPadUp`. Metallic adds `MouseButton1`, `MouseButton2`, `MouseButton3`, `MouseDelta`, `MouseWheel`, `MousePosition` + (how Roblox names mouse controls on `InputBinding` is (Unverified)). |
| `Up` `Down` `Left` `Right` | Composite for Direction2D: four keys make a Vector2. Opposite keys cancel (A and D together give 0). Result is normalised so diagonals have length 1 (Unity's DigitalNormalized (Sourced above); also today's behaviour). |
| `Forward` `Backward` (with `Up` `Down` `Left` `Right`) | Composite for Direction3D: x from Left/Right, y from Up/Down, z from Forward/Backward. Exact Roblox semantics (Unverified). |
| `PrimaryModifier`, `SecondaryModifier` | Keys that must be down. A binding with modifiers wins over the same key without them (Shift+W does not also fire a bare W binding on the same action). |
| `Scale`, `Vector2Scale`, `Vector3Scale` | Multiply the value. A negative component inverts that axis. |
| `ResponseCurve` | Metallic's meaning: output = sign(v) * abs(v)^ResponseCurve. Default 1 (linear); 2 gives finer control near the centre. |
| `Deadzone` + | 0 to 1. Radial for two axes. Default 0.18 on `Thumbstick1/2` (today's value), 0.05 on `ButtonL2/R2`, 0 elsewhere. |
| `ScaleByDeltaTime` + | Multiply by the frame time in seconds. For rates (a stick that turns the camera at 3 rad/s). |
| `ClampMagnitudeToOne` | Default true for Direction2D and 3D composites and sticks. |
| `PressedThreshold`, `ReleasedThreshold` | An analog control counts as pressed above 0.5 and released below 0.4 (start values). |
| `UIButton` | A `GuiButton` that drives the binding (a custom on-screen button). The automatic touch controls use the same path (section 7). |
| `PointerIndex` | Which finger (0 is the first). |
| `DisplayName`, `DisplayImage` | Overrides the text and icon in glyphs. |

### 3.5 Processing order and triggers

For each binding, per frame:

1. Read the raw control (0 to 1 for buttons and triggers, -1 to 1 for axes, pixels for mouse and touch deltas).
2. Check modifiers; if one is up the binding reads 0.
3. `Deadzone`: for a vector, `len = |v|`; if `len < dz` the result is 0, else `v / len * min((len - dz) / (1 - dz), 1)`.
   (This is today's `dead_zone` in `input.rs`.)
4. `ResponseCurve`, then `Scale` / `Vector2Scale` / `Vector3Scale`, then `ScaleByDeltaTime`, then
   `ClampMagnitudeToOne`.

Then per action: Bool is the OR of its bindings (an analog binding passes the thresholds). Direction types take the
binding with the largest magnitude (Unity's "most actuated", Sourced above), not the sum, so a stick and a key held
together do not add up to more than 1. Then toggle (section 11.1), then the trigger.

**Triggers** (Bool actions; `Pressed` and `Released` always follow the raw state):

| `Trigger` | `Triggered` fires |
|---|---|
| Press | on the press edge. Equals `Pressed`. |
| Release | on the release edge. |
| Hold | once, when the press has lasted `HoldTime` and is still down. A shorter press never fires. `GetHoldProgress()` runs 0 to 1. |
| Tap | on release, if the press lasted at most `TapTime`. |
| DoubleTap | on the release of the second tap, if both taps were within `TapTime` and the gap within `DoubleTapWindow`. |
| Repeat | on the press edge, then after `RepeatDelay`, then every `RepeatRate` while down. |

For Direction and ViewportPosition actions `Triggered` fires whenever the value changes (same as `StateChanged`);
scripts poll `GetState()` for continuous use. Hold and tap times use event timestamps, not frame counts, so a 30 fps
phone still reads a 200 ms tap. Triggers apply to the combined action state, so a hold works the same on any device.

### 3.6 Default contexts

| Context | Priority | Sink | Actions |
|---|---|---|---|
| `Metallic.System` (built in, not in the scene) | 10000 | true | `Menu` (Escape, `ButtonStart`, touch menu button). Opens the Player menu. |
| `CharacterControls` (in every new project) | 0 | false | `Move`, `Jump`, `Run`, `Look`, `Zoom` (section 6). |

`Menu` is rebindable like everything else but keeps at least one binding per device (section 8.3).

### 3.7 Arbitration: priority and sinking

Contexts are asked from the highest `Priority` down; equal priority: the context enabled most recently first
(ContextActionService also prefers the most recently bound, Sourced above). One pass per raw control change:

1. If a GUI object took the pointer or touch (a button, a focused `TextBox`), the control is "processed by GUI":
   pointer controls are not offered to contexts. While a `TextBox` has focus, keyboard controls are not offered
   either. `UserInputService` events still fire with `gameProcessedEvent = true`.
2. For each enabled context in order: find its enabled bindings that read this control. If there are none, go on.
   Otherwise the context's actions take the control. If `Sink` is true, stop: lower contexts never see it and read
   0 for it.
3. A ContextActionService handler (section 5.2) is a consumer at its own priority. It decides per event: `Sink` stops,
   `Pass` continues.

Sinking is per control, not per context. A vehicle context that binds W and S with `Sink` stops those two keys; A and
D still reach `CharacterControls.Move`. Sinking applies to the value too: a lower action reads 0 for a sunk control.
(What Roblox's `Sink` covers is (Unverified); this is the Metallic rule.)

## 4. Device abstraction

```
metallic_runtime::input
  RawEvent   { device: Device, control: Control, value: f32, pos: Option<[f32; 2]>, time: f64 }
  Device     KeyboardMouse | Gamepad(slot: u8) | Touch
  Control    the Roblox-named KeyCode set plus the Metallic mouse controls and Touch virtual controls
  trait RawSource { fn poll(&mut self, out: &mut Vec<RawEvent>) }
  InputQueue ring buffer, capacity 1024 events per frame
  InputRuntime { contexts, bindings (dense arrays), states, events, last_device }
      fn push(&mut self, e: RawEvent)
      fn evaluate(&mut self, now: f64, dt: f32)        // once per frame
      fn state(&self, action) -> Value ; fn drain_events(&mut self) -> impl Iterator<Item = ActionEvent>
```

The window library stays out of `metallic_runtime`: `metallic_player` has the winit and gilrs sources, tests have a
fake source (section 14). Button edges are never dropped from the queue; mouse moves are merged when it is full.
`time` is monotonic seconds. Pressed edges are latched until `evaluate` runs, so a tap shorter than a frame is not
lost, and a frame that runs no fixed tick (display faster than 60 Hz) does not lose a press.

### 4.1 Keyboard and mouse

- Keys bind to **physical** positions (winit `PhysicalKey`), so W-A-S-D stays a block on AZERTY and Dvorak. Glyph text
  uses the layout's label when the OS gives one, else the US name (Unverified: winit's API for this).
- Mouse: `MouseButton1/2/3`, `MouseWheel` (notches), `MouseDelta` (pixels, only while the cursor is captured or a
  binding with a held modifier needs it), `MousePosition`.
- Cursor capture: the Player locks the cursor while any enabled binding on `MouseDelta` has its modifiers down (today:
  right mouse held), or a script sets `UserInputService.MouseBehavior` to lock. Release on focus loss; a focus loss
  releases every held key (as `InputState::release_all` does today).

### 4.2 Gamepad (gilrs)

Buttons are named by position, so one binding works on any pad. The mapping:

| gilrs `Button` | Metallic `KeyCode` | Value |
|---|---|---|
| South, East, West, North | `ButtonA`, `ButtonB`, `ButtonX`, `ButtonY` | 0 or 1 |
| LeftTrigger, RightTrigger | `ButtonL1`, `ButtonR1` | 0 or 1 |
| LeftTrigger2, RightTrigger2 | `ButtonL2`, `ButtonR2` | 0 to 1 |
| LeftThumb, RightThumb | `ButtonL3`, `ButtonR3` | 0 or 1 |
| Select, Start | `ButtonSelect`, `ButtonStart` | 0 or 1 |
| DPadUp, DPadDown, DPadLeft, DPadRight | `DPadUp`, `DPadDown`, `DPadLeft`, `DPadRight` | 0 or 1 |
| Mode, C, Z, Unknown | not mapped (Mode stays with the OS) | |
| Axis LeftStickX/Y | `Thumbstick1` (x right, y up) | -1 to 1 |
| Axis RightStickX/Y | `Thumbstick2` | -1 to 1 |

- Use gilrs events, not only polled state: events carry press edges and connection changes. Keep polling the axes
  each frame (as `poll_gamepad` does).
- Up to 4 pads (`Gamepad(0..3)`). By default all pads feed the same actions (couch co-op is a later question); a
  binding's optional `Gamepad` filter is not in v1.
- Pad family for glyphs (letters or shapes, section 9.2): from the vendor id gilrs reports when it has one,
  else Generic. (Unverified: the exact gilrs accessor.) The player can override it in Settings.
- Rumble is out of scope.

### 4.3 Touch

winit `WindowEvent::Touch` gives finger id, phase and position. The Player converts pixels to **dp** with
`px_per_dp`: 1 dp is 1/160 inch (Android's definition, (Unverified) on the pages I read; the 48 dp rule uses it, Sourced:
https://support.google.com/accessibility/android/answer/7101858). On a phone the window scale factor gives it; on a Windows touch screen it is
`0.6 * scale_factor` (96 dpi at 100%); Studio's preview uses 1.0 (Unverified: winit's scale factor meaning on
Android and iOS). Fingers are captured by the first control they land on for their whole life (section 7.5). The
overlay turns fingers into **virtual controls**: `TouchStick`, `TouchLook` and one `TouchButton(action)` per
`ShowOnTouch` action. Their binding rows are implicit and shown read-only in Studio as "Touch: automatic".

### 4.4 Last-used device and hysteresis

State: `last_device` is KeyboardMouse, Gamepad or Touch (the three `PreferredInput` values, Sourced above). It drives
glyphs, the touch overlay's visibility and the cursor.

"Meaningful input" for each device (start values):

| Device | Counts | Does not count |
|---|---|---|
| KeyboardMouse | a key down; a mouse button down; the wheel; mouse movement of at least 12 px within 200 ms | movement below that |
| Gamepad | a button down; a stick or trigger beyond 0.5 | stick values under 0.5 (drift) |
| Touch | any finger down | none |

Switch rule: `last_device` changes to X when X has meaningful input and all of these hold:

1. The current device has had no meaningful input for 300 ms.
2. The current device holds no pressed control (a held key or button keeps it current).
3. The last switch was at least 500 ms ago (stops flicker when a player uses a pad and taps a key).

Actions already held keep working on every device; only the glyphs and overlay change. Touch overlay: shown when
`last_device` is Touch or the machine has no keyboard and no pad (a phone). On a laptop with a touch screen the
overlay fades in (140 ms) at the first touch and fades out (140 ms) when the keyboard or pad takes over; instant under
Reduce motion (tokens set `--t-base: 0ms`). Events: `UserInputService.LastInputTypeChanged`, `PreferredInput`,
`InputAction.GlyphChanged`.

## 5. Luau API

### 5.1 InputAction in scripts

Client scripts only (`LocalScript`).

```lua
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local input = ReplicatedStorage:WaitForChild("Input")
local dash = input:WaitForChild("Gameplay"):WaitForChild("Dash")     -- InputAction, Type Bool

dash.Triggered:Connect(function()        -- per the action's Trigger
    print("dash")
end)
dash.Pressed:Connect(function() end)     -- raw press edge
dash.Released:Connect(function() end)    -- raw release edge

local move = input.CharacterControls.Move
local v = move:GetState()                -- Vector2, x right, y forward, length <= 1
local held = input.CharacterControls.Run:GetState()   -- boolean
```

- `GetState()` returns the value as of the start of this frame: boolean, number, Vector2, Vector3.
- Handlers run on the client VM's next `step`, in event-timestamp order, with the usual 0.25 s handler budget.
- A disabled action or context reads false, 0, or a zero vector and fires nothing.

### 5.2 ContextActionService:BindAction (compatibility)

`ContextActionService:BindAction(name, handler, createTouchButton, ...inputTypes)` becomes an `InputAction` in a hidden
`InputContext` owned by the service:

| Roblox call | Metallic |
|---|---|
| `BindAction(name, fn, touch, ...)` | Creates (or replaces) a hidden `InputAction` named `name`. Each `Enum.KeyCode` becomes a binding. `Enum.UserInputType.MouseButton1/2/3` become the mouse controls; `Keyboard` binds all keys; `Gamepad1` binds the gamepad slot (Unverified exact set Roblox accepts). Its context priority is `Medium` (2000) unless set. |
| `BindActionAtPriority(name, fn, touch, priority, ...)` | Same with that priority. `Enum.ContextActionPriority` Low 1000, Medium 2000, High 3000 are Metallic's numbers (Roblox's are (Unverified)). Higher is called first; equal: most recent first (Sourced above). |
| handler `(actionName, state, inputObject)` | Called with `Enum.UserInputState.Begin` on press, `End` on release, `Change` on a changing axis, `Cancel` when the context is disabled or the action unbound while pressed. |
| handler return `Enum.ContextActionResult.Sink` or `Pass` | Decided per event (section 3.7). Returning nothing sinks, as in Roblox (Sourced above). |
| `createTouchButton = true` | Sets `ShowOnTouch = true`. |
| `SetTitle`, `SetImage`, `SetPosition` | `TouchLabel`, `TouchImage`, and `TouchLayout = Custom` with `TouchPosition`. |
| `GetButton(name)` | Yields until the overlay exists, returns a proxy `ImageButton` in `PlayerGui.TouchControls` on touch devices, nil elsewhere (Sourced above: nothing on non-touch devices). Phase 3 supports `Position`, `Size`, `Image`, `ImageTransparency`, `Visible`. |
| `UnbindAction(name)` | Destroys the hidden action; a pressed action gets `Cancel`. |

BindAction actions show in the Controls page under the name given and are rebindable like any action. A script that
binds a key the player already moved gets the player's binding (the override applies by name).

### 5.3 UserInputService (compatibility)

Raw events stay available for scripts that need them and for ported code. They come from the device layer after
arbitration, so `gameProcessedEvent` is true when a GUI or a sinking consumer took the input (section 3.7).

| Member | Metallic |
|---|---|
| `InputBegan/InputChanged/InputEnded(input, gameProcessed)` | `input` is an `InputObject` with `KeyCode`, `UserInputType`, `UserInputState`, `Position` (Vector3), `Delta` (Vector3). |
| `IsKeyDown(keyCode)` | From the key state table. |
| `GetMouseDelta()`, `MouseBehavior`, `MouseIconEnabled` | As A20 plans. |
| `GetLastInputType()`, `LastInputTypeChanged`, `PreferredInput` | From section 4.4. |
| `TouchEnabled`, `KeyboardEnabled`, `MouseEnabled`, `GamepadEnabled`, `GetConnectedGamepads()`, `GamepadConnected` | Device presence. |

Issue #80 owns this service; this spec adds only the last-input members and says raw events come after arbitration.
Docs and templates teach actions first.

### 5.4 ProximityPrompt and GUI buttons

`ProximityPrompt` (A15) shows its key through the glyph system: if the local player's game has an action with
`Role = Interact`, the prompt uses it (so a rebinding changes the prompt); else the prompt's own `KeyboardKeyCode`
(default E) and `GamepadKeyCode` (default `ButtonX`) (property names Unverified). On touch the prompt is a tappable
button. `TextButton` and `ImageButton` activate on the pointer's up event, with cancel by sliding off (XAG 107).

### 5.5 Server and client

Input exists only on the client. On the server:

- Reading `GetState()` or connecting `Triggered` errors with `InputAction can only be used from a LocalScript`
  (same style as `FireClient can only be called from the server`).
- The instances exist on the server (they replicate with the scene) but never fire.

A client script sends what the server needs. The server checks it. Example, a dash with a 2 second cooldown:

```lua
-- LocalScript (client)
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local dash = ReplicatedStorage.Input.Gameplay.Dash
local request = ReplicatedStorage:WaitForChild("DashRequest")   -- RemoteEvent

dash.Triggered:Connect(function()
    request:FireServer()           -- send the intent, never a position or a speed
end)
```

```lua
-- Script (server)
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local request = ReplicatedStorage.DashRequest
local cooling = {}

request.OnServerEvent:Connect(function(player)
    if cooling[player] then return end        -- ignore a request inside the cooldown
    cooling[player] = true
    -- apply the dash to player.Character here
    task.delay(2, function() cooling[player] = nil end)
end)
```

Rules for creators: send an intent (a name, a direction as a unit Vector2 or Vector3), not a result; the server
checks cooldown, range and state; rate limits are the remote limits in `docs/luau-api.md`. From M2 the character's
`Move`, `Jump` and `Run` do not go through scripts: the engine sends them in the input datagram every tick (networking
research, `Input` channel, 30 packets per second with redundancy). Section 6's `ActionFrame` is a plain struct for
that reason. Details belong to the networking spec.

## 6. The character

### 6.1 Default actions

Context `CharacterControls`, priority 0, no sink:

| Action | Type | Bindings | Notes |
|---|---|---|---|
| `Move` | Direction2D | `W` `S` `A` `D` composite (Up Down Left Right); arrow-key composite; `Thumbstick1` (Deadzone 0.18); `DPadUp/Down/Left/Right` composite; touch stick (automatic) | x right, y forward. Role Move. `ShowOnTouch` true (stick). |
| `Jump` | Bool | `Space`; `ButtonA`; touch button (automatic) | Held state matters: held jumps again on landing. Role Jump. Primary touch button (`TouchOrder` 0). |
| `Run` | Bool | `LeftShift`, `RightShift`; `ButtonL3`; touch button (automatic) | Role Run. `CanToggle` true. On touch it starts as a toggle (S1). |
| `Look` | Direction2D | `MouseDelta` with `PrimaryModifier = MouseButton2`, `Scale` 0.006, `Vector2Scale` (1, -1); `Thumbstick2` with `Scale` 3.0 and `ScaleByDeltaTime`; touch look pad (automatic) with `Scale` 0.01 | Radians this frame. Role Look. |
| `Zoom` | Direction1D | `MouseWheel` with `Scale` -2.0; `ButtonR1` (+) and `ButtonL1` (-) with `Scale` 12 and `ScaleByDeltaTime` | Studs this frame, positive moves the camera away. Role Zoom. Gamepad zoom is new (start value). |

The numbers 0.006, 3.0 and 2.0 are today's `MOUSE_ORBIT_SPEED`, `STICK_ORBIT_SPEED` and `ZOOM_PER_NOTCH`. The touch
look scale (0.01 rad per dp) is a start value. `Vector2Scale` (1, -1) turns screen-down mouse movement into
"y up" so that every Direction2D action reads y up.

### 6.2 Feeding MoveInput

Each frame, before the fixed ticks:

```rust
// metallic_runtime, in Game::frame's caller (the Player, the Studio viewport, headless tests)
let a = input.evaluate(now, dt);           // InputRuntime
let frame = FrameInput {
    movement: a.axis2("CharacterControls", "Move"),            // [x right, y forward], |v| <= 1
    jump:     a.bool("CharacterControls", "Jump"),
    run:      a.bool("CharacterControls", "Run"),
    orbit:    { let l = a.axis2("CharacterControls", "Look"); [l[0], -l[1]] },
    zoom:     a.axis1("CharacterControls", "Zoom"),
};
game.frame(&frame, dt);                    // unchanged
```

`Game::tick` then does what it does now:

```rust
let move_input = MoveInput {
    dir: self.camera.move_direction(input.movement),   // camera-relative, world space
    jump: input.jump,
    run: input.run,
};
```

So `MoveInput` itself does not change. `MoveInput::dir` keeps its contract (Y ignored, lengths above 1 count as 1, a
half-tilted stick walks at half speed). `a.axis2("...", "Move")` is looked up once at start and cached as an index,
not by string, each frame. The old `Action` enum and `InputState::set` are replaced by the device layer; `FrameInput`
stays as the boundary between input and `Game`. If the player's keys are all up, every field is zero and the character
stands still, as today.

### 6.3 Lookup when the data model has no input instances

If no `InputContext` named `CharacterControls` exists in the scene (old scenes, scripts-only tests), the runtime
builds the table above in memory. If one exists, **the scene wins entirely**: an action missing from it reads zero.
There is no per-action fallback, so a template can remove Run by deleting or disabling it. Assistant's call.

### 6.4 How a template overrides the defaults

1. **Change a binding**: edit the `InputBinding` children in Studio (Platformer 2D adds `ButtonX` to Run, the doc says
   "hold Shift or gamepad X").
2. **Remove an action**: set `Run.Enabled = false`.
3. **Replace the character controls**: set `CharacterControls.Enabled = false` and add a context of your own
   (Tower Defense: `Pan`, `Zoom`, `TurnLeft`, `TurnRight`; WASD no longer walks anyone):

```lua
-- LocalScript in StarterPlayerScripts
local input = game:GetService("ReplicatedStorage").Input
input.CharacterControls.Enabled = false
input.TowerDefense.Enabled = true
```

4. **Take over keys for a while**: enable a context with a higher priority and `Sink = true` (a vehicle, a menu, build
   mode). Disable it to hand the keys back.

## 7. Automatic touch controls

Mockup: `docs/design/input-actions/touch-layout.html`. Everything is in dp. The overlay is drawn by the Player with the
visual-language tokens: graphite fills, silver strokes, restrained violet for the primary button and the held knob; no
glow, no neon.

### 7.1 Stick

| Item | Value (phone / tablet) | Notes |
|---|---|---|
| Zone | left half of the safe area, below its top 15% | Start value. A touch that begins on a button or GUI does not start a stick. |
| Mode | Floating (default) or Fixed | Player setting. Floating: the base appears where the thumb lands. Fixed: base centre at safe-area left + 24 + R, bottom - 24 - R. |
| Base radius R | 60 / 70 dp | Diameter 120 dp. Start value. |
| Knob diameter | 56 / 65 dp | At least 48 dp. |
| Travel | R. Past R the base follows the finger so the knob stays at R. | |
| Dead zone | 0.15 R (9 dp) | Rescaled so output starts at 0 (section 3.5). Start value. |
| Output | Direction2D, y up, as virtual control `TouchStick`. | Move's automatic touch row reads it. |
| Opacity | hint ring 35%; base while held 55%; knob 85% | Hint ring shown until the first touch of the session. Player slider (25% to 100%) scales all. |
| Fade | 140 ms (`--t-base`) | Instant under Reduce motion. |

A second Direction2D action with `ShowOnTouch` and `TouchSlot` Look (Look by default) gets a **look pad**: any finger that
begins on the right half and not on a button drags the camera by `Scale` (0.01 rad per dp) times dp moved. If a game
has no such action, those touches become pointer touches (`ViewportPosition`, tap acts as `MouseButton1`).

### 7.2 Buttons

Every enabled action with `ShowOnTouch` and a Bool type gets a button, sorted by `TouchOrder` (ties: tree order).

| Item | Phone / tablet |
|---|---|
| Primary (`TouchOrder` lowest) | 72 / 84 dp circle, bottom-right, centre at safe-area right - 24 - size/2, bottom - 24 - size/2 |
| Secondary | 56 / 64 dp circle |
| Ring 1 around the primary centre | radius 84 dp, angles 180, 130, 80 degrees (0 is right, counter-clockwise) for S1, S2, S3 |
| Ring 2 | radius 152 dp, angles 175, 140, 105 for S4, S5, S6 |
| Tablet | both ring radii times 1.167 |
| Opacity | 90%; pressed: 100% and 94% size |
| Icon | the action's glyph or `TouchImage`, 24 dp; label under the button from `TouchLabel` or `DisplayName` (caption size, 11 px at 100%) |

Checked rules (a layout test, section 14): every control is at least **48 dp**; at least **8 dp** between control edges
(Sourced: https://support.google.com/accessibility/android/answer/7101858); at least **16 dp** inside the safe area;
nothing overlaps the stick zone's default stick. At most 6 buttons are placed; Studio warns above 5. Verified by hand for
844 x 390 dp with insets (48, 0, 48, 20), 640 x 360 dp, and 1180 x 820 dp with the tablet factor.

**Large controls** preset (Settings and Accessibility): every control is at least the Xbox minimum, 15 mm on a phone
(94 dp) and 24 mm on a tablet (151 dp) (Sourced: XAG 107 above; mm to dp: 15 mm / 25.4 * 160 = 94). The layout grows
the ring radii to keep the 8 dp gaps. If the buttons no longer fit inside the safe area, only as many as fit are placed, in
`TouchOrder`, and Studio's check lists the rest as not reachable with Large controls.

The `Menu` button is a 48 dp pause icon at the top centre of the safe area, 16 dp down.

### 7.3 Customisation in Studio

Studio's device preview (section 10.2) has "Edit touch layout":

- Drag a control; it snaps to a 4 dp grid; hold Alt to move freely. Drag a corner to resize in dp, never below 48 dp.
- Guides: safe-area edge, 16 dp inner margin, 8 dp spacing, stick zone. A control closer than 8 dp to another shows a
  warning chip; one under 48 dp is refused.
- "Reset layout" sets `TouchLayout = Auto`. Moving sets `TouchLayout = Custom`, `TouchPosition` (Scale from the safe area,
  Offset dp) and `TouchSize`. Positions are relative to the safe area so a phone and a tablet both work. The editor checks
  both the Phone and Tablet preview and the portrait orientation before saving.
- Everything has a keyboard path: select a control with Tab, arrow keys nudge by 4 dp, Shift+arrows by 16 dp, `[` and `]`
  resize (XAG 107: the customisation must be accessible). Shortcuts are registered in the keymap data (#129).

### 7.4 Safe areas

- Content rect = screen minus the OS safe-area insets: display cutouts and system bars (Sourced: https://developer.android.com/develop/ui/views/layout/edge-to-edge).
- Keep controls out of the system gesture zones: at least 24 dp above the bottom edge when the OS reports a bottom gesture
  inset, and at least 16 dp from the left and right edges where back gestures live (Sourced for the zones, same page;
  the dp values are ours).
- Where winit gives the insets, use them (`Window::safe_area` in newer winit, Unverified for the version we use); else
  the Player takes them from the platform layer on Android and iOS and uses zero on desktop.
- Landscape and portrait both work: the rule is anchored to bottom corners, so portrait just has a narrower zone. A game
  can lock an orientation with a project setting; the default is both (XAG 107).
- Device presets for Studio (width x height in dp, insets left, top, right, bottom). Typical sizes, not measurements of
  one product: Small phone 640 x 360, (0, 0, 0, 0); Phone 844 x 390, (48, 0, 48, 20); Tablet 1180 x 820, (0, 0, 0, 20).

### 7.5 Touch routing

- A finger is **captured** by the first thing it lands on, for its life: stick (zone), a button, the look pad, GUI, or
  pointer. Up to 10 fingers.
- A button activates on **touch down** (jump, fire and dash are essential down events, XAG 107 allows this), and releases
  on lift. If the finger slides more than 2x the button radius away, the press is cancelled without `Triggered` (the
  cancel XAG 107 asks for).
- Buttons keep 8 dp inactive space around them; a touch there does nothing (it does not start a stick).
- Menu and GUI buttons activate on **up**, with cancel by sliding off.
- Pinch zoom is not in v1. XAG 107 says multi-point gestures need another way; Zoom on touch comes with buttons first.

## 8. Rebinding UI for players

### 8.1 The Controls page

Built into the Player, in Menu > Settings > Controls. Drawn with the visual-language components (egui), keyboard and
gamepad operable, so a player who cannot use the mouse can rebind (XAG 107).

- Tabs: **Keyboard and Mouse**, **Gamepad**, **Touch**.
- One row per `Rebindable` action, ordered by context priority then tree order: icon, `DisplayName`, up to two binding
  slots (Primary, Alternate) for the tab's device, a Hold/Toggle switch for actions with `CanToggle`, a "Reset" icon.
- Direction2D actions with a composite show four slots (Up, Down, Left, Right) or "Left stick" / "Right stick" for pads.
  A "Swap WASD and arrows" shortcut is offered.
- Per stick and for the mouse: sensitivity 25% to 200% (XAG 107 asks for at least 50% of default each way), Invert X,
  Invert Y, dead zone 0 to 0.4. These write override values, not the creator's bindings.
- Touch tab: stick mode (Floating/Fixed), control size 0.8x to 1.5x (never below 48 dp), opacity 25% to 100%, swap sides
  (left-handed), "Edit layout" (drag and resize with the Studio rules in 7.3), preset layouts (section 11.3), Large controls.
- Footer: "Reset this tab", "Reset all" (both ask once, then show an Undo toast for 10 s).

### 8.2 Listening

Choose a slot: "Press a key, button or move a stick. Esc cancels, Backspace clears." (When the slot is Menu, Esc binds
instead of cancelling; the Cancel button or gamepad `ButtonB` cancels.) Listening takes the first control that crosses its press
threshold (the largest magnitude if several move together, as Unity's interactive rebinding does, Sourced above). It
times out after 8 s. Modifier keys alone do not bind; Shift or Ctrl followed by a key binds a chord only if the action
allows a modifier (creator-set chords stay). Listening runs on its own context at priority 20000 with sink, so nothing
else reacts. The control is read on **press** and the page ignores the release of that same press.

### 8.3 Conflicts and rules

A conflict is the same control on two **enabled actions in the same `RebindGroup`** (default "Game"; menu contexts use
"Menu"). Actions in different groups never conflict.

On a conflict the player picks:

| Choice | Result |
|---|---|
| **Swap** (default, focus) | The other action gets the control this slot had. |
| **Replace** | The other action loses the control; its row shows "Unbound" in the `warning` colour. |
| **Cancel** | Nothing changes. |

Rules: never unbind silently; a `Required` action cannot be left with no binding on a device tab that has any; `Menu` always
keeps one binding per device so the player can always open the page; unbound rows stay visible and are listed in a banner at
the top ("2 actions have no binding"). A control reserved by the OS (Alt+F4, Windows key, `ButtonMode`) is refused with a
message. Players can bind **any** action to any control, including Esc (XAG 107 names Esc), as long as the rule for Menu
holds.

### 8.4 Storage

Local first, cloud later.

- File: one JSON file per game and per local profile in the Player's user data folder, `input/<game id>/<profile>.json`
  (folder is the Player's existing settings location; exact path is a Phase 5 detail). `<profile>` is `local` until sign-in
  exists. No account names, emails or server addresses go in the file.
- Content: only differences, keyed by `Context/Action`, so new actions and creator changes still apply:

```json
{
  "version": 1,
  "overrides": {
    "CharacterControls/Jump": { "bindings": [ { "device": "Keyboard", "key": "E" } ], "toggle": false },
    "CharacterControls/Look": { "invertY": true, "sensitivity": 1.25 }
  },
  "touch": { "stick": "Fixed", "scale": 1.2, "opacity": 0.8, "swapSides": false,
             "positions": { "Gameplay/Dash": [0.78, -64, 0.62, -40] } },
  "glyphs": { "gamepadStyle": "Shapes" },
  "updated": "2026-10-30T12:00:00Z"
}
```

- Apply order: scene bindings, then the override for that action and device. Overrides replace the bindings of that device
  only; other devices keep the creator's. Unknown keys are kept and written back (a newer game version may use them).
  An override whose action no longer exists is kept but ignored.
- Cloud later: the same JSON stored with the account per game; merge per action by `updated`; the last change wins.
  Local file stays the working copy so offline play works. Not in the first phases.
- Writes happen when the page closes or the game exits, never per frame, atomically (write a temp file, rename).
- Implemented (#254): the folder is `%APPDATA%\MetallicPlayer` on Windows (`METALLIC_PLAYER_DATA_DIR` overrides it), so
  the file is `%APPDATA%\MetallicPlayer\input\<game id>\local.json`; `<game id>` is the name as a slug plus 8 hex
  digits of an FNV-1a hash of the exact name (names that differ only in case or punctuation get separate folders, and no id is a reserved Windows
  name). Files
  over 256 KB are set aside, at most 4 slots per device are read, and a Menu with no binding on a device gets the
  creator's binding back. Stick and mouse settings are stored per stick under
  `"sticks"` (`Thumbstick1`, `Thumbstick2`, `MouseDelta`: `sensitivity`, `invertX`, `invertY`, `deadzone`) rather than
  per action, so a stick swap keeps them; assist options under `"assist"` (`toggleAll`, `replaceHolds`,
  `holdTimeScale`). An empty slot is stored as `"key": "None"`. A file that cannot be read is renamed to
  `local.json.corrupt` and the creator's controls apply.

## 9. Glyphs

### 9.1 API

`action:GetGlyph(device?)` returns a table, never nil:

```lua
local g = jump:GetGlyph()        -- uses UserInputService.PreferredInput
-- g.Image   "metallic://glyph/gamepad/letters/button_a"   (usable as ImageLabel.Image)
-- g.Text    "A"            (keyboard: "Space", touch: "Tap", unbound: "Unbound")
-- g.Device  "Gamepad"      ("Keyboard", "Gamepad", "Touch")
-- g.Binding the InputBinding it describes (nil when unbound)
```

- `device` may be `Enum.PreferredInput.KeyboardAndMouse`, `Gamepad` or `Touch`.
- `action:GetGlyphs(device?)` returns all bindings. Composites return one glyph with a combined image (WASD as four
  keycaps) and `Text` "W A S D".
- `action.GlyphChanged` fires when `last_device`, the pad family or a binding changes (also on a rebind), so prompts update
  without polling. Glyphs are cached per (action, device) and rebuilt on that signal; a call allocates nothing after the
  first.
- `UserInputService:GetImageForKeyCode(keyCode)` (Roblox member, name Unverified) returns the same image for a key.
- The Controls page, the touch overlay, `ProximityPrompt` and the Studio panel all use `GetGlyph`; no UI formats key names
  by itself, so a rebinding is correct everywhere (XAG 107: "labelling of the new mapping is represented correctly").

### 9.2 Our icon set and what we may depict

We draw our own SVG set. Steam Input also ships glyphs per origin and style as SVG or PNG (Sourced above); we do the same
with our own art.

May depict:

- Keycaps (rounded rectangle with a legend in our UI font), arrows, mouse silhouettes with the pressed button filled.
- A generic gamepad: two sticks, a four-button face diamond, d-pad, two bumpers, two triggers, two small centre buttons.
- Face-button **layouts** in two styles, each drawn by us in the graphite and silver palette of the tokens:
  **Letters**: A, B, X, Y in a plain sans-serif font, bottom, right, left, top. **Shapes**: a circle, a cross, a square and a
  triangle as simple outline shapes, in the same positions.
- Positions are named as positions in our docs (South, East, West, North); `ButtonA` is the bottom button.
- Implemented (#254): **Letters** and a generic **Positions** style (a diamond of four dots with the pressed one
  filled, words "Bottom Button" and so on). **Shapes** waits for an owner decision. Auto picks Positions for pads with the
  Sony or Nintendo vendor id and Letters otherwise; the player can choose on the Gamepad tab.

May not depict:

- Any console or controller **logo**, word mark or product name in art (no "Xbox", "PlayStation", "Nintendo", "Steam" text
  or logos, no Guide, Home or PS button emblem, no controller silhouette traced from a product).
- The console makers' **colour schemes** for the face buttons (the coloured letters and the coloured shapes). Ours are
  monochrome.
- Controller outlines that copy a specific product's body shape.
- Anything copied from another engine's or store's icon packs.

Naming in the UI: "Letters" and "Shapes", not brand names. The docs may name brands to explain which pads use which style
(nominative use); shipped art and strings do not. Auto choice: Shapes for pads whose vendor is the PlayStation-style
family, Letters otherwise; the player can override. Whether the shapes alone are safe to ship is a legal question I cannot
answer: (Unverified) and listed for the owner in section 17.

### 9.3 Files

`assets/glyphs/` (repo root `assets`, not `branding/`): `keyboard/` (about 110 keys, generated from one keycap template plus
the legend; hand-drawn only for arrows, Shift, Enter, Backspace, Tab, Esc, Caps, Ctrl, Alt, Space), `mouse/` (5), `gamepad/letters/`
and `gamepad/shapes/` (face buttons, 4 shoulders and triggers, sticks with click and move, d-pad parts and whole, Start, Select;
about 28 each, only the four face buttons differ between styles), `touch/` (stick, tap, hold, swipe). SVG, 64 x 64 viewBox, 2 px
strokes, `currentColor` plus one fill role, so themes and High contrast (#134) work. Rendered once at 32, 48, 64 dp into
an atlas of at most 1 MB. Licence: the repo's own (our art). Rasteriser: whatever egui's SVG loader uses; its licence must pass
`cargo deny` (Unverified which crate that is).

## 10. Studio

### 10.1 Input Actions panel

Opens from the View menu and Ctrl K ("Input actions"); no new single key is taken (see 10.4).

- **Left**: tree of `InputContext` > `InputAction` > `InputBinding`, with Enabled eye and Priority.
- **Centre**: a **bindings matrix**. Rows are actions, columns Keyboard and Mouse, Gamepad, Touch. Cells show glyphs. Click a
  cell to add a binding. **Listen** captures a real key, button or stick from the connected devices (the Controls page's
  listening, 8.2). Touch cells show "Auto" for `ShowOnTouch` actions.
- **Right**: the property grid of the selection, with Roblox properties first and "Metallic" additions grouped below.
- **Checks** strip at the bottom (the same list feeds #182's Accessibility check):

| Check | Level |
|---|---|
| Same control on two enabled actions in one context (and not in a sinking-priority split) | Error |
| An action with no gamepad binding, or no keyboard binding (Bool and Direction types) | Warning |
| `ShowOnTouch` buttons above 5 | Warning; above 6 Error |
| Touch size under 48 dp, or gap under 8 dp, or outside the safe area in any preset | Error |
| Hold trigger over 1000 ms | Warning; over 3000 ms Error (XAG 107: avoid holds past 2 to 3 s) |
| A Bool action that is held for long (`Run`, `Aim`) with `CanToggle` false | Warning |
| `Esc` bound on any action (it opens the Player menu by default) | Error |
| Key reserved by Studio during Play (10.4) | Info |
| Action renamed (breaks players' overrides) | Info |

### 10.2 Preview as

The Play menu caret (next to Play, UI doc 5.7) gains **Preview as**: Keyboard and mouse (default), Phone, Tablet, Gamepad.

- **Phone / Tablet**: the viewport takes the device's size in dp (letterboxed), safe-area insets from the preset (7.4), and
  forces `last_device = Touch`. The touch overlay draws over the game. The mouse acts as one finger: left button down is a
  touch. A **right click** pins or releases a second finger at that spot, so a stick and a button can be held together.
  A dropdown picks portrait or landscape and the preset; "Show safe area" draws the insets.
- **Gamepad**: forces `last_device = Gamepad` so prompts show pad glyphs (letters or shapes selectable). Uses a real pad when
  connected; otherwise a small **virtual gamepad** window (clickable buttons, two draggable sticks) feeds the same device layer.
- Preview does not change the scene. It sets Studio's editor state only (like Play camera options).
- Headless equivalent for CI and screenshots: `metallic-player <project> --emulate phone|tablet|gamepad`
  (Phase 3, with `--capture`).

### 10.3 Touch overlay on desktop

The overlay is the Player's own code, so it is the same in the Player with `--emulate phone` and in Studio. On a desktop
without Preview as, the overlay shows when a real touch screen is used (section 4.4).

### 10.4 Studio keys and game keys

Studio shortcuts (UI doc 5.15, building tools section 15) must not steal game input, and games must not steal Studio's.
While Play has focus in the viewport:

- Plain keys, Shift+key and the mouse go to the game.
- **Studio keeps**: F1 to F12 (Play F5, Stop Shift+F5, Pause F8, Build in Play F4, panel focus F6, help F1), every chord
  with Ctrl or Alt (Ctrl K, Ctrl B, Ctrl+Shift+F5 and the rest), Esc goes to the game (it opens the Player menu); F6 and
  Shift+F5 leave the viewport.
- The Input Actions panel marks a game binding on a reserved key as "Studio keeps this key in Play; it works in the Player".
- In the standalone Player nothing is reserved except what the OS owns.
- The panel's own shortcuts are keymap data (#129), not hard-coded. The three presets (Metallic, Roblox Studio, Unity-style)
  never use the game's actions. The building-tools keys R, T, X, F, C, H, J, 1 to 9 for BuildKit are **game actions** (that doc
  says "All are #180 input actions"), so they are separate from the Studio keymap even though the letters match.

## 11. Accessibility

Xbox Accessibility Guideline 107 (Sourced above) drives this section. Player settings live in the Controls page and apply to every
game that uses actions, so a creator gets them without doing anything.

### 11.1 Hold to toggle (#182)

- Every Bool action with `CanToggle` (default true) can be a toggle for the player: a press flips the state, a release is
  ignored, a second press flips it back. `Run`, `Aim`, `Crouch`, `Fire` (auto-fire) are the targets.
- Settings: per action "Hold / Toggle" (Controls page row) and one switch "Toggle every hold action".
- Toggled state is visible: the touch button stays filled, the HUD can read `action:GetState()`. A "Toggle" badge shows on the
  prompt glyph. The toggle clears when the character dies or the game calls `action.Enabled = false`.
- **Hold trigger**: a setting "Replace holds with presses" makes a `Hold` action fire on press (the Long Dark's "accessible
  interactions" does this, Sourced in XAG 107 page). A second setting scales `HoldTime` by 0.5x to 3x.
- Touch: the Run button defaults to toggle; sliding off still cancels.

### 11.2 Remap everything

Section 8: every action, every device, Menu included; swap and replace; sensitivity and invert per stick; no action needs two
simultaneous presses by default (the engine's default contexts need none; creators are warned in the editor if a chord is
required). Touch: size, position, opacity.

### 11.3 One-handed presets

Presets are remaps by `Role` and apply only to actions that have one; the Controls page lists actions without a Role as
"not changed". Start values, to be tuned with testers (Unverified, owner decision):

| Preset | Move | Look | Jump | Run | Zoom |
|---|---|---|---|---|---|
| Keyboard, left hand | `W A S D` | `Q`/`E` turn, `R`/`F` pitch | `Space` | `LeftShift` toggle | `Z`/`X` |
| Keyboard, right hand | Arrows | `Numpad 4`/`6` turn, `8`/`2` pitch | `RightCtrl` | `RightShift` toggle | `PageUp`/`PageDown` |
| Gamepad, left hand | `Thumbstick1` | D-pad | `ButtonL1` | `ButtonL2` toggle | none |
| Gamepad, right hand | `Thumbstick2` | face buttons (X/B turn, Y/A pitch) | `ButtonR1` | `ButtonR2` toggle | none |
| Touch, one thumb | fixed stick at the chosen side | look pad off (camera follows) | primary button above the stick | toggle | none |

For one-thumb touch both stick and buttons sit in one 200 dp column at the chosen side; secondary buttons stack above the
primary with 8 dp gaps; "camera follows" needs the camera design (#72) and is limited to games that set it (open question).
Presets only change player overrides; the page shows what changed and "Reset" returns to the creator's bindings.

Also from XAG 107, supported by this design: analog and digital alternatives (every Direction action can be bound to keys);
two-stick games are not forced (Look has key, d-pad and touch routes); sticks' click (`ButtonL3`, `ButtonR3`) is rebindable;
sensitivity sliders; the customisation page itself is operable with keyboard and gamepad. Not covered here: switch access,
voice control and eye tracking (platform features; AccessKit work is tracked in the learning-course accessibility pass).

## 12. Templates

Migration plan. "Defaults" means `CharacterControls` only. Each template gets a `ReplicatedStorage/Input` folder in its scene;
Parkour needs nothing else. Action names are proposals; each template issue finalises them. Key facts are from `docs/templates/*.md`.

| Template | Today (docs) | Actions and contexts | Touch plan |
|---|---|---|---|
| Parkour (`templates/parkour`) | Defaults only (WASD, Space, Shift, right drag) | Defaults. Add the `CharacterControls` context to the scene; mark `Jump` primary and `Run` toggle. **Nothing else changes** (the issue's done-when). | Stick, Jump, Run. |
| Platformer (2D) | Walk, run with Shift or gamepad X, jump | Defaults, with `Run` gaining `ButtonX`; `Move` uses x only. Variable jump height reads `Jump:GetState()` held. | Stick locked to the x axis (`TouchStick` axis lock is a Phase 3 detail); Jump, Run. |
| FPS Arena | Fire, aim (right mouse), reload, slot keys, Shift run, raw mouse look | `Weapons` context (priority 10): `Fire` (`MouseButton1`, `ButtonR2`), `Aim` (`MouseButton2`, `ButtonL2`; CanToggle), `Reload` (`R`, `ButtonX`), `NextSlot`/`PrevSlot` (wheel, `ButtonR1`/`L1`), `Slot1..4` (`1`..`4`). Defaults for the rest. | Fire (primary), Aim, Reload, Jump; slots via a swap button; look pad with sensitivity in Settings. |
| Battle Royale | Space to jump and glide on the carrier, inventory keys 1 to 3 and 4 to 5, wheel, use and cancel | `Gameplay` (glide uses `Jump`), `Inventory` context (priority 20, Sink while open: `Slot1..5`, `CycleSlot` wheel, `Use`, `Cancel`), plus FPS `Weapons`. | Jump, Fire, Aim, Use; Inventory is a GUI bar (tap). |
| Build Sandbox | Mouse ray, R rotate, T tilt, X delete, F paint, C clone, H/J grid, Ctrl+Z, 1 to 9 for blocks; gamepad shoulders cycle, triggers place and delete (doc) | `Build` context (priority 20): `Place` (`MouseButton1`, `ButtonR2`, touch tap), `Delete`, `Rotate`, `Tilt`, `Paint`, `Clone`, `GridToggle`, `GridSize`, `Undo` (chord `LeftCtrl`+`Z`), `Slot1..9`, `Pointer` (ViewportPosition). | Pointer touches (no look pad while building), Place as a button, Rotate, Delete, Undo. |
| Horror | Sprint, crouch, flashlight keys, prompts show their key | Defaults + `Crouch` (`C`, `ButtonB`; CanToggle), `Flashlight` (`F`, `ButtonY`), `Interact` role on the prompt. `Run` renamed Sprint in the DisplayName only. | Crouch toggle, Flashlight, Interact (prompt is tappable). |
| Tower Defense | WASD or right drag to pan, wheel zoom, Q/E turn 90 degrees, hotkeys 1 to 4, gamepad sticks | `CharacterControls.Enabled = false`; `TowerDefense` context: `Pan` (Direction2D), `Zoom`, `TurnLeft`, `TurnRight`, `Tower1..4`, `Select` (Pointer). | Pan with the stick, Zoom +/- buttons (no pinch in v1), `Select` as a pointer tap; docs say "touch drag later". |
| Party Rounds | Q dashes (or the touch button) with a 2 s cooldown; defaults | Defaults + `Dash` (`Q`, `ButtonX`), the RemoteEvent pattern in 5.5. | Dash is S1 (the doc says "the touch button"). |
| Simulator | Click or hold to swing; pick under the mouse or nearest in 10 studs | `Swing` (`MouseButton1`, `ButtonR2`; Trigger Repeat so holding keeps swinging), `Pointer`. | Swing button (hold repeats); tap a crystal. |
| Tycoon | Mostly GUI and prompts; defaults | Defaults + `Interact` role. | Prompts are tappable; stick, Jump. |
| Survival Nights | Tool swing, hotbar, build/craft | `Gameplay`: `UseTool` (Repeat), `Interact`, `Hotbar1..5`, `Inventory`. Names proposed; the doc gives few input details (Unverified). | UseTool, Interact, Jump. |

Rules for every template: each Bool action a template needs on touch sets `ShowOnTouch`; the template doc's accessibility
checklist ("Keyboard and mouse, gamepad, and (from the mobile milestone) touch all reach every action the game needs", `docs/templates/quality-bar.md`)
is checked by the Studio checks in 10.1. A migrated template keeps its server scripts unchanged: they only receive RemoteEvents.

## 13. Performance

Budget row from `docs/research/2026-10-30-performance-and-scale.md` section 3.2: **Input, window events 0.2 ms (Dev, 60 fps),
0.3 ms (Mid, 60 fps), 0.5 ms (Low, 30 fps)** of CPU main-thread time per frame. Input action evaluation takes part of that row:

| Item | Dev | Mid | Low |
|---|---|---|---|
| `InputRuntime::evaluate`, reference load (4 contexts, 32 actions, 96 bindings, 40 raw events) | 0.05 ms | 0.10 ms | 0.20 ms |
| Rest of the row (winit and gilrs polling, queueing) | 0.15 ms | 0.20 ms | 0.30 ms |
| Touch overlay draw (inside the "Game UI" row 0.5 / 0.7 / 1.5 ms) | 0.10 ms | 0.15 ms | 0.30 ms |
| Luau signal dispatch for input events (inside the Scripts row) | at most 16 events a frame, 0.1 ms | same | at most 8 events, 0.2 ms |

These are assistant's calls (start values), not measurements. The reference load is far above a game's real needs (Parkour has
5 actions and about 20 bindings). Rules that make them realistic:

- No heap allocation per frame in `evaluate`: bindings are resolved once into dense arrays (control index to a list of binding
  indices) and rebuilt only when the world's change journal reports an input instance changed. Events go to a reused `Vec`.
- Lookup by index, never by string, in the hot path. `GetState()` is a field read.
- At most 1,024 raw events per frame; repeated mouse moves merge; if the queue is full, button edges win.
- Signal dispatch is capped (above); excess `StateChanged` for axes are merged to the last value, button edges are never merged.
- Glyph lookups are cached (section 9.1); the atlas is at most 1 MB.
- Input to motion: events are read at the start of the frame, before the fixed ticks, so a key pressed before the frame moves
  the character in that frame's first tick: at most one frame (16.7 ms at 60 fps) of added latency from input handling.
- A criterion bench `input_eval/96_bindings` runs in the perf CI lane (#122) and fails above 2x the Dev number (0.1 ms).
- The numbers above are checked by that bench when Phase 1 lands; if the bench measures lower, tighten them.

## 14. Testing

- **Pure unit tests** (no window, no gilrs, no GPU): dead zone (`0.17` gives 0, `1.0` gives 1, `0.59` rescales), composites
  (opposite keys cancel, diagonals normalise to length 1), response curve, modifiers and chords (Shift+W beats W),
  every trigger with simulated time (Hold at 499 and 500 ms, Tap at 199 and 201, DoubleTap at gap 299 and 301, Repeat at
  400, 500, 600 ms), toggle, priority and sink (vehicle sinks W and S, A and D pass), last-device hysteresis (drift 0.4 ignored,
  300 ms and 500 ms rules), override application order, override file round trip, glyph choice per device.
- **Headless injection**: `InputRuntime::push(RawEvent)` is the only entry. The Player gets `--input-script <file>`: one event
  per line, `frame device control state [value]`, for example:

```
30  keyboard  Space       down
36  keyboard  Space       up
40  gamepad0  Thumbstick1 axis  0.0 1.0
90  touch0    Down        pos   140 300
```

  The parkour route test (`crates/metallic_runtime/tests/parkour_route.rs`, "a bot using only player input finishes the
  course") is rewritten to drive keyboard, gamepad and touch event streams through the same route and expects the same finish.
  This is the issue's done-when ("plays fully on keyboard, on a gamepad and in the touch emulator").
- **Fake source**: `trait RawSource` has a `ScriptedSource` implementation for tests; the gilrs source is tested by its mapping
  table only (no real hardware in CI).
- **Touch layout**: a pure function `layout(device_profile, actions) -> Vec<Rect>`; property test over the three presets, both
  orientations, 1 to 6 buttons, Standard and Large: every rect at least 48 dp, gap at least 8 dp, inside the safe area minus
  16 dp, no overlap with the default stick. A screenshot test with `--emulate phone --capture` for the mockup match (checked by
  eye, attached to the PR).
- **Luau tests** in `crates/metallic_script/tests/input.rs`: `Triggered`, `Pressed`, `Released`, `GetState`; server-side use errors;
  `BindAction` handler order, `Sink` and `Pass`, `UnbindAction` cancel; RemoteEvent round trip carrying a `Vector2`.
- **Accessibility**: toggle and hold-replacement tests; rebind with conflict (swap, replace, cancel); reset.
- **Studio**: the checks list (10.1) as unit tests on a sample scene.

## 15. Phasing

Five follow-up issues, each small, in this order (filed with PR #247).

| Phase | Issue | Area | Priority | Blocked by |
|---|---|---|---|---|
| 1 | #250 Runtime: input actions core and keyboard/gamepad on the Player | runtime | P1 | this PR |
| 2 | #251 Luau: InputContext, InputAction, ContextActionService and Vector2 | script | P1 | #250 |
| 3 | #252 Player: automatic touch controls and device emulation | runtime | P1 | #250 |
| 4 | #253 Studio: Input Actions panel, Preview as and touch layout editing | studio | P2 | #251, #252 |
| 5 | #254 Player: Controls page, rebinding storage, glyphs and accessibility options | runtime | P2 | #250, #252 |

Phase 1 finishes Parkour on keyboard and gamepad. Phase 3 finishes it in the touch emulator, which completes the issue's
done-when. Template migrations (section 12) follow as part of each template issue (#84 to #87, #89, #91) once Phase 2 is merged; they are
not new issues here.

## 16. Out of scope

Motion and gyro input, voice, rumble, switch access, split-screen multi-pad, key chords as a **creator-visible** default,
pinch zoom, on-screen keyboard (text boxes use the OS), the M2 network format for `Move/Jump/Run` (networking spec), VR.

## 17. Owner decisions and unverified items

Decisions (Assistant's call): Roblox class names with Metallic additions; scene wins entirely over the built-in default;
largest-magnitude combine; floating stick by default; touch buttons activate on down; Menu rebindable with one binding kept;
physical key positions for bindings; glyph set names "Letters" and "Shapes".

Unverified:

- What Roblox's `InputContext.Sink`, `Priority` default, `ResponseCurve`, composite fields and mouse `KeyCode`s do; the names
  come from its pages, the behaviour is ours.
- `Enum.ContextActionPriority` numbers; which `UserInputType`s `BindAction` accepts.
- Apple's 44 pt minimum (page returned no text).
- winit: `Window::safe_area`, scale factor meaning on Android and iOS, layout-aware key labels; gilrs vendor id accessor.
- Whether the shape face-button symbols can ship (legal question for the owner), and the SVG rasteriser crate and its licence.
- Floating stick as default and the exact stick numbers (start values from a developer blog, not a study).
- One-handed preset key choices; the Survival Nights action names.
- Phone and tablet preset insets are typical, not measurements.

Sources not cited inline are the repository files named at the top.
