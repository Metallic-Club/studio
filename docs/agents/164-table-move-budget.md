# #164 table.move ignores the time budget (branch `studio/164-table-move-budget`)

## Goal
`table.move` runs its element loop in C, where the per-resume time budget's interrupt cannot fire:
`table.move({}, 1, 2^31-3, 2, {})` blocked the host for 72 s. Make it fail or finish within 0.5 s.

## Done
- `crates/metallic_script/src/vm.rs`:
  - `init_luau_flags()` (a `std::sync::Once`, called before the first `Lua::new_with`) enables the
    Luau dynamic flag `LuauTableMoveTimeoutFix` via `Lua::set_fflag` (exists in luau0-src 0.22.0+luau740,
    `VM/src/ltablib.cpp`; off by default). With it, a sparse move costs the size of the tables, not of the
    range. Best effort: a failed set is ignored, because of the next point.
  - `TABLE_LIMITS` (Lua, loaded in `setup` next to `STRING_LIMITS`, before the library table is frozen)
    wraps `table.move`: when both range ends read as numbers (`tonumber`, so numeric strings too), it
    truncates them toward zero like `luaL_checkinteger` and raises
    `table.move: too many elements to move (limit 16777216)` (level 2) if the count is above 2^24 or
    NaN (NaN, ±inf and 1e300-style ends are rejected). Everything else goes to the original unchanged.
- Tests in `crates/metallic_script/tests/limits.rs`: `table_move_huge_count_is_rejected_quickly`
  (8 huge/odd ranges each fail in < 0.5 s with a `T:1:` error; an allowed 2^24 sparse move is quick),
  `table_move_keeps_normal_semantics` (overlap both ways, other destination, numeric strings, empty
  range, arg errors, read-only destination).
- Red/green: without the fix the first test hung (killed after 30 s; an uncapped run took 76 s in debug).
  Flag alone: the repro returns in ~50 µs (no error); wrapper alone also rejects it.

## Next
- Review and PR (not opened by this session).

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_script -j 4 --test limits table_move
cargo test --workspace -j 4; cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses; cargo fmt --all -- --check
```

## Open questions
- `table.sort` without a comparator sorts in C with no interrupt: 8,000,000 random numbers took 1.14 s
  (debug test build, Luau at opt-level 2). A script can build such a table across several resumes
  (`task.wait`) and then sort it once. With a comparator, the Lua callback is interruptible. Fixing it
  means capping `#t` (a design call on the limit) or a C-level change; not done here.
- Other `table` functions checked (`insert`, `remove`, `unpack`, `concat`, `find`, `clear`, `clone`,
  `create`, `maxn`): their loops are bounded by the table's real size (256 MB VM cap, at most ~16M
  slots) or by the stack (`unpack`), measured at tens of ms for 8M elements. No change.
- Errors raised by the original `table.move` (bad argument types) now carry an inner `[engine]:14:`
  after the script position, the same as the existing `string.rep` wrapper.
- `Lua::set_fflag` is process-global and mlua asks that it run before any VM exists; the `Once` in
  `ScriptVm::with_output` (the only `Lua` constructor in the crate) does that.
