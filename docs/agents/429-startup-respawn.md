# 429: startup character respawn

Base494e8a5. Reused the independently reviewed23-line correction and four public regressions.
Capture already-spawned server Player IDs before eager scripts; seed timers only after successful startup.
Combined and never-spawned behavior is retained. No World borrow spans script execution.

Guarded lane124 CPU validation at fe8c679: ten passed, zero failed (six simulation, four regressions).
Raw log: external419-startup-review/main-followup-cpu.log. Original isolated red proof remains preserved.
Full workspace, strict all-target Clippy and license gates remain before opening a PR; the user requested a1% pass.
Next: finish those gates and publish the issue-specific PR. No merge or deployment claim.
