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

Rust source checkpoint b68455f compiled all render test targets without running
them in 1m08s via guarded lane121 with four build jobs. The root worker coordinator
reviewed the wrappers without an actionable Rust concern. Its Python review found
two gaps: Windows child creation flags and a missing post-child binary hash.

The new final-trial-mutation regression first failed against the proposed runner
(returned zero instead of one, 0.022s); this is a tool-policy red, not a renderer
crash. After correction, all nine runner tests passed in 0.641s, including a
combined child-failure/image-change case preserving the original failure/output.
Children now use CREATE_NO_WINDOW on Windows. Post-child hashes are recorded even
on the last trial; an unreadable/changed image cannot produce no_crash_observed.
Logs are `metallic-353-last-mutation-red.log` and
`metallic-353-runner-green.log` in the local temporary directory.

Issue349's actual window is released; the first bounded six-child GPU matrix is
granted after this Python-only checkpoint. Rust sources remain exactly b68455f;
no new Rust build is required. Record source/toolchain/executable provenance before
execution, preserve every observation, then run the remaining required gates in
coordinated windows. No renderer crash has been reproduced or hidden in this task.
No own-PR merge or remote-machine action is permitted.
