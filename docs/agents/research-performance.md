# Research: performance and scale (branch `studio/research-performance`)

Goal: budgets, design changes, a benchmark suite and a perf-CI plan, grounded in measurements of the merged code.
Deliverable: `docs/research/2026-10-30-performance-and-scale.md`.

## Done
- Criterion benches for `metallic_core`: `crates/metallic_core/benches/world.rs` (`criterion` 0.8.2 as a workspace
  dev-dependency, default features off).
- Fix: `classes::find_property` walks the class chain without allocating; `World::set` skips the signal-key
  allocation when no property signal exists. Unit test `no_class_redeclares_an_inherited_property` guards the search
  order. Before/after numbers in section 2.1 of the research doc.
- Scratch measurements (not committed): Rapier step layouts, mlua interrupt/call/memory costs, core allocation counts.
  Programs lived in the session scratchpad; their code is described in the doc.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
CARGO_TARGET_DIR=$PWD/target-perf cargo bench -p metallic_core --bench world
# compare against a saved run
cargo bench -p metallic_core --bench world -- --save-baseline before
cargo bench -p metallic_core --bench world -- --baseline before
```

## Next
- Re-run the benches on an idle PC (tonight's runs had 88-99 % CPU load from other sessions).
- The owner or lead decides which proposed issues (section 6 of the doc) to create.
