# Status

Read this first, then `CLAUDE.md`. Updated: 2026-10-30. Statuses are planned / implemented / tested / accepted / released.

## What Metallic is
Metallic is the owner's game platform. This repo holds the **Metallic Engine** (our own game engine, Rust, Windows 11
first) and **Metallic Studio** (the editor creators use). Games run in a shared **Metallic player**; scripts are
sandboxed Luau with Roblox-style names. Website and backend live in `Metallic-Club/plateform`, not here.

## Current milestone: M1 (engine plays Parkour, Studio edits it, installer and updates)
| Part | Goal | Issues |
|---|---|---|
| M1A | `metallic-player` plays the Parkour game | #1 to #8 |
| M1B | Studio creates, edits and plays Parkour; zip acceptance | #9 to #12 |
| M1C | Installer, GitHub Releases updates, developer mode | #13, #14 |

## In progress
- #1 Task 1 (workspace, toolchain, licence gate): lead session, branch `studio/m1-engine-20261030`.

## Ready now
None until #1 is merged. Then #2 (core) is the only open path; after #2: #3 render, #4 physics, #6 Luau VM in parallel.

## Task graph
T1 -> T2 -> {T3, T4, T6}; T4 -> T5; T6 -> T7; {T3, T5, T7} -> T8 -> T9 -> {T10, T13, T14}; T10 -> T11 -> T12.
Issue number = task number (#1 is Task 1 ... #14 is Task 14).

## Later milestones (epics)
#15 M2 Multiplayer + Team Create, #16 M3 2D + more templates, #17 M4 Creation workflow, #18 M5 Renderer track,
#19 M6 Modeller/animation/VFX, #20 M7 Marketplace, community, learning, Litha.

## Where things are
- Spec: `docs/superpowers/specs/2026-10-30-metallic-studio-design.md`
- Plan (M1, Tasks 1-14): `docs/superpowers/plans/2026-10-30-engine-m1.md`
- Brand files: `branding/` (placeholder icon only)
- Per-issue progress notes: `docs/agents/<issue#>-<slug>.md`
- Overview: `README.md`, `ARCHITECTURE.md`
- Board: `gh issue list -R Metallic-Club/studio -l status:ready` (no GitHub Project board; the token lacks the `project` scope)
