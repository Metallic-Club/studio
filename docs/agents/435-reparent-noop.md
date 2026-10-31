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
The six test bodies are unchanged from the baseline. Focused green execution and
independent review are next; no fixed result is claimed yet.

Use released guarded lane124 and four jobs after the coordinator/core handoff.
First command is the Studio commands integration target. Full gates follow a
focused green run, independent review and disk report. No own merge or force push.
