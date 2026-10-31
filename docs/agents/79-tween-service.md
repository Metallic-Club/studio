# #79 part 1: TweenService

Roblox-compatible `TweenService`, `TweenInfo` and `Tween` for Luau (`crates/metallic_script/src/api/tween.rs`).
Clickable UI (GuiButton, TextButton, ImageButton) follows in a second PR.

## Design

- Curves come from `metallic_anim::ease` (Back and Elastic InOut use Penner's own forms, as Roblox). The timeline is
  Roblox's: `RepeatCount + 1` cycles of one pass (two with `Reverses`, ending at the start), `DelayTime` before every
  pass, a negative count repeats forever. `Info::phase` computes it in f64 (`Delayed`/`Playing`/`Finished`).
- Memory: `Create` gives each `Tween` userdata a Lua buffer (its user value) as large as the Rust-side state, so the
  goals count toward the VM's 256 MB Lua limit and an unbounded `Create` loop fails with "not enough memory".
- Each VM owns a `Tweens` list. `ScriptHost::step` calls `advance_tweens(dt)` for each side after inputs and before
  `Heartbeat`: one world borrow, values lerped `a*(1-t) + b*t` (exact at both ends), exact goals written at the end.
  No wall clock, so a headless simulation (#309) is deterministic.
- `Completed` is a hub signal keyed `(RunService, EventKind::TweenCompleted(id))`, like the input services' carrier
  signals. Events queue while tweens advance or are cancelled and fire at the next step with the PlaybackState item.
  Connections are removed when the tween is destroyed or its Lua object is collected (`Hub::remove_key`).
- Override: `Play` cancels other playing tweens of the same instance with an overlapping target. A part's `CFrame`
  and a model's `WorldPivot` go through `metallic_core::transform`, so a `CFrame` tween overlaps `Position` and
  `Orientation` tweens.
- No per-step allocation: buffers are swapped and reused. The test `playing_tweens_do_not_allocate_per_step`
  counts allocations with 0 and 200 playing tweens. A CFrame-typed stored property (not a part's `CFrame` or a
  model's `WorldPivot`) boxes its value on each write, as `Value::CFrame` requires.

## Tests

`crates/metallic_script/tests/tween.rs`: easing values against reference formulas, TweenInfo defaults and errors,
step-clock progress and a single `Completed`, every tweenable type (bool snaps at the end), a part CFrame tween,
override and cancel, pause/resume/cancel/replay, repeat with reverse and delay, `Create` validation, determinism with
uneven steps, and the allocation check.
