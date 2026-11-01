# 435: same-parent Explorer drops preserve undo/redo

Goal: omit same-parent Reparent placements after top-level selection filtering,
preserving World order, prior undo/redo, parked ids and saved cleanliness. This
child addresses only the Explorer paragraph of #396; GPU work remains separate.

Branch `studio/435-reparent-noop`, exact fresh main
`8699192791825af0c8c44edb9d833339440cc508`. Live issue/main checks found the task
ready and unclaimed. No editor.rs or visual design change is in scope.

Test-only checkpoint `2c46b121a8ea38adba2cb73e3bb61ed0ba9ae574` retained the exact
main commands implementation. Its actual commands-target run compiled and exited
101: 19 passed, five failed, no ignores. All five failures were new public API
regressions: extra saved-no-op history, one Undo consumed before the prior edit,
lost redo, mixed recorded ids and an ancestor-suppressed no-op entry. Existing
18 cases and the new rejected-operation/prior-history control passed. The red
log, source blobs and before-execution manifest were saved before production edits.

The fix adds one same-parent condition to the initial placement filter, after
top_level ancestor suppression. No undo/take_last compensation, editor.rs change,
generic history rewrite or prepopulated replay behavior change is introduced.
The six test bodies are unchanged from the baseline. Focused green execution at
`690853fc9f62263ba4d18e5022740bdad3f115e6` passed all 24 command cases, with no
failures or ignores. Independent worker-coordinator and backend source reviews
found no actionable issue in filtering order, caller Nothing behavior, mixed
moves, error rollback or the public regression oracles.

Use released guarded lane124 and four jobs after the coordinator/core handoff.
The first command was the Studio commands integration target; full gates followed
the focused green result, independent review and disk report.

## Required gates

Frozen tested source: `690853fc9f62263ba4d18e5022740bdad3f115e6`, tree
`58d81baf30324784eac70029814d3b92ccdfed38`, on the exact main pin above.

- Guarded locked workspace prebuild passed.
- Full locked workspace execution with serial harnesses: 2,021 passed, zero
  failures, eight upstream ignores across 147 result groups including doc tests.
  All six new case lines and all three complete Parkour routes passed.
- Strict full all-target Clippy passed in 35.16s. License, workspace format and
  whitespace against the pinned base passed. No dependency or manifest changed.
- No retry, new ignore, budget change or assertion relaxation was required.
  The actual red log remains separate from the corrected runs; the red and green
  command-test blobs are identical.

Raw logs, wrapper exits, source/blob fingerprints and before-execution manifests
are retained under external `integration-evidence/435-reparent-noop`. The final
publication child changes only this progress note; production and tests remain
the executed/reviewed source. Exact-head CI is next. Lane124 first-party dev
artifacts can be released after publication/proof retention under the explicit
coordinator authorization; no performance-binary retention is needed.

No editor.rs or visual change, own merge or force push was performed. This child
does not close the broader GPU diagnostics in #396.
