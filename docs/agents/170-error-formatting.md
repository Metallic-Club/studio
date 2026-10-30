# Issue 170: structured Luau callback errors

Goal: engine callbacks caught by `pcall` return positioned strings, and displayed errors retain user text without
Rust callback traceback metadata. Worktree `170-error-formatting`, branch `studio/170-error-formatting`.

## Implemented

- Recognize `mlua::Value::Error` instead of guessing from `tostring`, prefixes or newline markers.
- Strip only structured `CallbackError` metadata from a display copy, including causes inside `BadArgument` and
  `WithContext`. Preserve argument labels, context, original error graphs and every byte of RuntimeError text.
- Install trusted binding wrappers during VM setup. Cover task spawn/delay, collector, print/warn, pattern
  functions and returned gmatch iterators without changing `pattern.rs` or the execution budgets.
- Keep native global pcall/xpcall unchanged. A global Lua replacement was rejected during prototyping because
  error levels and missing-argument attribution exposed its extra frame. No unsafe FFI or Luau fork was used.
- Document the boundary: raw callbacks installed by an embedding host retain mlua error userdata under pcall.
  They still get structured display formatting when uncaught. No claim is made that arbitrary host callbacks
  or traceback text already embedded inside a RuntimeError string are normalized.

## Research and regression evidence

Read the pinned mlua 0.12.2 `src/error.rs`, `src/value.rs` and weak ValueRef implementation from Cargo's registry,
and Luau 0.740 `VM/src/lbaselib.cpp`. CallbackError separates traceback metadata from its cause; RuntimeError
does not. Luau's native protected calls preserve arbitrary error objects and yielding. The public
[Luau library reference](https://luau.org/library/#global-functions) documents those contracts. No third-party
implementation was copied.

Before changing production code, two formatter tests failed and the integration baseline showed four product
failures: caught callback userdata, inner-line normalization, literal host callback truncation, and multiline
property-name truncation. A fifth failure was a fixture's abbreviated timeout expectation, corrected to the
existing full timeout message; it is not counted as a product regression. Uncaught collector and malformed
pattern messages were already clean on current main, reflecting an earlier partial fix.

Focused final prototype: 3 unit tests and 16 integration tests pass, including native pcall frame snapshots,
callable tables, missing arguments, nil/multiple results, xpcall handler behavior, user error identity,
multiline text, inner-function lines, delayed output, yields, sticky timeout recovery, original large-pattern
repro, returned-iterator errors, GC and VM release. The 12 MB pattern repro permits either exact complexity
failure or exact timeout failure because wall-clock load can decide which existing limit fires first.

The 1,000 successful-pcall smoke passed the unchanged 100 ms budget: 691.2 microseconds in the earlier baseline,
1.2416 milliseconds after the narrow prototype. These single runs had uncontrolled host load and are not a
performance comparison. The smoke catches gross overhead; there is no speedup or latency-target claim.

## Validation and next

Owned build lane `cargo-target-114`; the reviewed `tools/cargo_lane.py` from `209-build-lane-guard` forced local
workspace-package cleanup on entry, preserving third-party dependency cache. Compilation paths identify this
worktree. All Cargo builds use `-j 4`.

- Focused formatter/compatibility units: 3 passed.
- Focused error-formatting integration binary: 16 passed, serial, 0.24 seconds.
- `cargo fmt --all` and `git diff --check`: passed.
- `cargo test --workspace -j 4 -- --test-threads=1` through the lane guard: 231 passed, zero ignored.
  The final forced-clean workspace prebuild took 58.35 seconds and compiled all seven local packages from this
  worktree; 118.9 MiB of local package artifacts were removed while dependency cache was preserved.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings` through the lane guard: passed, 8.30 seconds.
  The first attempt reported `arc_with_non_send_sync` on mlua's required `Arc<Error>` cause fields. Narrow,
  reasoned `#[expect]` annotations in the formatter and its construction test resolved that API-shape lint;
  no runtime code or budget was changed after the full test pass.
- `cargo deny check licenses`: passed (`licenses ok`; existing unused MPL-2.0 allowance warning).
- Final `cargo fmt --all -- --check` and `git diff --check`: passed.

Independent source review found no blocking findings. Rebased onto current main `12a6a6b` (avatar assets only
since the original base). All required local gates are complete. Next: publish the PR, inspect CI and address
review feedback; do not merge.
