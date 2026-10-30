# #7 Luau API and RemoteEvent (branch `studio/7-luau-api`)

## Goal
Roblox-named Luau API and RemoteEvent over the Task 6 VM. See `docs/luau-api.md`.

## Done
- `metallic_script/src/api/` (mod, instance, datatypes, services, signals, remote, errors) and `host.rs` (`ScriptHost`).
- Core: class `PlayerGui`, `InstanceId::to_bits`. VM: `ScriptVm::run_script_with` (per-script globals such as `script`).
- Task 6 guard now strips the Rust traceback from userdata errors.
- Tests: `tests/api.rs`, `tests/remote.rs`, `tests/common/mod.rs`.

## Next
- Task 8 feeds `touched`, `humanoid_died`, `character_added`, clones StarterGui into `PlayerGui`, calls `step(now, dt)`.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_TARGET_DIR=<own dir>
cargo test --workspace -j 6; cargo clippy --workspace --all-targets -j 6 -- -D warnings; cargo fmt --check
```

## Open questions
- `PlayerAdded` fires on the first step (Player1 exists from `start`, but `GetPlayers` is empty until then, as on a Roblox server).
