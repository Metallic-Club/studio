# 353: headless GPU failure policy and diagnostics

Child of #276. That native-crash issue remains open; no root cause or accepted fix
is claimed. Claimed by the Codex engine-research worker on branch
`studio/353-gpu-test-diagnostics`, based on exact main
`c914767167406da06292398769124ba1240b0ff0`. PR309 remains unchanged.

## Scope

- Existing four headless, four lighting and seven post-chain cases retain their
  assertions, per-test context ownership and parallelism. Test-only wrappers skip
  only explicit NoAdapter; device/readback errors fail with phases and adapter
  metadata. Existing GL coverage stays explicitly GL.
- Strict test-only backend selection; no production interfaces/defaults or
  dependencies change. Handle-drop phases describe Rust operations, not native
  final-free guarantees. No extra polling or serialization is added.
- Adapter-independent policy tests and a standard-library fresh-process runner
  with policy/process tests. Three balanced one/four-thread pairs, 120-second
  children, retained raw/signed/hex statuses, adapter coverage, first-failure stop.
  All-skipped or partial GPU coverage is inconclusive.

## Triage evidence

The reported native crashes are the issue body and linked #269 comment. A scoped
search of workspace integration evidence/worktree text and 242 Metallic-prefixed
temporary logs found no original crash stack/log; this does not establish its
absence elsewhere. Source readback retains resources through submission/poll/map,
drops the mapped view before unmap, and has no confirmed lifetime defect. Instance
drop is supported by wgpu. Merely setting WGPU_BACKEND does not configure the
current default constructor; explicit test selection is required.

## Next / checks

The seven standard-library runner tests passed without a GPU (0.565s), including
an actual owned-child timeout/reap case, balanced order, failure stop, status
classification, evidence retention and missing/partial adapter coverage. Format
and whitespace checks passed. Rust compilation and all GPU observations remain
pending; these policy tests are not crash reproduction evidence.

Source checkpoint and root worker-coordinator review before build/actual GPU use.
Slot two is granted for guarded lane121 compile-only checks with four jobs;
actual GPU work is held until issue349's full Lua window is released. First run
is the bounded matrix without artificial load, followed by required gates in
coordinated windows. No renderer crash has been reproduced or hidden in this task.
No own-PR merge or remote-machine action is permitted.
