# Issue 311: explicit ScriptHost roles and player lifecycle

Status: refreshed source `01ca571debe1e5c14770e19376c76e887eb4d52d` on pinned main
`a0bb1bfc0e84ffd637fe8d1759f4822322601cd8` passed all required local gates. The full locked serial
workspace command exited zero: 1,868 passed, zero failed, eight upstream ignored across 138 summaries.
Focused GUI/role/server controls passed 29 cases after a retained one-failure test-only baseline. The actual
window/lane122 were released directly to root's combined CPU check. Append-only publication to existing
PR #335 and exact-head CI remain next; no own merge, force push or new-main chase.
Earlier results below remain source-specific history, rather than relabeled current-main results.

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
issue 296 and transport issue 297 were implemented elsewhere and are now on the
final pinned base. This issue changes neither implementation. No scheduler
implementation edits or core class/journal edits. Use guarded lane121 with four
build jobs. The root worker coordinator released slot one after A1's actual suite;
the first focused prebuild covered the script library, host_roles, server_host and
api targets. Root's pure movement-math prebuild had slot two; no other Lua suite
or measurement overlapped that focused actual window.
Before broad gates the root worker coordinator required a second compatibility
refresh onto exact `d1eeccee2c3d46ca61921baffa562af4bd9472e8`. Main now contains
A4 CharacterState/replica physics, A5 QUIC/memory transport, pivot APIs with boxed
Value::CFrame/PropDef.hidden, controls, avatar and Studio additions. This is a
substantive ABI/source refresh, not a documentation-only change. The rebase had
no conflicts; PlayerRemoving, upstream Lua conversions, PV methods and hidden
property filtering are all retained. Current code checkpoint: `593371a`.

Fetch observed newer main `2e17419`, deliberately excluded from this pinned gate
base. The issue diff remains the same eight script/docs files with no core,
manifest or lockfile edits. All 61 focused results above remain labeled to cb26593;
the completed new-base gates are recorded below.

The slot-one grant covered only `test --workspace --no-run -j 4` in guarded
lane121; its completion was reported before separately granted actual/check windows.
PlayerScripts coverage still depends
on actual core registration, not a fabricated class or a copied unmerged branch.

## Final gates on pinned d1eecce

Frozen source checkpoint `593371a432403e5a044bebc0ad24f58832151ef6`:

- Guarded full workspace no-run prebuild passed in 6m10s, without warnings/errors.
- Separately granted actual `test --workspace -j 4 -- --test-threads=1` exited zero:
  1,242 passed, zero failed, six existing ignored across 110 harness/doc groups.
  Networking, physics, pivot, runtime, script-role and documentation tests all passed.
  Ignored tests are three window tests, two getter diagnostics and one real-budget
  pattern probe. No new case is ignored. The source remained frozen during this run;
  other managed Lua suites were held, with only short root core-only work permitted.
- Strict full workspace all-target Clippy with `-D warnings` passed in 1m20s.
  License, format and whitespace checks passed in the same granted check sequence.
- Saved logs: `metallic-311-workspace-prebuild.log`,
  `metallic-311-workspace-final.log`, `metallic-311-clippy-final.log`,
  `metallic-311-licenses-final.log` under `<TEMP>`.

Lane121 and its actual/check windows are released; all Cargo is held. Commit this
documentation-only result, publish/attach the PR, set issue 311 to review and inspect
CI. This does not change tested production behavior or claim A3/M2 acceptance.
Do not merge the PR.


## Current-main compatibility refresh

Previous published head: `e8f6efea1455b53f9d3ae4833b08e1ce3bedfa9a`. The conductor assigned this existing
branch/worktree to Codex studio_core while engine_research owns #309/runtime and another worker owns #317.
Normal append-only merge of pinned main `a0bb1bfc0e84ffd637fe8d1759f4822322601cd8`; no rebase, force push
or own PR merge. Issue #311 is in progress during the refresh. All main source changes remain intact.

Only two textual conflicts occurred: `host.rs::stop` keeps both client-scope reset and GUI-queue clearing;
`lib.rs` keeps GUI exports plus ScriptRole. Auto-merged GUI event queuing still referenced removed
`run_client`; it now uses the equivalent explicit `role != Server` predicate. TouchEnded, GUI signal kinds
and PlayerRemoving coexist. Compilation is held, so no new green result is claimed.

Next source/test check: queue cleanup across explicit Client rebind/removal/invalid binding must include the
new GUI queue alongside existing input/remotes. Add public role/GUI payload and transition controls, observe
proposed-integration reds when granted, then apply narrow queue cleanup if reproduced. Preserve trusted
roster ownership, aggregate/held deadlines, nil/multiple coroutine results and native protected-call behavior.
No runtime driver, edit kernel, journal or dependency/schema work is included in this refresh.


## Proposed current-main GUI regressions (not executed yet)

Compatibility merge checkpoint: `26ac6006213158ab199185ea99274dd83c23a37f`. Three public tests now cover
Combined/Server/Client routing with ordered button/pointer payloads; queued clicks across a no-op bind,
actual rebind and local removal/rebind with fresh-input controls; and borrowed-World retry plus transferred
GUI script ownership with a newly eligible positive control. The rebind case is expected to expose a
proposed-integration queue leak; no executed red or new-main bug claim is made before a granted run.

