# #356 Local play lifecycle foundation

## Goal

Implement the nonvisual real-Game lifecycle child of #11. Task #10 is closed. Task #11 remains open for mode/action
registration, viewport/input/Explorer integration and manual Play/Stop acceptance.

## Source and ownership

- Branch `studio/356-local-play-lifecycle`, based on main `c914767167406da06292398769124ba1240b0ff0`.
- Own `play/local.rs`, the small declaration/re-export in `play.rs`, `tests/local_play.rs`, and these API docs.
- No dependency on open PR #309's Simulation facade or #275's clone-subtree fix. The Game constructor/frame/stop
  contract also survives the reviewed Simulation facade.
- Guarded build lane 122 was released by root after #42. The guard changes ownership to this worktree and cleans
  workspace packages only; historical #42 source and external logs stay unchanged.

## API and tests

`LocalPlaySession::start(name, snapshot, output)` copies the snapshot and starts a fresh Game. `frame` accepts an
already evaluated FrameInput; `inspect_world` only lends a shared World for a closure. Stop takes and drops the
private Game guard, whose Drop calls Game::stop. The guard exists before fallible Game::start, including its error
and unwind paths. This does not recover panics. The existing PlaySession adapter advances neutral input once per
caller update. Callers choose either that update or explicit frame per frame, never both.

Tests use real Luau, Game ticks and collapsed Output repeat counts: edit data/history/cursor/signal isolation,
parked-history removal through clean_world, pending delays/waits/handlers across Stop/Drop and fresh sessions,
one player and handler per session, real partial-start failure, combined output and trait cleanup. A private
world Weak reference additionally checks that stopped/dropped sessions release VM/world ownership.

## Limits

- Snapshot copying is O(world). EditorState::start_play currently creates clean_world first and lends it through
  PlayStart, so a future adapter through that seam copies twice. No mode is registered here.
- Runtime output has one sink for both VMs. Every line uses the combined Local Play tag; error text is preserved.
- Startup can still encounter independent #92/#275 recursive clone_subtree behavior when copying StarterGui.
- No rendering, raw input wiring, UI design, networking, persistence or manual/product acceptance claim.

## Validation

Tests were written before the implementation. The initial compile-only check failed with E0432 for the missing
LocalPlaySession import, as expected. Implementation and private ownership regressions are now written; no new
lifecycle test has executed yet. Actual Lua execution is held until #353's timing window is released.

Required gates before PR: full serial workspace tests, strict workspace/all-target Clippy, licenses, format and
task-range whitespace. Use `tools/cargo_lane.py` with explicit lane 122, `--locked` and `-j 4`.
