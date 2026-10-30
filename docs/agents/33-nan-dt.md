# #33: NaN/infinite dt must not freeze physics

## Goal
`PhysicsWorld::step(NaN)` poisoned the accumulator (NaN forever, every later step a no-op); `step(inf)` simulated a full `MAX_STEP`.
Treat any non-finite `dt` as 0, keep negative -> 0 and large -> `MAX_STEP`.

## Done
- `crates/metallic_physics/src/lib.rs`: `step` maps non-finite `dt` to 0 before clamping; resets the accumulator to 0 if it is ever non-finite (defensive). Doc comment states the behaviour.
- Tests in `crates/metallic_physics/tests/physics.rs`: `nan_dt_does_not_freeze`, `infinite_dt_is_ignored`.
- test, clippy, deny pass (see commands).

## Next
- Open the PR (`Closes #33`) after review by the lead.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
```

## Open questions
- The issue suggests the same guard in `Game::frame` (#8) and `ScriptHost::step`; not done here (out of scope, separate issue/PR).
