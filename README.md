# Metallic Engine + Metallic Studio

The **Metallic Engine** is our own game engine, written in Rust. **Metallic Studio** is the editor creators use to
make 2D and 3D games with it. Games run in the shared **Metallic player**, scripted in sandboxed Luau with Roblox-style names.

Status: milestone 1 is being built (see `docs/STATUS.md`). Private repo; the website/backend is `Metallic-Club/plateform`.

## Quick start (Windows 11)
Prerequisites: Git, [Rust via rustup](https://rustup.rs). The exact toolchain installs itself from `rust-toolchain.toml`
on the first `cargo` call. Git Bash: `export PATH="$HOME/.cargo/bin:$PATH"`.

| Do | Command | Exists yet? |
|---|---|---|
| Build | `cargo build --workspace` | after Task 1 (#1) |
| Test | `cargo test --workspace` | after Task 1 |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | after Task 1 |
| Licence check | `cargo deny check licenses` (`cargo install cargo-deny --locked` once) | after Task 1 |
| Play Parkour | `cargo run --release -p metallic_player -- templates/parkour` | after Task 8 (#8) |
| Open Studio | `cargo run --release -p metallic_studio` | after Task 9 (#9) |
| Package a zip | `python tools/package.py` | after Task 12 (#12) |

Daytime tip: add `-j 4` to cargo so your PC stays responsive.

## Repo map
- `crates/` Cargo workspace: `metallic_core`, `_render`, `_physics`, `_script`, `_runtime`, `_player`, `_studio` (not yet created)
- `templates/` game templates as project folders (Parkour first)
- `branding/` icon and splash files; the only place they live
- `docs/` spec, plan, `STATUS.md`, `luau-api.md` (later), `agents/` per-issue progress notes
- `tools/` packaging and release scripts (Python)
- `.github/` CI, issue and PR templates

## Working here
- [`CLAUDE.md`](CLAUDE.md) working agreement for AI sessions and people: claim an issue, branch, PR
- [`ARCHITECTURE.md`](ARCHITECTURE.md) how the crates fit together and where to add things
- [`docs/STATUS.md`](docs/STATUS.md) what is in progress now
- [Design spec](docs/superpowers/specs/2026-10-30-metallic-studio-design.md) and [M1 plan](docs/superpowers/plans/2026-10-30-engine-m1.md)
- Issues are the task list: `gh issue list -R Metallic-Club/studio -l status:ready`
