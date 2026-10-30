# #31 Scene persistence integrity

Status: implementation and review follow-up tested; ready for review. Not accepted or released.
Branch: `studio/31-scene-persistence`.

## Goal

Preserve the last complete scene when a save fails and reject scene input that would silently discard intent.
Keep valid text scenes deterministic and references efficient as scene sizes grow.

## Done

- Encode and preflight before touching the destination; write to a sibling file created exclusively, sync it,
  close it, and rename over the destination. Normal write/rename errors remove the temporary file.
- Injected partial-write and rename-failure tests verify the previous destination survives and no sibling
  temporary file remains. A success test verifies existing-file replacement on Windows.
- Preorder traversal is iterative and builds a hash map for reference indices instead of searching every
  instance for every reference.
- Both save and load enforce the exported `MAX_SCENE_DEPTH = 32`. The RON recursion guard remains enabled,
  with enough parser frames for the documented limit. Saves reject moved built-in services before replacement.
- Loading rejects unknown fields, duplicate property names and unmatched/duplicate/misplaced built-in markers.
- Regressions cover dangling references, Unicode paths/names, 512 cross-references, supported boundary depth,
  excessive loading depth, and preserving an existing file when a too-deep or moved-built-in world is rejected.
- Updated public API documentation and architecture. No dependencies changed.

## Initial verification

- Before implementation, the new integrity suite had five failures: duplicate properties, invalid built-in
  markers, unknown fields, excessive save depth, and a saved 32-level tree that failed to load.
- `cargo test -p metallic_core -j 4`: passed 28 tests, including 12 new regression/unit tests.
- `cargo test --workspace -j 4`: passed 45 tests, including all existing physics tests.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed (existing unused-license-allowance warnings).
- `cargo fmt --all` and `git diff --check`: passed.
- Tested on Windows with Rust 1.99.1. No UI changes or screenshot requirement.

## PR #45 review follow-up

- Resumed the existing branch and merged main `ebb94f3` normally (no rewritten
  history); resolved the core exports conflict by preserving both the change-journal
  API and `MAX_SCENE_DEPTH`.
- Reproduced #55 with a failing unknown Vec3-member regression. Scene-local Serde
  remote representations now reject unknown members of Vec3, Color3 and UDim2;
  public value serialization outside scene files is unchanged.
- Lead approved the strict v1 saver policy for #58: every canonical builtin must
  exist exactly once at its original parent, including StarterPlayerScripts.
  Missing/destroyed/shadowed Workspace, missing/moved nested builtin and builtinless
  utility worlds are rejected before writing. `World::new_empty` stays available
  for in-memory trees. Legacy v1 load behavior still supplies omitted services.
  All actual save callers use `new_with_services`; updated the depth-only fixture.
- Addressed #57 with Windows-only retries for raw OS sharing/access/lock errors
  5, 32 and 33, sleeping 10/20/40 ms at most before returning the last error.
  A real deny-delete-sharing handle first blocks replacement, then releasing it
  permits the retry. A persistent handle through public `save_scene` preserves
  old bytes and removes the temporary sibling. No delete-then-rename fallback.
- Added public saver failure-cleanup coverage and a constructed depth-33 loader
  test that verifies the loader's own limit independently of the RON parser.
- Expanded save/load error docs and documented entry-replacement semantics:
  symlinks are replaced; other hard links retain old data; Windows ACLs/alternate
  streams are not copied. Unix mode bits are copied when metadata is available.
  File sync does not guarantee directory/power-loss durability.
- Finite-value review item #56 is covered separately by PR #211; no duplicate
  World::set validation added here. Integrated scene regression can follow once
  that branch is merged into main.
- Targeted checks: eight core unit tests (including real Windows sharing tests)
  and all thirteen integrity regressions passed after the value-error assertion
  was corrected to accept RON's `Unexpected field` wording.
- Verified the owned worker target and force-cleaned all seven local workspace
  packages before switching from the property-key branch; third-party cache kept.
- Final gates repeated after a second seven-package dev-profile clean. Initial
  workspace build failed before tests with a rustc allocation failure and a missing
  bytes rlib artifact. With other heavy builds paused, cleaned only bytes in the
  owned worker lane (2.6 MiB), then prebuilt the Studio binary and integration
  targets sequentially with `-j 4`. No common target/cache was deleted.
- `cargo test --workspace -j 4 -- --test-threads=1`: 232 passed, zero failed/ignored,
  including Windows share locks and the full Parkour route. Logs identify this
  scene-persistence worktree for local crate compilation.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed; existing unused allowance warnings only.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Added duplicate/misplaced nested-builtin inputs to the existing loader regression
  after the full run; reran the integrity target and strict Clippy for those fixtures.

## Limits and next

- Replacement atomicity relies on the destination filesystem's same-directory rename behavior. The temporary
  file is synced; this does not claim guaranteed survival of directory entries through power loss on every
  filesystem. A process crash can leave a temporary sibling file for later cleanup.
- Trees deeper than 32 instance levels now fail with a scene error instead of saving unreadable output.
- Unknown fields are errors in scene format version 1; format extensions need an explicit version change.
- Review fixes pushed to existing PR #45 and the review answered; issue #31 is back
  to review. Inspect final-head CI and further feedback. Do not merge.
- No changes to renderer, physics, scripts, editor UI or the platform repository.

## API references

- [Rust file rename behavior](https://doc.rust-lang.org/std/fs/fn.rename.html).
- [Exclusive temporary-file creation](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html#method.create_new).
- RON 0.12.3 `Options::with_recursion_limit`, verified against the locally installed dependency source.
- [Serde remote representations](https://serde.rs/remote-derive.html) and
  [strict container fields](https://serde.rs/container-attrs.html).
- [Windows file sharing](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew):
  rename/delete access requires compatible sharing until existing handles close.
