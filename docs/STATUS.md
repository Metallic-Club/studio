# Status

Snapshot: 2026-10-30 14:39 UTC, main `ebb94f3`. Status words are planned / implemented / tested / accepted / released.
Read [CLAUDE.md](../CLAUDE.md) for the working agreement. Check live issues and claims before choosing work; this
file describes the verified repository snapshot and does not reserve tasks or supersede newer issue comments.

## Product and current milestone

Metallic Engine is the Rust engine; Metallic Studio is the editor; Metallic player runs games in sandboxed Luau.
Windows 11 is first. The website/backend lives in [Metallic-Club/plateform](https://github.com/Metallic-Club/plateform).

Milestone 1 is still in progress. Tasks #1 through #9 are closed, and their implementations are present on main.
Tasks #10 through #14 remain open. Issue labels on closed tasks can lag their actual state; filter for open issues.
This snapshot does not declare the complete editor, installer round trip or milestone acceptance finished.

| Area on main | Implemented evidence | Remaining work |
| --- | --- | --- |
| Engine foundations, Tasks #1-#7 | Seven-crate workspace; typed Instance world and scene format; renderer; Rapier physics and characters; sandboxed Luau, scheduler and Instance/RemoteEvent API | Tracked correctness/performance fixes remain on the issue board; open PRs are not merged implementations |
| Runtime/player/Parkour, Task #8 | `metallic_runtime::Game`, window/headless player, input/camera/game GUI and Parkour template; player smoke, runtime tests and input-driven full-course route test | Maintain regression coverage and complete owner acceptance against the milestone specification |
| Studio shell, Task #9 | Window/theme/splash, start screen, project/template handling and recent projects | Viewport/editor manipulation under #10; Play/Stop under #11 |
| Packaging, Task #12 | Specification and plan exist | `tools/package.py` is absent on this snapshot; package/licence/acceptance work remains open |
| Installer/updater, Task #13 | GitHub auth/update library, release script/workflow, tests and startup Velopack hook | GitHub App configuration and real installed update round trip; sign-in/check UI wiring and tracked release fixes. See [releasing.md](releasing.md) |
| Developer mode, Task #14 | Plan exists | `metallic_studio/src/devmode/` is absent; overlay/log viewer/inspector/flags/report bundle remain open |

The [building-tools design](design/building-tools.md), [visual-language documents](design/visual-language/visual-language.md)
and [Launcher L1 plan](superpowers/plans/2026-10-30-launcher-l1.md) are planning/design artifacts. They do not establish
that the editor tools or desktop Launcher have shipped. Inspect each implementation PR and its acceptance evidence.

## Find current work

Use the live board rather than the original bootstrap statement that nothing is ready:

```sh
gh issue list -R Metallic-Club/studio --state open --label status:ready
gh issue list -R Metallic-Club/studio --state open --label status:in-progress
gh pr list -R Metallic-Club/studio --state open
gh issue view <number> -R Metallic-Club/studio --comments
```

Choose the highest-priority unblocked issue. Verify every dependency is closed and check recent claims before
claiming it. Open Tasks #10-#14 currently carry blocked labels; their issue text and live coordination determine
what can start. Do not infer that a dependency is still open just because a closed issue retained an old label.

The [conductor agreement](CONDUCTOR.md) and [baton issue #44](https://github.com/Metallic-Club/studio/issues/44) govern
coordination. Workers may pick ready issues without waiting for a conductor. Only the lead or owner merges after
review and green CI; keep individual progress in `docs/agents/<issue>-<slug>.md`.

## Commands present on this snapshot

```sh
cargo build --workspace -j 4
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo run -j 4 -p metallic_player -- templates/parkour --headless --scene tests/smoke.mscene --frames 1200
cargo run -j 4 -p metallic_studio
```

The Rust toolchain is pinned in `rust-toolchain.toml`; install `cargo-deny` separately if needed. Daytime builds use
`-j 4`. Follow current conductor resource limits and keep build caches isolated or guarded when changing worktrees;
do not attribute another checkout's cached outputs to this one. A command's presence is not a performance or release claim.

## Plans and later milestones

- [Engine/Studio specification](superpowers/specs/2026-10-30-metallic-studio-design.md)
- [M1 plan, Tasks #1-#14](superpowers/plans/2026-10-30-engine-m1.md)
- [Architecture and crate interfaces](../ARCHITECTURE.md), [Luau API](luau-api.md), [Task #8 progress](agents/8-runtime-parkour.md)
- Later epics: #15 multiplayer/Team Create; #16 2D/templates; #17 creation workflow; #18 renderer track;
  #19 modeller/animation/VFX; #20 marketplace/community/learning/Litha.

The original dependency graph remains in the M1 plan. Preserve its contracts while fixing the current tracked
issues; a closed implementation task does not by itself prove every later milestone or owner acceptance step.
