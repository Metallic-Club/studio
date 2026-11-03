# Working in this repository

Read this before you touch anything. The full rules and the roadmap are in the pinned issue **#1**.

Metallic is a Rust game engine and platform: one engine (`crates/`) powers Studio, Player and Launcher (`apps/`).
The decisions behind the layout are in [`docs/adr/`](docs/adr/), and the map is in
[`docs/architecture.md`](docs/architecture.md).

## The main rule: few things at once

1. **One active milestone.** Only issues labelled `status:ready` may be started.
2. **At most 3 issues `status:in-progress` in the repo, and at most 1 per `area:*` label.**
3. Pick the highest priority (`P0` → `P3`), then the lowest number. Never start a `status:blocked` issue.
4. **Stay in scope.** Work outside the issue's "Done when" list becomes a new issue, not part of your PR.
5. Questions only the owner can answer go on the issue with `status:needs-owner`. Don't guess.

## Working an issue

1. **Claim:** make sure no open PR references the issue (`gh pr list --search <N>`), then assign yourself, set
   `status:in-progress` and comment `Claimed by <session>`.
2. **Branch:** `<issue#>-<slug>`, in its own git worktree, with its own `target/` folder. Never share a
   `CARGO_TARGET_DIR` between worktrees, because their binaries have the same names.
3. **Commit:** messages start with the area: `core: add CFrame inverse (#12)`.
4. **Check** before opening the PR. CI (`.github/workflows/ci.yml`) runs the same commands:
   ```bash
   cargo fmt --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo xtask check-layout   # crate boundaries, metallic_core's allow-list, cfg(target_os) (ADR 0001)
   cargo check -p metallic_engine --no-default-features --features physics,luau   # headless server build
   cargo deny check           # advisories, licenses, sources (cargo install cargo-deny --locked)
   ```
   CI also runs `cargo check --workspace --exclude metallic_studio --exclude xtask --target <t>` for
   `aarch64-linux-android` and `aarch64-apple-ios` (after `rustup target add <t>`), and macOS tests on `main`.
   PRs that touch `crates/` also run the **perf gate** (`.github/workflows/perf.yml`): it fails when a
   `metallic_core` hot path executes more than 5% more instructions than on the base. It needs Linux and Valgrind;
   see [`docs/perf.md`](docs/perf.md) to run it locally or add a workload.
5. **Open the PR:** one per issue, with `Closes #N` in the body, then set the issue to `status:review`. Never push to
   `main`. Never merge your own PR; the conductor reviews and merges.
6. **If you have to stop:** leave the branch building and comment on the issue saying where you stopped.

## Code rules

- **Respect the crate boundaries** in [ADR 0001](docs/adr/0001-project-layout.md). Dependencies only point down;
  `metallic_core` stays pure; `#[cfg(target_os = ...)]` appears only in `crates/platform` and `apps/`.
- **Every public item has a doc comment** (`missing_docs` is on, and CI treats warnings as errors). Say what the
  item is for, not how it is written.
- **No `unwrap()` / `expect()` outside tests.** Return errors (`thiserror` in libraries). A panic in the engine is a
  bug.
- **No `unsafe`** unless the crate root allows it with a comment explaining why. Every `unsafe` block needs a
  `// SAFETY:` comment.
- **Tests sit next to the code** (`#[cfg(test)] mod tests`). Integration tests go in `crates/<name>/tests/`.
- **New dependencies** are declared in the root `Cargo.toml` under `[workspace.dependencies]`; crates use
  `dep.workspace = true`. Prefer small, portable crates. Every dependency must also build for Android and iOS.
- **Comments explain why**, not what. Match the style of the code around you.
