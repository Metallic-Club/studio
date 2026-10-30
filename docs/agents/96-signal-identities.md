# #96 Globally unique signal connection identities

Goal: an opaque connection ID can disconnect only the handler that issued it,
including across argument types and after a signal is dropped and recreated.

## Audit and plan

- Fresh issue check: #96 ready, unclaimed, no blocking dependencies. Branch
  `studio/96-signal-identities`, latest-main base `fd164c2`.
- Remaining ready P2 #99 is not duplicated: PR #112 merged on 2026-10-30,
  removing the flattened-property Vec and unused signal-key lookup. PR #185
  remains open with borrowed property reads/cached defaults for the remaining path.
  This does not change #99's benchmark acceptance or claim #185 is merged.
- Current Signal stores a per-instance u64 next counter starting at zero. Same-type,
  cross-type and dropped/recreated signals can therefore collide on disconnect.
- Design reviewed by the root worker coordinator before implementation: one module-global
  AtomicU64 shared by all Signal argument types, relaxed checked atomic update,
  panic before exhausted-counter reuse or handler insertion. Keep ConnectionId(u64)
  opaque, preserve the local ordered handler vector and unknown-ID no-op behavior.
- No public API expansion or dependencies. Counter uniqueness does not make an
  individual Signal concurrently callable; each instance still requires exclusive access.
- Verified primary [Rust 1.99.1 atomic documentation](https://doc.rust-lang.org/std/sync/atomic/struct.Atomic.html):
  `fetch_update` is deprecated since 1.99 and renamed `try_update` (stable since 1.95).
  Use `try_update` with checked_add; a None result leaves the counter unchanged.
  Relaxed ordering provides identity allocation without publishing handler storage.

## Tests and next

- Tests written first: same-type foreign disconnect, actual World property/child
  signals with different argument types, drop/recreate, 1,024 concurrent independent
  connections across two argument types, and local firing/disconnection order.
- Baseline focused tests reproduced four failures; local firing order already passed.
  The concurrent case issued only 128 distinct IDs for 1,024 connections. The guard
  cleaned the prior #169 workspace artifacts before the new source identity compiled.
- Implemented the shared checked allocator and an isolated-counter boundary test.
- `test -p metallic_core -j 4 -- --test-threads=1`: all 29 focused tests passed,
  including 1,024 unique concurrent IDs and repeated exhausted-counter failures.
- Independent root-worker review found no blocking source/test issues; this is a
  worker review, not repository acceptance or approval to merge.
  Run repository gates with guarded lane121, coordinating builds.
- Update module docs/architecture, open and attach a reviewable PR, inspect CI.
  Do not merge this session's PR.
