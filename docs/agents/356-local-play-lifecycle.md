# #356 Local play lifecycle foundation

## Goal

Implement the nonvisual real-Game lifecycle child of #11. Task #10 is closed. Task #11 remains open for mode/action
registration, viewport/input/Explorer integration and manual Play/Stop acceptance.

## Source and ownership

- Branch `studio/356-local-play-lifecycle`, based on main `c914767167406da06292398769124ba1240b0ff0`.
- Before publication, normal-merged exact main `238c6fb8af7a05f031cb29930e59c8afe57cf0fa` at
  `2be73970a5670ed6a05ccc0a1e344d11a48240ac`. The merge was automatic; lifecycle/core/script/runtime source did
  not change. Upstream Studio/token follow-ups are inherited, with no visual acceptance claim from this task.
- Own `play/local.rs`, the small declaration/re-export in `play.rs`, `tests/local_play.rs`, and these API docs.
- No dependency on open PR #309's Simulation facade or #275's clone-subtree fix. The Game constructor/frame/stop
  contract also survives the reviewed Simulation facade. Independent review against #309 head
  `5b1960a42fcfa7b4224b4ec8c84253abd4c9ce3c` found source-level compatibility only; no combined execution or
  additional runtime clone is claimed.
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
LocalPlaySession import. First execution had seven passes and one fixture failure: the test incorrectly treated
StarterPlayerScripts as a root. Commit `21673fe` locates it under StarterPlayer; no production or budget change.
The corrected eight integration cases passed, followed by both private Weak/unwind cases. External logs retain
the original failed fixture result.

Historical checks at `21673fe` on main `c914767`: full serial workspace 1,580 passed, zero failed, seven upstream
ignored tests; strict workspace/all-target Clippy, licenses, format and task-range whitespace passed. The normal
workspace dependency graph was captured. This is separate from the current-base result below.

Final code `2be73970a5670ed6a05ccc0a1e344d11a48240ac`, exact main
`238c6fb8af7a05f031cb29930e59c8afe57cf0fa`:

- Guarded `cargo test --locked --workspace --no-run -j 4`: passed.
- Guarded `cargo test --locked --workspace -j 4 -- --test-threads=1`: 1,592 passed, zero failed, seven upstream
  ignored tests across 122 result groups. All eight new integration and both private ownership cases passed.
- Guarded `cargo clippy --locked --workspace --all-targets -j 4 -- -D warnings`: passed (14.79 seconds).
- `cargo deny check licenses`, `cargo fmt --all --check`, and task-range `git diff --check`: passed.
- Normal/build workspace graph and source/log hashes captured outside the repository in the task evidence.

The guard uses dedicated lane 122, dev profile and no target override. Actual tests ran only after the other
agents released their timing windows. Documentation-only publication updates do not change the tested crate
tree. Manual UI checks: not run; this change adds no UI or mode registration. Task #11 stays open.
