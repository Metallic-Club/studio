# Issue 311: explicit ScriptHost roles and player lifecycle

Status: all eighteen role/lifecycle cases and 61 focused cases pass on cb26593;
independent final review is clean. Broad gates are pending; Cargo is held until
a fresh explicit build grant.

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

## Refreshed base and review

Refreshed the unpublished branch once onto
`cb265939f8f1c9a20f9fbfdecd0786e08888df37`, after inspecting both the CFrame
core/script merge and subsequent Studio colour-popover/dependency merge. Rebase
had no conflicts; CFrame members and PlayerRemoving event plumbing both remain.
Checkpoint: `67fcdfd`.

The refreshed prebuild passed in 39.05s. Actual serial focused selection passed
57 cases, zero failed, two existing manual probes ignored: fourteen library,
eighteen API, five new-main CFrame, fourteen role/lifecycle and six server-host cases.
The role/lifecycle group completed in 1.39s. Logs:
`metallic-311-refreshed-prebuild.log`, `metallic-311-refreshed-focused.log`.

C2 independently re-reviewed the scoped pacing fix and reported no further concrete
roster/scope/cancellation finding. Removing's immediate dispatch precedes caller
destruction; deferred handlers read later roster/world state, as documented.
The subsequent ownership corrections are recorded and tested separately below.

## Further ownership-entrypoint audit and reproduction

`dispatch_input_now` runs deferred/action work outside the normal step eligibility
refresh. `remove_player` fires lifecycle handlers through the same client Hub.
A GUI changed to another player's ownership between steps may therefore retain
old executable handlers at these two public entrypoints. Prepared public tests:
deferred input-dispatch work after GUI transfer; lifecycle dispatch with a valid
positive control before transfer; and fail-closed input dispatch while the World
is borrowed, with valid postponed work eventually completing.

The granted two-entrypoint red run failed both cases on the unfixed implementation
in 0.34s (13.51s incremental build). Input dispatch resumed an eighth transferred
deferred handler (expected seven already-completed handlers); lifecycle dispatch
called the transferred observer a second time. Both positive controls passed.
Log: `<TEMP>/metallic-311-ownership-red.log`. At that stage the borrowed-World
control and local-removal input assertion were prepared; they are now included
in the complete green selection below.

The reviewed Client-only prune fix prunes changed
running IDs before either dispatch without consuming the normal journal cursor
or starting new scripts. Added controls require unrelated starter work to continue
and newly created scripts to start only on the following step, without lost changes.
Owner cancellation only removes work/hidden actions; it does not call Lua handlers.
Ordinary/held dispatch deadlines remain in place before immediate Lua execution.

An independent audit also found normal Client step could continue to scheduler work
after sync_scripts could not borrow the World. A public control transfers a GUI
through a held RefMut and advances public step's virtual clock past task.delay's
deadline. It requires no old owned task to run and an unrelated starter task to
remain queued until scope validation succeeds. Fresh Heartbeat is not used as the
repro because resolving RunService also fails under that borrow. The control failed
on the unchanged step path in 0.01s (7.67s incremental build): both owned-task and
global-task ran while scope was unreadable. Log: `metallic-311-step-borrow-red.log`.

The Client-only readable-World preflight now returns before any clock/budget/queue
mutation. The granted complete serial focused selection passed 61 cases, zero
failed, with two existing library probes ignored. All eighteen role/lifecycle cases
passed in 1.61s; all five CFrame cases remain green. This includes both ownership
reds, normal-step/input borrowed-World controls, local removal before destruction,
held deadlines, unrelated starter continuity and no lost late-script starts.
Log: `metallic-311-ownership-green.log`.

C2's final read-only review found no further concrete scope/roster finding. Reviewed
host blob `0ad98326f6fe1a1cfe4ee2a7d0e634dc62b3ac6f` and test blob
`ef96a20fb4f8e29b1a3da19132d3fb2f5a1a3c92`. No scheduler/budget constants,
Server/Combined dispatch behavior or core registry/class interfaces were changed.

## Ownership and next

Only script host/API/event plumbing, focused tests and public docs. Physics state
issue 296 and transport issue 297 are actively owned elsewhere. No scheduler
implementation edits or core class/journal edits. Use guarded lane121 with four
build jobs. The root worker coordinator released slot one after A1's actual suite;
the first focused prebuild covers the script library, host_roles, server_host and
api targets. Root's pure movement-math prebuild has slot two; no other Lua suite
or measurement overlaps the reserved actual focused window.
Do not run further Cargo commands until a fresh explicit build grant; the prior
focused grant ended at its reported completion. Request the next workspace prebuild,
then a separately coordinated actual suite and remaining gates. PlayerScripts coverage still depends
on actual core registration, not a fabricated class or a copied unmerged branch.
Run meaningful focused tests and all required workspace/Clippy/license/format gates,
then open/attach one PR, inspect CI, and do not merge it.
