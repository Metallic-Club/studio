# 429: startup character respawn

Base494e8a5. Reused the independently reviewed23-line correction and four public regressions.
Capture already-spawned server Player IDs before eager scripts; seed timers only after successful startup.
Combined and never-spawned behavior is retained. No World borrow spans script execution.

Guarded lane124 CPU validation at fe8c679: ten passed, zero failed (six simulation, four regressions).
Raw log: external419-startup-review/main-followup-cpu.log. Original isolated red proof remains preserved.
Full workspace, strict all-target Clippy and license gates remain before opening a PR; the user requested a1% pass.
Next: finish those gates and publish the issue-specific PR. No merge or deployment claim.

## Completed validation

Refreshed published history with main b68193f by normal merge. Tested source9bddb73eb75487e24d2b854f49b78452562d6295.
Guarded lane124, locked Cargo, four jobs and serial test harnesses:
- Full workspace:2112 passed,0 failed,9 existing ignores,159 result groups.
- Strict full all-target Clippy:exit0.
- Licenses, format and pinned-main whitespace:exit0.
The source correction and four controls retain their previous independent proposal review. No visual changes or new dependencies.
Raw logs/results: external integration-evidence/429-startup-respawn. Next: PR review and exact-head CI; no own merge.
