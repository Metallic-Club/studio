# #460 class registry (step 2 of the class-system migration)

Branch `studio/460-class-registry`. Design: `docs/design/class-system.md` (sections 3, 4, 11, 14 row #460).

## Goal
Replace `metallic_core/src/classes.rs` with one file per class under `src/classes/<group>/`, discovered by `build.rs`,
resolved once into flat per-class tables. `ClassId` in World nodes, abstract classes of section 3.4, `registry_hash` v5,
abstract and service refusal in `Instance.new`. No script behaviour change beyond that refusal.

## Done
- `build.rs`, `classes/{mod,desc,resolve,lists,prelude}.rs`, 51 class files (43 old, 8 new abstract).
- Thin wrappers kept: `class_def`, `find_property`, `is_subclass`, `all_properties`, `class_index`, `class_at`,
  `property_index`, `property_by_index`, `default_value(_ref)`, `registry_classes`; `PropDef`/`ClassDef` are type aliases.
- Tests: `crates/metallic_core/tests/class_registry.rs`, `class_lookup_allocations.rs`,
  `crates/metallic_script/tests/class_flags.rs`, a new unit test in `replication.rs`. Fixtures recorded from the old code:
  `crates/metallic_core/tests/fixtures/{is_subclass,properties}_before_460.txt`.

## Next
Nothing in this issue once CI is green on the PR. #461 adds `ScriptValue`, `MethodCall`, `MethodImpl::Native` (fingerprint
byte 0 is reserved for it) and the `Host` completeness test; `PVInstance.GetPivot`/`PivotTo` are declared `Host` already.

## Decisions where the design left room
- `Member` is a struct (`kind`, `owner`, `context`, `hidden`) with a `MemberKind` enum instead of the design's four-variant
  enum: `context` and `hidden` apply to every kind.
- Name table: a third candidate won; see Perf. It also replaces the design's per-registry hash map for class names.
- `defaults` and `hide` on a property leak one changed `PropDesc` copy per entry, once per process, so `find_property`,
  `all_properties`, Studio and the Luau binding honour them with no change.
