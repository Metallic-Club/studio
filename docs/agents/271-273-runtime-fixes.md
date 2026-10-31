# 271 + 273 Player pacing and runtime fixes

Goal: issues #271 (a minimised Player spins a CPU core) and #273 (small runtime items from the #184 review).

## Done
- #271: `crates/metallic_player/src/pacing.rs` decides the loop's pace from the window state. Focused: `Poll`, vsync
  paces it. Unfocused but visible: 30 frames a second (`WaitUntil`); the game still runs its 60 Hz ticks. Minimised or
  occluded: `Wait`, no redraw requests, the game paused, and the first frame back starts with a fresh frame time.
  Automated runs (`--fps`, `--frames`, `--capture`, `--autopilot`, `--input-script`) are never throttled.
  `--minimised` starts the window minimised for this measurement.
- #273 restart: `Game::start` after `stop` removes the StarterGui copies of the last start before copying again (GUI
  that scripts made at run time stays).
- #273 destroyed character: a local character Model that is destroyed, taken out of the Workspace or missing its
  `HumanoidRootPart` is replaced after `RESPAWN_DELAY` (5 s), with `CharacterAdded`.
- #273 mover lag: each tick steps physics before the character controller, so the character meets movers where they
  are drawn.
- #273 banner overflow: GUI text that does not fit its label shrinks to fit (never below 8 pt). Engine-side only; the
  Parkour banner itself is unchanged because #333 moves it for touch.
- #273 docs: `m1-build.md` now says egui is built with `default_fonts`.

## Measured
Debug build, `metallic-player templates/parkour --minimised --no-overrides`, CPU time over 15 s after a 5 s warm-up
(PowerShell `TotalProcessorTime`): before 12.72 s (84.7% of a core), after 0.08 s (0.5%).

## Deferred
- The banner says "Oops! Back to your checkpoint..." before any checkpoint is reached: that text lives in the
  Parkour HUD script and scene that #333 is changing; left to it.
