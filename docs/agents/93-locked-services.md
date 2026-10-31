# #93 Locked built-in services, non-creatable classes

## Goal
Built-in services cannot be reparented, destroyed or cloned; `World::create` refuses services and abstract classes
(so `root("Workspace")` can never disappear and physics never silently simulates nothing).

## Done
- `CoreError::Locked(class)` and `CoreError::NotCreatable(class)` (`lib.rs`).
- `ClassDef::creatable` (`classes.rs`): false for `Instance`, `BasePart` and every service (`Workspace`, `Players`,
  `ReplicatedStorage`, `ServerScriptService`, `StarterPlayer`, `StarterPlayerScripts`, `StarterGui`, `Lighting`,
  `RunService`).
- `instance.rs`: `create` checks `creatable`; `set_parent` and `clone_subtree` return `Locked` for built-ins;
  `destroy` is a silent no-op for built-ins (signature unchanged). `new_with_services` uses `alloc` directly and scene
  load matches built-ins instead of creating them, so neither goes through the check. A scene file with a plain
  (non-builtin) service or abstract class now fails to load with `NotCreatable`.
- Tests: `tests/tree.rs` (`builtin_services_cannot_be_reparented`, `destroying_a_builtin_service_is_a_no_op`,
  `builtin_services_cannot_be_cloned`, `services_and_abstract_classes_are_not_creatable`,
  `concrete_classes_are_still_creatable`, `deep_clone_keeps_builtins_locked`), `tests/scene_file.rs`
  (`load_refuses_non_builtin_services_and_abstract_classes`). ARCHITECTURE.md updated.

## Next
Open the PR (`Closes #93`).

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_core -j 4
cargo test --workspace -j 4 ; cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses ; cargo fmt --all -- --check
```

## Open questions
- `PlayerGui` stays creatable: `ScriptHost` creates one per player (Roblox makes it not creatable too).
- Luau `Instance.new("BasePart")`/`("Instance")` now surfaces the core message ("class `X` cannot be created"), not
  Roblox's "Unable to create an Instance of type"; `metallic_script` maps only `UnknownClass` to that text.
- `Player` stays creatable (Roblox makes it not creatable via `Instance.new`); the runtime and tests create it directly.
- Same-parent `set_parent` early return is #97, not done here.
