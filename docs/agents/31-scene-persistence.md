# #31 Scene persistence integrity

Status: implemented and tested; ready for review, not accepted or released.
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

## Verification

- Before implementation, the new integrity suite had five failures: duplicate properties, invalid built-in
  markers, unknown fields, excessive save depth, and a saved 32-level tree that failed to load.
- `cargo test -p metallic_core -j 4`: passed 28 tests, including 12 new regression/unit tests.
- `cargo test --workspace -j 4`: passed 45 tests, including all existing physics tests.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed (existing unused-license-allowance warnings).
- `cargo fmt --all` and `git diff --check`: passed.
- Tested on Windows with Rust 1.99.1. No UI changes or screenshot requirement.

## Limits and next

- Replacement atomicity relies on the destination filesystem's same-directory rename behavior. The temporary
  file is synced; this does not claim guaranteed survival of directory entries through power loss on every
  filesystem. A process crash can leave a temporary sibling file for later cleanup.
- Trees deeper than 32 instance levels now fail with a scene error instead of saving unreadable output.
- Unknown fields are errors in scene format version 1; format extensions need an explicit version change.
- Open the scoped PR for owner/lead review after rebasing on latest `origin/main`; do not merge it here.
- No changes to renderer, physics, scripts, editor UI or the platform repository.

## API references

- [Rust file rename behavior](https://doc.rust-lang.org/std/fs/fn.rename.html).
- [Exclusive temporary-file creation](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html#method.create_new).
- RON 0.12.3 `Options::with_recursion_limit`, verified against the locally installed dependency source.
