# #371 item 1: TouchEnded between ordinary parts

Branch `studio/371-touch-ended`, built on #370 (`studio/267-bone-touch-volumes`), which adds
`ScriptHost::touch_ended` and the `BasePart.TouchEnded` event for character parts. Rebase onto main
when #370 merges. Item 2 (limb against limb) waits for #370.

## Done
- `TouchTracker` (`crates/metallic_physics/src/touched.rs`) queues a stop only for a pair that was
  touching (`Touched` reported), so each `Touched` has at most one `TouchEnded`. `forget` drops a
  removed part's queued stops.
- `PhysicsWorld::drain_touch_ended()`; `Game::feed_events` calls `scripts.touch_ended(a, b)` and
  `(b, a)` after the `Touched` calls of the same frame.
- Tests: `crates/metallic_physics/tests/physics.rs` (`touch_ended_fires_once_per_separation`,
  `removed_parts_do_not_report_touch_ended`), `crates/metallic_runtime/tests/touch_ended.rs`
  (two parts touch and separate: each fires `Touched` once and `TouchEnded` once).

## Not done
- Item 2, limb-vs-limb touches between characters.
