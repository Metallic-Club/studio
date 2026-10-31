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

Issue #349's actual window is released; the first bounded six-child GPU matrix is
granted after this Python-only checkpoint. Rust sources remain exactly b68455f;
no new Rust build is required. Record source/toolchain/executable provenance before
execution, preserve every observation, then run the remaining required gates in
coordinated windows. No renderer crash has been reproduced or hidden in this task.
No own-PR merge or remote-machine action is permitted.

## Actual observations and final gates

Final runner checkpoint `1126893` preserves the reviewed/prebuilt Rust render
tree from `b68455f`; its extra Python-only append keeps Git metadata probes
windowless as well. Three adapter-independent Rust policy tests passed. The first
matrix at `1126893` executed all six fresh children in order 1,4,4,1,1,4. Every
child returned raw 0 / signed 0 / hex 0x00000000; each recorded four ready GPU
contexts, no adapter skips, no timeout and unchanged post-child executable hashes
(24 actual case executions). All adapter records identify Vulkan / NVIDIA GeForce
RTX 3060 Ti / driver 610.88. This is different hardware from #276's AMD RX 6650 XT
report. No artificial load was introduced; this is bounded non-reproduction,
not a fix, root cause, performance measurement or proof of general stability.

Evidence is retained outside git at `integration-evidence/353-gpu-diagnostics`:
`first-matrix-provenance.json`, `first-matrix/results.json`, all per-child stdout/
stderr, policy output and `first-matrix-evidence-hashes.json`. Executable SHA256:
`456a170bc3337fd4dbcf3227650d94343b0ad4402e58edb38a34c39f97d8de08`.
Provenance includes source/Rust-tree identity, rustc/Cargo, build-log and guarded
lane-owner fingerprints. Gate logs and hashes are also copied into its `gates/`.

At source `8c3bf0a` (CI-only append; Rust/runner unchanged):

- Full workspace no-run passed in 1m56s; strict all-target workspace Clippy passed
  in 57.83s, using guarded lane121 and four build jobs.
- Full serial workspace suite exited zero: 1,573 passed, zero failed, seven
  upstream ignores across 121 result groups including doc tests. Render groups:
  four headless, three policy, four lighting and seven post-chain cases passed.
  All three Parkour routes passed. No test or case was newly ignored by this task.
- Nine standard-library runner tests passed; both Windows and POSIX CI sections
  now run them. License, format and task-range whitespace checks passed.
- The root worker coordinator's Rust/Python reviews found no remaining actionable
  concern after the reproduced/corrected runner findings above.

The seven upstream ignores are three window-opening tests, two manual getter
probes, a manual real-budget pattern probe and a Studio actions doc example.
Actual Lua/GPU work used coordinated exclusive windows. The source remains pinned
at `c914767`; object-only merging with later main `238c6fb` is clean (tree
`f35dfaad`), which is source compatibility evidence, not an execution of that tree.
The publication append only records evidence after these gates. Exact-head CI
is inspected after publication. #276 stays open and unresolved.
