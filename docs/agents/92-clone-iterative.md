# Issue #92: iterative `World::clone_subtree`

## Goal
`clone_subtree` recursed once per tree level and overflowed the stack (process abort) on deep chains.

## Done
- `crates/metallic_core/src/instance.rs`: removed the recursive `clone_rec`; `clone_subtree` walks the subtree with an
  explicit stack (pre-order, children pushed in reverse), so allocation order, child order, ref remapping, the
  non-built-in flag, no copied signals and the change journal entries are as before.
- `crates/metallic_core/tests/tree.rs`: `clone_subtree_handles_deep_chain` (100,000-deep Folder chain on a normal
  test thread; overflowed with `0xc00000fd` before the fix) and `clone_subtree_keeps_child_order_and_external_refs`.

## Next
Review and merge. Nothing else planned.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_core -j 4 --test tree
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all -- --check
```

## Open questions
None.
