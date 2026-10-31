# #123 Release: dist profile

Status: implemented and measured locally on Windows; no release has been built with it yet.
Branch: `studio/123-dist-profile`.

## Goal
A Cargo profile for shipped builds (proposal 4.1.10 in `docs/research/2026-10-30-performance-and-scale.md`): choose
LTO, codegen units, panic strategy and debug info from measurements and from what the code needs, then make the
release script and workflow use it.

## Decision
```toml
[profile.dist]
inherits = "release"
lto = "thin"
codegen-units = 1
panic = "unwind"
debug = "line-tables-only"
split-debuginfo = "packed"
strip = "none"
```
- **LTO thin, one codegen unit.** The three shipped exes get 4.4-6.8 % smaller, and the build takes about as long as
  `--release` on this PC. Fat LTO was still on its final link of the three binaries after 26.6 minutes at `-j 4`, so I
  stopped it (over the 25-minute limit) and did not adopt it. A smaller exe also makes Velopack's per-file deltas smaller.
- **panic = "unwind" (required).** `grep catch_unwind` finds:
  - `crates/metallic_asset/src/import.rs:76`: shipped code. The glTF parser is wrapped so a hostile file returns an
    error instead of killing the process. Studio and the player both depend on `metallic_asset` (`cargo tree`).
  - `crates/metallic_core/examples/support/requested_heap.rs:113` (example) and
    `crates/metallic_core/src/signal.rs:83` (a test). Both need unwinding to work as written.
  - mlua 0.12.2 (`state/util.rs` and others) wraps every Rust callback in `catch_unwind` and turns a panic into a Lua
    error. Studio and the player both run Luau.
  - Thread joins that treat a panicked thread as an error: `metallic_update/src/feed.rs:453` (Studio and Launcher)
    and `metallic_audio/src/device.rs:64` (not in a shipped exe today).
  With `panic = "abort"`, each of these would kill the process instead.
- **Debug info: line tables in a separate file, nothing stripped.** On Windows the debug info always goes into the
  `.pdb` beside the exe (`split-debuginfo = "packed"` is the MSVC default; it is written out so that macOS and Linux
  builds also keep it out of the binary). The exe only gains a PDB path, so it gets no bigger, and a crash address can
  be resolved to file:line with the matching `.pdb`. `strip` removes exactly that debug info (I believe on MSVC it also
  stops the PDB being written; not checked), so it stays `"none"`. The `.pdb` is not packed: `tools/release.py` copies it to `target/release-symbols/<tag>/`.

## Measurements
PC: 12 threads, Windows 11, rustc 1.99.1. Other sessions were building at the same time (7-18 rustc/cargo processes),
so build times are only accurate to a few minutes. Every build was cold, in its own target dir, with `-j 4 --locked`:
`cargo build -p metallic_studio -p metallic_launcher -p metallic_player`.

| | `--release` | `--profile dist` (thin) | `dist` with `lto = "fat"` |
|---|---:|---:|---:|
| Build time, 3 shipped bins | 1195 s | 1093 s | stopped after 26.6 min, still linking |
| Build time, `perf_ci` example afterwards | 64 s | 59 s | - |
| `metallic-studio.exe` | 21,657,600 B | 20,193,280 B (-6.8 %) | - |
| `metallic-player.exe` | 21,377,024 B | 20,446,208 B (-4.4 %) | - |
| `Metallic.exe` (Launcher) | 7,441,920 B | 6,968,832 B (-6.4 %) | - |
| `metallic_studio.pdb` (not shipped) | 9,687,040 B | 51,138,560 B | - |
| `metallic_player.pdb` (not shipped) | 8,876,032 B | 58,535,936 B | - |
| `Metallic.pdb` (not shipped) | 5,738,496 B | 22,122,496 B | - |
| `perf_ci reads`, median of 21 | 2.663 ms | 2.646 ms (-0.7 %) | - |
| `perf_ci writes`, median of 21 | 1.163 ms | 1.157 ms (-0.5 %) | - |
| `perf_ci clone`, median of 21 | 1.404 ms | 1.429 ms (+1.7 %) | - |

The perf_ci runs were interleaved (release, thin, release, ...) once the builds had finished. The differences are
within noise: the core workloads are one crate plus std, and `release` already does crate-local thin LTO, so thin LTO
gains little there. The 10-20 % from the perf book is not shown for our code; a real frame benchmark (render stress,
Studio play) is the place to look next. Start-up was not measured.

## Done
- `[profile.dist]` in the root `Cargo.toml`. Nothing else in Cargo.toml changed, no dependencies, and `Cargo.lock` is
  unchanged.
- `tools/release.py`: `build_command()` runs `cargo build --locked --profile dist -p <pkg>... -j N`. The exes are
  staged from `target/dist/` (`BUILD_DIR`), and the `.pdb` files are copied to `target/release-symbols/<tag>/`, with a
  warning if one is missing. Token handling is unchanged. Note: the script had no `--locked` before; I added it, so a
  release now fails if `Cargo.lock` is out of date, as the other CI builds already do.
- `tools/test_release.py`: dry-run expectations for the new command lines; checks of the build command, the profile
  (inherits release, keeps unwinding, `target/dist`) and the PDB names. All 18 tests pass.
- `.github/workflows/release.yml`: comment and step name only. The workflow builds through `release.py`; its token env
  is unchanged.
- `docs/releasing.md`: the profile, the reasons for unwinding, and where the symbols go.

## Next
- Keep the PDBs from the workflow: the runner is thrown away, so CI releases lose them today. This needs an
  `actions/upload-artifact` step pinned to a full SHA (the release workflow pins every action), or an upload to the
  release as an extra asset. That is a decision about where symbols live, so it is left open.
- Measure a frame-time benchmark and Studio start-up under `dist`, and record exe size and cold start in release notes
  (proposal 6.10).
- Fat LTO could be measured at night with more jobs, if size or speed ever matters more than build time.

## Commands
```
cargo build --locked --release       -p metallic_studio -p metallic_launcher -p metallic_player -j 4 --target-dir target/m-release
cargo build --locked --profile dist  -p metallic_studio -p metallic_launcher -p metallic_player -j 4 --target-dir target/m-thin
CARGO_PROFILE_DIST_LTO=fat cargo build --locked --profile dist ... --target-dir target/m-fat   # stopped at 26.6 min
cargo build --locked --profile dist -p metallic_core --example perf_ci -j 4 --target-dir target/m-thin
target/m-thin/dist/examples/perf_ci.exe reads|writes|clone
py -3 -I tools/test_release.py
py -3 -I tools/release.py --channel dev --version 0.1.1 --dry-run
```
