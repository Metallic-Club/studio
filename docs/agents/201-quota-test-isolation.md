# #201 Quota test isolation

Status: implemented and locally tested; independent review passed; PR/CI pending.
Branch: `studio/201-quota-test-isolation`, based on `ebb94f3`.

## Evidence and goal

Issue #201 records a full parallel workspace run where Client2 timed out instead of producing the expected
queue-full assertion; the exact test and serial workspace rerun passed. A wall-clock execution budget interfered
with a queue assertion under concurrent load; this does not establish that production sandbox budgets are wrong.

A fresh baseline run with 16 harness threads passed all 14 limits tests in 2.32 seconds. The original intermittent
failure did not reproduce in that run; no deterministic red result or general host scheduling guarantee is claimed.

## Change

- Serialize only tests within `metallic_script/tests/limits.rs` using a local standard-library mutex. Each guard is
  acquired before VM construction or timing and held through fixture teardown. A prior test panic releases the
  guard; later independent tests can still run despite poison.
- Keep deliberate infinite-loop/timeout cases from starving queue/payload/metatable correctness cases in the same
  binary. No global CI setting, production code, execution/memory budget, safety timeout, skip or new dependency.
- Build the one-million-byte payload once per queue fixture, then send the same bytes each time. The original
  Client2 rebuilt it 30 times despite an already-full message queue; that unrelated allocation work obscured the
  queue assertion. Client3 still enqueues independently serialized messages and must hit the same 16 MB byte cap.
- Preserve all existing message counts, byte-limit assertions, error text checks and recovery behavior.

## Verification

Reviewed issue-209 Cargo helper scoped-cleaned local packages on ownership transition; compile paths identify this
worktree. Baseline parallel limits run passed as noted above. The modified limits binary passed all 14 tests with
16 harness threads in 5.63 seconds; serialization adds about 3.3 seconds in this observation.

- Normal `cargo test --workspace -j 4`: 212 passed, with the default parallel harness unchanged. No ignored tests.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`, `cargo deny check licenses`, `cargo fmt --check`
  and `git diff --check`: passed. No dependencies or production source changes.
- Independent read-only review verified all 14 guards precede timers/VMs and outlive fixture teardown. It also
  checked that each FireServer still serializes/copies and charges its own payload bytes, preserving the queue
  count/byte-limit assertions. No actionable findings.

## Next

Commit/push/PR and inspect normal CI. Host-wide scheduling delays from other processes remain possible; this change
removes interference introduced by this test binary and redundant payload construction, not all wall-clock noise.
