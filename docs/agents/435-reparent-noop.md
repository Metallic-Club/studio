# 435: same-parent Explorer drops preserve undo/redo

Goal: omit same-parent Reparent placements after top-level selection filtering,
preserving World order, prior undo/redo, parked ids and saved cleanliness. This
child addresses only the Explorer paragraph of #396; GPU work remains separate.

Branch `studio/435-reparent-noop`, exact fresh main
`8699192791825af0c8c44edb9d833339440cc508`. Live issue/main checks found the task
ready and unclaimed. No editor.rs or visual design change is in scope.

First checkpoint: six new public EditorState/History regressions, production
unchanged. They cover a saved no-op, one Undo reaching the prior real edit, redo's
parked id, mixed moves, ancestor suppression and rejected-operation controls.
Actual baseline execution is pending; source-derived failures are not yet called
reproduced. Do not change production before recording that run.

Use released guarded lane124 and four jobs after the coordinator/core handoff.
First command is the Studio commands integration target. Full gates follow a
focused green run, independent review and disk report. No own merge or force push.
