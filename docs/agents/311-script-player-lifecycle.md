# Issue 311: explicit ScriptHost roles and player lifecycle

Status: implementation and fourteen public behavioral cases pass focused checks;
refreshing unpublished branch onto new main before broader gates.

Branch `studio/311-script-player-lifecycle`, initial main
`e20f0828454d21ae2325b671add29f9bf5c3a7c3`. This user-prioritized remaining A3
slice follows Simulation issue 299/PR 309; it does not accept A3/M2 or provide
authentication, admission, snapshots, transport, process hosting or playable multiplayer.

## Scope and design

- Replace the fixed private boolean with explicit Combined/Server/Client roles,
  preserving `new` and `new_server`. Client startup creates one client VM and no
  synthetic Player/PlayerGui. LocalScript startup waits for trusted local binding.
- Player APIs consume existing world-local InstanceIds, never backend numeric IDs.
  Add/bind reject stale, non-Player and improperly parented instances. An explicit
  roster deduplicates Added/Removing and retains deterministic World child order.
- Preserve legacy queued PlayerAdded/GetPlayers timing. Pending add then removal
  in an active session delivers Added before Removing; stopped-session cancellation
  emits no callbacks. Removing is dispatched while the Player is still alive, with
  normal signal/resume budgets. Yielded/deferred handlers are not guaranteed to finish
  before later destruction.
- The caller owns Player/Character destruction after removal and before the next
  step. ScriptHost owns VM/tasks/connections/event cleanup, with existing journal
  cleanup for destroyed Character scripts. Stop/restart clears session state and
  replays live registered players once in fresh VMs.
- Client ownership uses current local PlayerGui/Character and existing
  StarterPlayerScripts. Rebinding stops old client-owned work; local Character or
  binding-validity changes force eligibility re-evaluation. PlayerScripts is not
  registered on the initial main base: do not fabricate it or copy unmerged PR 305.
  Track its actual class-dependent fixture until that dependency lands.

## Test matrix

Role/context flags and startup separation; no synthetic Client player; deferred
binding; two-player ownership and late scripts/reparenting; local Character change;
invalid handles and borrowed World; duplicate add/remove and pending removal order;
GetPlayers during lifecycle callbacks; Removing before caller destruction; task/event
cleanup and restart; unchanged legacy Combined tests. Use actual host/World APIs.

## Evidence on initial base e20f082

- Guarded focused prebuild passed in 30.91s, after lane ownership cleanup of prior
  local package artifacts. The first actual selection passed 50 cases: fourteen
  library, eighteen API, twelve role/lifecycle and six server-only cases; two
  existing manual library probes were ignored.
- Independent read-only C2 review identified a new-path pacing gap: synchronous
  removal outside step had no aggregate deadline. A real public-API regression
  failed in 0.33s because all eight independently owned 40 ms handlers ran at once.
  This was a bug in the proposed implementation, not an unchanged-main claim.
- Added an internal RAII dispatch-budget guard. It starts the normal aggregate
  budget only when none exists and ends only its own boundary; active/held frame
  deadlines and all scheduler/resume budget constants remain unchanged.
- The repeated focused selection passed 52 cases, zero failures, with the same two
  existing ignored probes. All fourteen role/lifecycle cases passed in 1.32s,
  including the multi-owner regression and an expired held-input deadline control.
- Saved logs: `<TEMP>/metallic-311-focused-prebuild.log`,
  `metallic-311-focused-first.log`, `metallic-311-pacing-red.log` and
  `metallic-311-focused-paced.log`. No broad workspace result is claimed yet.

## Ownership and next

Only script host/API/event plumbing, focused tests and public docs. Physics state
issue 296 and transport issue 297 are actively owned elsewhere. No scheduler
implementation edits or core class/journal edits. Use guarded lane121 with four
build jobs. The root worker coordinator released slot one after A1's actual suite;
the first focused prebuild covers the script library, host_roles, server_host and
api targets. Root's pure movement-math prebuild has slot two; no other Lua suite
or measurement overlaps the reserved actual focused window.
Main advanced with CFrame APIs; inspect and refresh once before broad validation,
preserving both API/event plumbing changes. PlayerScripts coverage still depends
on actual core registration, not a fabricated class or a copied unmerged branch.
Run meaningful focused tests and all required workspace/Clippy/license/format gates,
then open/attach one PR, inspect CI, and do not merge it.