The minimal merge changes are complete, but queue behavior remains uncorrected for the test-only checkpoint.
Do not start Cargo while compilation is held. When a slot/window is granted, compile first and distinguish
fixture errors from actual behavior failures, retain the red result, then add GUI clearing at the same binding
cleanup points as existing input/remotes. Production budgets, scheduler and protected-call APIs stay unchanged.


## Observed current-main integration red

At test-only `70a123b` on pinned `a0bb1bfc`, the granted focused command selected gui_events, host_roles and
server_host. Cargo returned 101: both existing GUI cases passed, and the role binary passed 20 cases but
failed `queued_gui_events_do_not_cross_client_binding_changes`: the replacement binding received one click
queued for the previous binding (expected zero). The valid/no-op first-binding controls ran before that
assertion; role routing, pointer payload, World-borrow/GUI ownership and all existing role-budget cases passed.
Cargo stopped after that binary, so server_host was compiled but not executed in this baseline command.
This is a reproduced gap in the proposed refreshed role integration, not an unchanged-main bug claim.
Raw log: `integration-evidence/335-main-refresh/focused-gui-role-baseline.log`.

The narrow correction adds GUI queue clearing beside the existing action-input/remotes cleanup for explicit
rebind, local removal and invalid-binding cleanup. The same-binding early return is preserved; stop keeps
both cleanup paths from the merge. No production budget, scheduler, authentication or roster ordering change.
The focused green rerun is next within the granted short window, then release it to #309 before broad gates.


## Focused green and resource hold

The identical focused selection at `ad3ded1d55a0e8ae8350d23edc60539c5ba4cccc` exited zero: 29 passed,
zero failures or ignores (21 role/lifecycle, two GUI, six server-only cases). The role binary completed in
1.68s; command durations are observations rather than latency benchmarks. The stale-click failure is fixed,
while valid no-op binding input, fresh post-rebind input, ownership/borrow retry, routing/payload, held
aggregate deadlines and all older role controls pass. No scheduler or production budget changed.

Logs and `focused-results.json` remain under `integration-evidence/335-main-refresh`. The compile/actual
window was explicitly released to #309. Root subsequently held all Cargo because C: filled during another
prebuild; this branch has no active compiler or test. Do not start broad gates until a slot is reassigned.
The old root lane114/debug has no binary-retention requirement from this task; named benchmark/release domains
remain protected. No new-main bug, multiplayer acceptance, authentication or direct PR/main merge is claimed.


## Final current-main mandatory gates

Frozen source `01ca571` on exact main `a0bb1bfc` passed the guarded workspace no-run (2m39s) and strict
workspace/all-target Clippy (34.23s), both locked and four build jobs. Licenses (`licenses ok`), formatting
and whitespace passed. Disk remained above the five-GB threshold after root's coordinated cleanup; no
named benchmark/release artifacts were overwritten or copied. All local packages came from this worktree.

After verified #318 completion, the reserved normal guarded command
`test --workspace --locked -j 4 -- --test-threads=1` exited zero: **1,868 passed, zero failed, eight upstream
ignored cases across 138 summaries**, including GUI/role, Lua, Parkour and renderer groups. No failure was
retried, removed or newly ignored, and no production budget or scheduler constant changed. The eight ignores
are three upstream window-opening tests, a GPU stress case, two getter diagnostics, the manual large-pattern
acceptance probe and an existing Studio documentation example. This refresh adds no ignore.

The actual window was released immediately after verified exit to root's private combined CPU check.
`integration-evidence/335-main-refresh/manifest-before-full.json`, raw workspace log and `workspace-result.json`
retain exact source/tree/blobs and checksums; focused baseline/green logs remain separate. Historical 1,242-test
and other earlier results above keep their original bases. The final commit records validation prose only.
Push append-only to the existing branch/PR, return #311 to review, and inspect current-head CI. No aggregate
PR, main merge, authentication, admission, complete runtime-driver or live multiplayer acceptance is claimed.


## Second pinned-main refresh: ModuleScript compatibility (source preparation)

The conductor assigned a normal append-only merge of exact main
`995f880ced789284354ccbb1e64b6bf841382abc` into published
`65c4c99644e36a6ba8f20a0b752414235b558db2`. Merge checkpoint:
`990874b139ea627cefffb521830d14588ad6679a`. The only textual conflict was in
`docs/luau-api.md`: retain explicit roles/roster/lifecycle/GUI cleanup documentation and
main's complete ModuleScript/require section; remove the obsolete claim that this branch
has no loader. No production source needed manual conflict resolution. The loader,
script environment helper and stop_owner -> require.release_owner hook match pinned main.
All earlier validation above remains tied to its original source, not this merge.

New public controls in `tests/require_roles.rs` cover Combined/Server/Client module
visibility and per-VM caching, completed Client cache retention across rebind versus fresh
cache on restart, suspended-loader release on rebind with no stale wake, and transfer of a
PlayerGui loader to another player while an eligible waiter takes over. This main has no
ServerStorage class/service, so the visibility fixture uses actual ServerScriptService;
no replacement class is fabricated. Existing require cycle/error/deadline/sandbox cases
are preserved untouched. No new timeout value, ignore or protected-call replacement.

All Cargo is held pending the conductor's grant. No new test result is claimed. The
released lane124 is the eventual guard-owned target; lane122 belongs to physics408 and
lane123 to root305. Next: inspect the frozen source, run focused role/require plus GUI and
server controls when granted, then mandatory full gates and append-only PR update. No
transport, admission, blocked #336 implementation or multiplayer acceptance is included.