- `Instance` and `BasePart` are `ABSTRACT` (#93 names them). `GuiButton` stays `CREATABLE` because the design does not say
  otherwise and today `Instance.new("GuiButton")` works. `Player`, `PlayerGui`, `PlayerScripts`, `StarterPlayerScripts` stay
  `CREATABLE` for the same reason.
- `ServerStorage` is `SERVICE`, so `Instance.new("ServerStorage")`, which worked by omission, is now refused.
  `GetService("ServerStorage")` is unchanged until #466.
- `World::create` refuses `ABSTRACT` (new `CoreError::AbstractClass`) but not `SERVICE`: engine code and hand-built worlds
  create services.
- Fields left for their steps: `ClassDesc::enums` and `EnumDesc` (#469), `luau` (#470), `PropDesc::script_write` and computed
  access (#464), `editor` (#473), `context_message` (#468).
- `registry()` on a set that does not resolve is an empty registry plus `registry_error()`, so lookups fail closed with no
  panic; `registry_hash` returns the `SchemaError`.
- `cargo fmt` does not reach discovered modules, so `ci.yml` got one step running `rustfmt --check` on the folder.
- `metallic_studio/src/icons.rs` maps the 8 new abstract classes to existing icons: `every_registry_class_has_its_own_icon`
  (added on main by #476) walks every registry class and must stay unedited.

## Perf
Windows 11, a working desktop with other sessions active, not a controlled machine. Base `58224e0`, head this branch, both
built `--release` in this worktree and run interleaved. The CI gate counts Callgrind instructions, which needs Linux and
Valgrind: **not run here**, so the first real gate result comes from the PR. Python is not installed, so `perf_ci.exe` was
run directly, without `tools/perf/measure.py`.

`perf_ci` workloads, `elapsed_ns`, 31 samples each:

| Workload | Base min / median | Head min / median | Median change |
|---|---|---|---|
| `reads` | 2.598 / 2.677 ms | 2.358 / 2.478 ms | -7.4% |
| `writes` | 1.225 / 1.265 ms | 1.032 / 1.070 ms | -15.4% |
| `clone` | 1.414 / 1.580 ms | 1.311 / 1.516 ms | -4.1% |
| `create` | 0.072 / 0.076 ms | 0.077 / 0.082 ms | +7.9% |
| `destroy` | 0.468 / 0.494 ms | 0.429 / 0.504 ms | +2.0% |

`create` is the one to watch at the gate. It is not the registry path: replacing the name lookup in `World::create` with a
cached id left it at 0.084 ms, while padding `Node` back by 16 bytes gave 0.076 ms (base 0.077 ms in that run). Likely the
allocator's behaviour when `Vec<Slot>` grows past 1,024 entries at the smaller slot size; that cause is inferred, not traced.
Criterion's `tree/create_1000_parts` went from 714.2 to 640.3 us (-9.7%).

Criterion `world`, base then head (same machine, one after the other; `deep_clone_5000` varied from 1.73 to 2.92 ms across
head runs, so read it as noise):

| Bench | Base | Head | Change |
|---|---|---|---|
| `property/get_set_value` | 15.16 ns | 15.62 ns | +2.9% |
| `property/get_default_value` | 32.72 ns | 28.24 ns | -13.4% |
| `property/get_name` | 57.41 ns | 57.59 ns | +0.6% |
| `property/get_ref_set_value` | 13.52 ns | 13.44 ns | -1.9% |
| `property/get_ref_default_value` | 51.32 ns | 25.73 ns | -50.1% |
| `property/get_ref_name` | 46.66 ns | 21.19 ns | -54.7% |
| `property/set_position` | 103.11 ns | 96.07 ns | -6.3% |
| `property/is_a_basepart` | 10.13 ns | 5.74 ns | -43.3% |
| `frame_read/seven_props_per_part/5000` | 1.392 ms | 1.197 ms | -12.6% |
| `frame_read/borrowed_seven_props_per_part/5000` | 1.254 ms | 0.724 ms | -42.1% |
| `tree/find_first_child_miss_1000` | 44.19 us | 21.40 us | -51.6% |
| `tree/full_name` | 212.05 ns | 135.20 ns | -36.1% |
| `tree/destroy_1000_siblings_one_by_one` | 351.9 us | 361.5 us | +1.4% |
| `tree/deep_clone_5000` | 2.682 ms | 2.923 ms | +9.0% |
| `scene/save_5000` | 34.63 ms | 34.22 ms | no significant change |
| `scene/load_5000` | 32.33 ms | 34.58 ms | +5.4% (p = 0.03, wide interval) |

Name table candidates (design section 4.2 names the first two), `perf_ci reads` median in one interleaved run each:

| Candidate | Base | Candidate |
|---|---|---|
| Hash map with a fast non-cryptographic hasher | 2.752 ms | 3.100 ms (+12.6%) |
| Linear scan of the flattened list | 2.752 ms | 2.578 ms (-6.3%) |
| Entries grouped by name length (kept) | 2.700 ms | 2.508 ms (-7.1%) |

Lookup alone, 5M iterations: the hash map took 13 to 18 ns on most names regardless of class; the scan took 3 to 10 ns on
`Part` (14 members) and up to 24 ns on 22-23-member classes. Grouping by length does not get slower as a class grows, needs no
dependency and also replaces the class-name hash map, so no hasher is in the crate.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_core -j 4
cargo test -p metallic_script -j 4
cargo run -p metallic_core --example registry_hash    # new golden value after a class change
find crates/metallic_core/src/classes -mindepth 2 -name '*.rs' -print0 | xargs -0 rustfmt --edition 2024 --check
```

## Open questions
None blocking. The CI instruction gate has not been seen on this branch: it needs Linux and Valgrind.

## Last check results (head of this branch on base 58224e0)
`cargo test --workspace -j 4 --no-fail-fast`: 2299 passed, 1 failed, 9 ignored; the failure is the known #436
(`metallic_render --test shading gpu_timing_measures_a_frame_where_supported`). `cargo clippy --workspace --all-targets -j 4 --
-D warnings`: clean. `cargo deny check licenses`: licenses ok. `cargo fmt --all -- --check` and the class-file rustfmt check: clean.
