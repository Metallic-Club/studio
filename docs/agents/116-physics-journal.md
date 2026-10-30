# Physics journal synchronization: issue 116

Status: implemented and tested; ready for review.

## Goal and scope

Consume the bounded instance change journal in `PhysicsWorld::sync_from` instead of collecting and reading every part each frame. Keep the existing transform polling and simulation behavior in `step`; teleport policy and ray-normal behavior in issue 34 are separate work.

## Acceptance

- An unchanged world examines no parts, traverses no scene tree, and does not rerun collision detection.
- Sparse edits examine only journaled parts and affected character-model subtrees. Repeated edits to one instance are deduplicated.
- Creation, destruction, generational slot reuse, subtree reparenting, and Workspace membership preserve body/query behavior.
- Creating, destroying, or reparenting a Humanoid refreshes sibling/descendant character collision membership, including nested models and multiple Humanoids.
- First synchronization, journal overflow, or a different world performs a full reconciliation. Same-world overflow keeps matching bodies and momentum; replacing the world resets physics state, contacts, and accumulated time.
- Independent readers have independent cursors. A small last-sync stats snapshot records deterministic operation counts for tests/profiling.

## Design notes

The journal identifies instances rather than properties. Part signatures still decide whether a body/collider must be rebuilt. Character membership additionally depends on a Model having a direct Humanoid child; the journal does not report the unchanged sibling parts when that child changes. Cache Humanoid parents and character-model membership to expand only affected model subtrees.

`Changes::Overflow` combines dropped history and a foreign-world cursor. A minimal `ChangeCursor::same_world` helper distinguishes those cases without exposing or persisting world identity.

## Work log

- Read repository agreement, issue 116, current physics/core/renderer source, issue 34 and PR 53, and the performance research.
- Claimed issue 116 and refined its acceptance in a claim comment.
- Added regression tests first, including a deterministic 5,000-part static/sparse workload.
- Implemented incremental synchronization, cached character membership, the identity helper, and last-sync stats.
- Tests-first run failed on the missing stats API as expected. After implementation, physics unit tests passed
  (12 tests, including 10 journal regressions); core journal tests passed (4 tests).
- The 5,000-part test records 5,000 examined parts on first sync, zero on an unchanged sync, and one examined/rebuilt
  part after two signature edits to one part. This is an operation-count check, not an elapsed-time benchmark.
- Rebased onto `origin/main` at `532c4f7` before final gates.
- Initial complete gates used an isolated lane target directory after the coordinator found artifact collisions between
  concurrently built worktrees sharing one target directory. Earlier shared-cache runs are not final evidence.
- `cargo test --workspace -j 4`: passed, 201 tests across all suites, including all 18 character tests, 16 existing
  physics tests, 12 physics unit tests, and GPU headless tests; zero failures.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed, no warnings.
- `cargo deny check licenses`: passed (`licenses ok`); existing unused MPL-2.0 allowance warning only.
- `cargo fmt --all -- --check`: passed. No visual change or windowed manual test.

## Runtime integration refresh

Merged main at `937dcd8` normally into the published branch, keeping the new skin/shirt/pants avatar exports beside
the journal test module. No force push or pull-request merge was used.

Sequential reuse of a lane target also exposed a stale core artifact: physics could not find `same_world` despite
the source containing it. Cleaned only the seven Metallic workspace packages from the verified own lane directory,
preserving third-party dependency artifacts. Rebuilt all seven local crates; compiler output named this issue's
worktree as their source. Do not rely on sequential cross-worktree cache reuse without that local-package clean.

Final validation after the runtime merge and forced local-package rebuild:

- `cargo test --workspace -j 4 -- --test-threads=1`: all 223 tests passed, zero failures or skips. Includes the input-only
  bot finishing the full Parkour course, runtime determinism, player smoke, all 19 character tests, 16 existing physics
  tests, 10 journal regressions, cursor identity, Luau, GPU headless rendering, and Studio tests.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed, no warnings.
- `cargo deny check licenses`: `licenses ok`; existing unused MPL-2.0 allowance warning only.
- `cargo fmt --all -- --check`: passed.

## Commands

Use a command-scoped `CARGO_TARGET_DIR` pointing at the isolated `lane116` directory beneath the Metallic cargo
cache. Run Cargo with `-j 4` and use the installed cargo executable explicitly. Do not reuse one target directory
for concurrent builds of different worktrees of this package.

## Limits

`step` still polls transforms and performs the original fixed 1/240 s outer simulation substeps. This issue changes
wrapper synchronization only; no wall-clock speedup or frame-rate claim is made. The bounded journal reports IDs,
not property names, so an unrelated property change on one BasePart still reads that part's physics properties,
but retains the existing body and momentum when its signature matches.
