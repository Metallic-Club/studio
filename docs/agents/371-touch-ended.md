# #371 item 1: TouchEnded between ordinary parts

Started by the script agent in #381 (commit 6881eb6e), which was stacked on the pre-squash #370 branch. Taken
over as a fresh branch from main (`studio/371-touch-ended-parts`) with that commit cherry-picked, plus the review
fixes below. Item 2 (limb against limb) is `studio/371-limb-touches`.

## Done
- `TouchTracker` (`crates/metallic_physics/src/touched.rs`) queues a stop only for a pair that was touching
  (`Touched` reported), so each `Touched` has at most one `TouchEnded`.
- `PhysicsWorld::drain_touch_ended()`; `Game::feed_events` calls `scripts.touch_ended(a, b)` and `(b, a)` after the
  `Touched` calls of the same frame. The host fires on the part that still exists (#370).
- Review fixes:
  - `forget` (a collider removed) now ends the part's pairs instead of dropping them. A part destroyed mid-contact
    fires `TouchEnded` on the other part, matching the character rule of #370 and #397.
  - A part rebuilt after a `Size`, `Shape` or `CanCollide` change used to fire a second `Touched` with no
    `TouchEnded`. Its `forget` stop is now cancelled when the rebuilt collider starts touching again in the next
    contact pass, so nothing fires while it stays in contact; if it no longer touches, one `TouchEnded` fires.
  - A pair that separates and meets again before anyone drains also cancels its stop, so `Touched` never arrives
    before the `TouchEnded` of the same pair.
- Tests:
  - `crates/metallic_physics/tests/physics.rs`: one `TouchEnded` per separation; a destroyed part ends its touch
    once; rebuilding a touching part (Size, Shape, CanCollide) fires nothing; a rebuild that separates ends the touch
    once; no `TouchEnded` without a `Touched`; a start and a stop between drains reports both.
  - `crates/metallic_runtime/tests/touch_ended.rs`: two parts touch and separate (each fires `Touched` once and
    `TouchEnded` once); destroying a touching part fires `TouchEnded` once on the other.

## Known limits
- Two anchored parts that do not move never pair in Rapier (no fixed-fixed contacts), so they fire neither event.
  The tests move an anchored part in, or use an unanchored one.
- Rebuilding a resting unanchored part's collider (a `Size` or `Shape` change) can trip a debug-only Rapier island
  assertion ("touching pair not linked in the persistent islands"). It predates this change; the rebuild tests use an
  anchored zone. Tracked as a separate follow-up.
