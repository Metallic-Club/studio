# Metallic

A game engine and platform written in Rust. One engine powers three programs:

- **Studio** — the editor (Windows, Linux, macOS)
- **Player** — runs games, standalone or on the Metallic platform
- **Launcher** — shows the website and runs Player (and Studio on PC) inside itself

Games are built from a class-based API (`Object → Instance → WorldObject → Light → SpotLight`) and scripted in
Luau, with room for more languages. Targets: Windows, Linux, macOS, Android, iOS, PlayStation and Xbox. 3D comes
first; 2D comes later.

> **Status:** early foundation work (milestone M1). Nothing is playable yet. See the pinned roadmap issue #1.

## Build

You need Rust; `rust-toolchain.toml` picks the exact version.

```bash
cargo build --workspace
cargo test --workspace
```

## Layout

```
crates/   the engine: core · classes · script · luau · render · physics · project · platform · engine
apps/     player · studio · launcher
docs/     architecture.md and the decision records (adr/)
xtask/    repository automation (`cargo xtask`)
```

- **New here?** Start with [`docs/architecture.md`](docs/architecture.md).
- **An AI agent or a contributor?** Read [`AGENTS.md`](AGENTS.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

See [`LICENSE`](LICENSE). The license is being decided in #27.
