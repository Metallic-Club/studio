# #283 Server-only ScriptHost constructor

Status: implemented and focused tests passed; full required gates in progress.

Branch `studio/283-server-script-host`, based on main
`48b20dd8ac404513dee2a8bc37c4559bf4a9c1ae` (input actions/class registry changes).
Issue freshly checked and claimed. This is a user-prioritized independent foundation;
it does not change blocked M2 epic #15 acceptance.

## API and scope

Add `ScriptHost::new_server(world, output) -> ScriptHost` with a private fixed
run-client flag. Keep `ScriptHost::new` and existing combined behavior unchanged.
Server startup creates only the server VM, no synthetic Player1/PlayerGui/PlayerAdded,
and starts only eligible server Scripts. Preserve pre-existing world instances.
Filter client-context candidates before recording running scripts at startup and on
journal updates. Reuse existing scheduler, event dispatch and shutdown paths.

No transport, player admission, dedicated-server executable, process supervision,
remote machine changes, physics/render extraction or playable-multiplayer claim.
`ModuleScript` and `require` support are not implemented by this task; preserve the
current sandbox. Root clarified that module loading was outside the intended scope.

Issue #173 / PR #285 (branch `studio/173-task-thread-boundaries`) owns separate scheduler
correctness changes. Do not edit or import
`task_lib.rs` or its tests. Eventual untrusted-game hosting still needs that work and
the later process/transport/isolation slices. Re-test composed lifecycle behavior
when it lands; adding a constructor does not fix scheduler bugs.

## Tests first

Six public behavioral cases are written in `tests/server_host.rs`:

- Server Scripts under Workspace and ServerScriptService, Instance writes, actual
  Heartbeat/task.wait, no synthetic player/PlayerAdded and unchanged sandbox.
- Preserve existing player/gui state; suppress LocalScripts at startup and after
  create/reparent/enable journal updates.
- Disable/reparent/destroy cancels only that server script's events/tasks while an
  unrelated delayed control runs; a newly eligible server Script still starts.
- Stop/restart clears old tasks, queued events and signal connections.
- Repeated client-targeted remotes have no receiver or accumulating undelivered work.
- Existing default constructor runs both sides and delivers local PlayerAdded.

Tests were written first, then the additive implementation while build slots were
occupied. Following root-worker review, use the final passing boundary suite directly:
an explicit Combined positive control proves client/player behavior is retained,
while the new constructor's cases prove absence of synthetic state/client execution.
There is no deliberately failing comparison or unchanged-main red claim for this new
feature. No existing Combined behavior is declared a defect, and no passing server-only
capability is claimed before execution.

## Validation / next

The guarded lane121 ownership transition removed 3.1 GiB of prior local package
outputs and preserved third-party dependencies. The focused prebuild passed in
10.42 seconds; logs identify this worktree's core/script sources. All six public
cases passed in 0.10 seconds, and the private VM/running-map/queue invariant passed
in 0.01 seconds after a 6.46-second incremental unit-test compile. No failures or
corrective code changes. These non-budget public fixtures use the existing two-second
test-rig resume setting; the production default is unchanged. The private case uses
the default budget. No latency/performance acceptance claim follows from these tests.

Fresh fetch before wider gates confirmed main remains `48b20dd`. Full workspace
prebuild passed with four build jobs through the guard. Strict workspace/all-target
Clippy with `-D warnings` also passed in 1 minute 9 seconds. Logs identify this
worktree's local crate sources. The actual complete serial suite is held until the
backend's bounded readiness investigation/rerun releases its CPU window. Licenses/fmt
remain pending; no full workspace pass is claimed yet.

The constructor and role-aware candidate filtering are implemented; a private unit
checks one VM, excluded client entries, drained no-recipient queues and shutdown.
Public docs/architecture are updated. Run focused/full required gates. Coordinate quiet CPU time for the actual
workspace budget tests. Publish a separate PR and inspect CI; never merge it here.

Root worker coordinator independently reviewed the host delta and six public cases,
with no blocking finding. This is worker feedback, not repository acceptance.
