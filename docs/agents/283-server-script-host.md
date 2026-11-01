# #283 Server-only ScriptHost constructor

Status: implemented, independently reviewed and locally tested; PR publication/CI next.

Branch `studio/283-server-script-host`, based on main
`48b20dd8ac404513dee2a8bc37c4559bf4a9c1ae` (input actions/class registry changes).
Issue freshly checked and claimed. This is a user-prioritized independent foundation;
it does not change blocked M2 epic #15 acceptance.

Before the actual full suite, main advanced to
`c9cba5fdceb1fa0d3622a1cea622f625a04701ab`, merging borrowed reads (#185), runtime
membership (#262), required-update pagination (#203), and Luau/runtime input APIs
(#251). Rebased the unpublished branch onto this pinned base. The only conflict was
startup: keep main's input-state clear and this role's optional PlayerAdded list.
All new input methods/dispatch and exports remain. Older focused/prebuild/Clippy
results below remain evidence for `48b20dd`, not this updated source.

Server mode now drops client action events at the normal delivery step because it
has no client receiver; Combined delivery and input-type metadata remain unchanged.
The private queue invariant covers this path, and the public startup case retains
server-side InputAction state rejection. No scheduler or transport changes.

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

## Initial `48b20dd` verification

The guarded lane121 ownership transition removed 3.1 GiB of prior local package
outputs and preserved third-party dependencies. The focused prebuild passed in
10.42 seconds; logs identify this worktree's core/script sources. All six public
cases passed in 0.10 seconds, and the private VM/running-map/queue invariant passed
in 0.01 seconds after a 6.46-second incremental unit-test compile. No failures or
corrective code changes. These non-budget public fixtures use the existing two-second
test-rig resume setting; the production default is unchanged. The private case uses
the default budget. No latency/performance acceptance claim follows from these tests.

On the original `48b20dd` source, full workspace
prebuild passed with four build jobs through the guard. Strict workspace/all-target
Clippy with `-D warnings` also passed in 1 minute 9 seconds. Logs identify this
worktree's local crate sources. The actual complete serial suite was held for the
backend's bounded readiness investigation/rerun. Main advanced during that hold,
so no complete workspace pass is claimed for this original base.

## Final pinned-base verification

On `c9cba5fdceb1fa0d3622a1cea622f625a04701ab`, the refreshed script library and role
suite passed: ten library units (two existing manual probes ignored), plus six public
cases. The private invariant also covers no-recipient action queue draining and the
public startup case preserves client-only InputAction state access.

The refreshed workspace prebuild passed in 1 minute 18 seconds. The actual complete
serial workspace suite then passed **760 tests, zero failures, three ignored**. The
ignored cases were the two diagnostic property-getter probes and the real-budget
twelve-MB pattern acceptance probe; none were executed or counted as passing. New
role cases, current input/script tests, membership, player/runtime/avatar and full
input-driven Parkour route all passed. Strict workspace/all-target Clippy passed in
27.08 seconds; licenses, format and whitespace checks passed. No source corrections
were needed after these tests.

Commands used four build jobs and the owned lane121 through `tools/cargo_lane.py`:

```text
cargo test -p metallic_script --lib --test server_host -j 4 -- --test-threads=1
cargo test --workspace --no-run -j 4
cargo test --workspace -j 4 -- --test-threads=1
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --check
git diff --check
```

Saved local output is in the `metallic-283-focused-c9.log`,
`metallic-283-workspace-c9.log` and `metallic-283-clippy-c9.log` temporary artifacts.
Root worker coordinator approved publishing this tested pinned base without another
unrelated main chase; CI validates its merge result. No networking, process-hosting,
capacity or M2 acceptance claim. PR #285 composition remains a separate focused
verification after publication, without modifying reviewed branches.

The constructor and role-aware candidate filtering are implemented; a private unit
checks one VM, excluded client entries, drained no-recipient queues and shutdown.
Public docs/architecture are updated. Next: publish the separate PR, inspect CI and
hand the exact head to the separate PR #285 composition check. Never merge it here.

Root worker coordinator independently reviewed the host delta and six public cases,
with no blocking finding. This is worker feedback, not repository acceptance.
