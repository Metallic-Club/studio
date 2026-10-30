# #47: invalid property-signal ids must not panic `World::destroy`

## Goal
`World::property_changed` stores any `InstanceId`, even a forged out-of-range one; `World::destroy` then indexed
`self.slots[k.index]` while pruning `prop_signals` and panicked when destroying an unrelated valid instance.

## Done
- Decision: public API unchanged. `destroy` prunes `prop_signals` with `slots.get(..)` and keeps a signal only when its
  slot exists, the generation matches and the node is live. Out-of-range and stale keys are dropped safely.
- `property_changed` docs say ids are not validated; an invalid id's signal never fires and the next `destroy` drops it.
- Regression tests in `crates/metallic_core/tests/tree.rs`: `destroy_ignores_out_of_range_signal_id` (panics on old
  code) and `destroy_ignores_stale_generation_signal_id` (guard only: slots never shrink, so a stale in-range id did not
  panic before). Both check that an unrelated surviving instance's signal still fires.

## Next
- Push the branch and open the PR (`Closes #47`).

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
```

## Open questions
- None. Rejecting invalid ids in `property_changed` would change its signature; not done.
