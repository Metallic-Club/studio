# #40 Animation system: `metallic_anim` + editor widgets

Status: **implemented** and **tested** (unit, integration and headless UI tests); not yet accepted.

## Goal
A pure-Rust animation crate (keyframes, clips, skeletons, blending, state machine, tweens, IK, root motion, text
format) and egui editor widgets (dope sheet, curve editor, transport, previews) that Studio can embed later.

## Done
- `crates/metallic_anim`: typed curves (f32, Vec2, Vec3, Quat, Color3, bool), interpolation (constant, linear,
  cubic Bezier with tangent modes, all 11 Roblox easing styles x 3 directions), clips with joint and property tracks
  and event markers, `Skeleton`/`Pose` for 3D joints and 2D bones, `Animator` (priority layers, weights, speed,
  crossfade, additive, per-joint masks, marker/loop/end events), `StateMachine` (+ 1D blend), `Tween`/`TweenInfo`,
  two-bone IK and look-at, root-motion extraction, RON clip/skeleton/state-machine format with a version field.
- `crates/metallic_anim_editor`: `DopeSheet`, `CurveEditor`, `TransportBar`, `KeyInspector`, `preview::rig_2d`,
  `preview::figure_3d`, `DemoApp`; theme tokens from the Studio UI design (PR #127, section 5.13).
- Demo: `cargo run -p metallic_anim_editor --example anim_editor_demo`.
- Screenshots (headless, egui_kittest + wgpu, looked at and iterated):
  `docs/agents/40-anim/editor-dope-sheet.png`, `editor-curves.png`, `editor-walk.png`, `editor-recording.png`.
  Regenerate: `cargo run -p metallic_anim_editor --example render_screenshots -- docs/agents/40-anim`.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_TARGET_DIR=$PWD/target
cargo test -p metallic_anim -p metallic_anim_editor -j 4
cargo bench -p metallic_anim --bench sample -j 4 -- --warm-up-time 1 --measurement-time 4
cargo run -p metallic_anim_editor --example anim_editor_demo
```

## Benchmark (release, Windows, `-j 4`, machine shared with other builds)
1,000 characters x 30 joints, each joint a 16-key Quat Bezier rotation track plus a root Vec3 track:
- `sample 1000 characters x 30 joints`: 2.72 ms (about 91 ns per joint track).
- `sample + model space`: 3.37 ms.
- `animator crossfade 1000 x 30` (two playing tracks per character, fading, layered): 9.60 ms.
Single-threaded. A 60 fps frame has 16.6 ms, so 1,000 crossfading characters fit on one core; Rayon over
characters and SIMD (ozz-style SoA) are the next steps if needed.

## Decisions (assistant's calls)
- **Editor is a crate, demo is an example.** Studio must embed the widgets (the UI design wants Timeline and Curve
  editor as shared widgets), so they live in a library; the window app is only `examples/anim_editor_demo.rs`.
- **glam 0.30.10** to match `metallic_render` (wgpu 30). **egui 0.37.0** (egui-wgpu 0.37 uses wgpu 30, and PR #127
  picks 0.37). eframe and egui_kittest are dev-dependencies only.
- **No `default_fonts`.** Its font crate licence fails `deny.toml`; `theme::install_system_fonts` loads installed
  fonts at runtime (IBM Plex if installed, else Segoe UI / Consolas, platform fonts elsewhere). Nothing bundled.
  When the owner approves OFL fonts (PR #127 question 13), swap in bundled Plex.
- **Deps stay in the crate `Cargo.toml`s**, not root `[workspace.dependencies]`: the task said to touch root Cargo
  files only to add crates. Move them when Studio starts sharing egui.
- **Joint tracks are offsets from rest** (like Roblox Poses): translation added, rotation `rest * track`, scale
  multiplied. An f32 rotation track turns about +Z (2D bones; frontal plane in 3D).
- **Tween delay happens once**, before the first cycle (Roblox docs say "delay before the tween starts"); repeats
  and reverses follow without delay.
- **Triggers expire at the end of each state-machine update** (Rive style), so an unused trigger cannot fire later.
- **Back and Elastic InOut** are built symmetrically from the In curve (`InOut(t) = In(2t)/2` then mirrored), the
  same rule as every other style. easings.net scales the overshoot constant for Back InOut (c2 = c1 * 1.525), so
  mid-curve values differ slightly. Reference table in `tests/easing.rs` uses our rule; Roblox parity needs checking in Studio.
- **Moved keys win collisions** when dragged onto another key on the same track.
- **Record keys (K) is explicit and off by default**; while on, the previews get an `error` outline. Key selected is
  I. Space plays. (PR #127 section 5.11, after Rive's auto-key traps.)
- **Timeline counts frames** (ruler labels every 5/10/20 frames, frame counter "22 / 60"), as in PR #127.

## Research notes (brief)
- Roblox: `KeyframeSequence` -> `Keyframe` (time) -> `Pose` (CFrame offset per joint, `EasingStyle`/`Direction` per
  pose, easing applies to the segment *after* the pose); `Animator:LoadAnimation` -> `AnimationTrack` with `Priority`
  (Core, Idle, Movement, Action, Action2-4), `Play(fadeTime, weight, speed)`, `AdjustWeight`, `AdjustSpeed`,
  `GetMarkerReachedSignal`, `Looped`. `TweenInfo(time, easingStyle, easingDirection, repeatCount, reverses,
  delayTime)`, repeatCount < 0 loops forever. Animation Editor: one track per part, 30 fps, right-click easing.
  https://create.roblox.com/docs/reference/engine/classes/AnimationTrack ,
  https://create.roblox.com/docs/reference/engine/datatypes/TweenInfo ,
  https://create.roblox.com/docs/reference/engine/enums/EasingStyle , https://create.roblox.com/docs/animation/editor
- glTF 2.0 animation: samplers with STEP / LINEAR / CUBICSPLINE (in-tangent, value, out-tangent), channels target
  node translation/rotation/scale/weights; rotations are unit quaternions, slerp for LINEAR. Our Bezier keys can be
  exported as CUBICSPLINE later. https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#animations
- Spine / DragonBones: 2D bones with per-timeline curves (Bezier per segment), events, IK constraints, mixing between
  animations with a mix duration. Matches our 2D bone builder + per-key interpolation + crossfade.
  http://esotericsoftware.com/spine-api-reference
- ozz-animation: runtime separates sampling (SoA, local space) from blending and local-to-model; samplers keep a cache
  per track. ACL compresses clips with error bounds per joint. We sample into a reusable `Pose` (no allocation per
  frame) and keep the layout simple; compression is a later step. https://guillaumeblanc.github.io/ozz-animation/ ,
  https://github.com/nfrechette/acl
- Rive: state machines with inputs (boolean, number, trigger), transitions with conditions, exit time and duration;
  layers; separate Design and Animate modes because users fell into auto-key traps.
  https://rive.app/docs/editor/state-machine/state-machine , https://rive.app/docs/editor/fundamentals/design-vs-animate-mode
- Unity Mecanim / Godot AnimationTree: state machines + blend trees (1D/2D), Any State transitions, avatar masks,
  additive layers. We took the simplest useful subset: states, any-state transitions, 1D blend, masks per Animator
  track. https://docs.godotengine.org/en/stable/classes/class_animationtree.html
- Curve editors (Blender Graph Editor, Unity Curves): auto-clamped tangents avoid overshoot at extremes; beginners
  struggle with handle modes, Euler flips and not knowing which key is selected. Hence: auto-clamped default,
  computed handles become Aligned when grabbed, key glyph shape shows interpolation, easing gallery with live plots.
  Easing reference values: https://easings.net/
- Common beginner problems (forum threads and the PR #127 survey): keys created silently while scrubbing (auto-key),
  animations that "don't play" because a higher-priority track overrides them, and confusion about loop vs once. Hence explicit record, a
  visible priority on Animator tracks, and the loop mode in the Clip inspector.

## Next
- Bind `Animator` and `Tween` to the World in `metallic_runtime` (Motor6D-like joints, Instance properties).
- Move `DopeSheet`/`CurveEditor` into `metallic_studio::widgets`, wrap `EditorState` edits in `Command`s (undo).
- Onion skin, rotate gizmo on the selected part, Copy pose / Paste mirrored (PR #127 Animate mockup).
- Show rotation curves in degrees in the curve editor (now radians for f32 rotation tracks).
- glTF import/export of clips; clip compression if memory needs it.

## Open questions
- Roblox parity for Back/Elastic InOut constants (check against Roblox Studio TweenService output).
- Bundled IBM Plex fonts depend on the owner's OFL-1.1 decision (PR #127).
