# Issue 301: authoritative edit-command kernel

## Goal and scope

Implement the in-memory command-kernel prerequisite for M2 C2 in `metallic_core::edit`, shared by a future Studio
document and edit server. This does not provide Team Create sessions, networking, persistent logs, authentication,
script leases, optimistic editing, undo, presence or saving. M1/Epic 15 acceptance is unchanged.

Branch: `studio/301-edit-kernel`. Initial base: `16ce1d7634690c90f2903d1d5aff1c2bb178eb19`.
The preceding `fbaa17d` core source is identical; the base advance adds the M2a plan.

## Implementation

- `EditAuthority` exclusively owns the original `World`. Typed Create/Set/Move/Destroy/ScriptText batches first
  execute on `World::deep_clone()` with independently empty signals, then replay into that original World.
  No candidate World, signal or instance handle replaces live state. Staging is O(world) per request and keeps
  existing World validation/default/no-op semantics; this is an edit-only foundation, not a scaling claim.
- Trusted host calls grant Owner/Edit access and whole-script write ownership. Play/None cannot submit. Every
  Source write uses the same ownership check, including Create properties and Set; Source capability is detected
  through its registry property, supporting classes such as ModuleScript without requiring Script inheritance.
  New scripts belong to their creator. Existing scripts initially have no write owner until assigned by the host.
- Object identities are `(EditorId, creation_counter)`. Initial objects use editor zero in ordered depth-first tree
  order. A positive creator high-water counter survives destruction, so accepted object IDs cannot be reused.
  Batch client sequences start at one, advance only on acceptance, and may be corrected/retried after rejection.
  Every acceptance receives one global server sequence. Exact cached repeats acknowledge without rebroadcast;
  changed payload reuse and evicted old requests reject. Numeric equality includes the sign of zero.
- Different property writes compose; server order selects the last writer for the same property. Deleted targets,
  deleted parents, cycles, absent/self/wrong-parent sibling anchors reject the whole batch. `after: None` appends.
  Builtin services cannot be created, renamed, moved or destroyed, including indirectly through a containing
  subtree. Other registered service settings remain editable; the regression uses Workspace's fallen-parts height.
- `EditReplica` consumes a trusted sequential accepted log, resolves stable references into its own local slots,
  rejects gaps/changed repeats, and treats an exact last-record repeat as a no-op. It does not authenticate records
  or perform optimistic reconciliation. `EditSnapshot` includes effective values and ordered roots/children;
  future initial/resync adapters must additionally preserve the stable identity association.
- Limits bound live bindings, lifetime editor metadata, operation count, accounted request bytes, Source UTF-8
  bytes and retained exact request/receipt metadata. Byte accounting includes native owned metadata; it is not a
  cross-platform wire size. A future transport must bound decoding before allocating these DTOs.

## Atomicity boundary and initial invariants

The initial World must have the same ordered semantic tree on all replicas; differing local allocation histories
are supported. Edit ownership prevents concurrent or out-of-band mutable access. Do not run scripts or physics
against this World while using the kernel. Reads and `into_world` preserve the original World's identity/signals.

Ordinary input/access/schema failures occur during preflight and leave live values, tree order, journal and original
subscriptions unchanged. Accepted operations retain existing per-operation callbacks; a batch is not a new signal
transaction. Anchor ordering follows the existing create/reparent callback, rather than changing that callback API.
The accepted log/snapshot carries final order. No claims of callback-panic rollback are made.

Unexpected replay invariant errors produce `ReplayFault`, no accepted receipt and terminal poison; a caught callback
panic also leaves poison set. Partial live effects may already exist. Subsequent submits fail closed pending a host
reload/resync, rather than misreporting an atomic rejection. Tests deliberately inject a private discrepancy to
exercise this otherwise-unreachable boundary.

## Research and coordination

Read M2 design 6.4/C2, the networking research, existing Studio shell/modeling commands and World implementation.
`World::deep_clone` directly copies slots/free-list/tree and installs independent empty signals. Issue 92/PR 275
concern recursive `clone_subtree`, not this staging path; that unmerged change was not copied or required.
The root approved this atomicity approach, strict anchor rejection and explicit poison policy. A1 owns the typed
journal work and A2 owns registry services. Only the agreed `pub mod edit;` declaration in instance.rs and separate
lib.rs re-export overlap; this module otherwise has its own files and adds no dependencies.

## Verification checkpoint

- Guarded lane116 `cargo test -p metallic_core --test edit_kernel --no-run -j 4`: passed.
- Guarded lane116 `cargo test -p metallic_core -j 4 -- --test-threads=1`: 52 passed, zero failures/ignored.
  Includes seven new integration tests, 5,000 seeded two-editor requests with both accepted/rejected conflicts,
  full semantic equality plus a test-build hash across differently allocated replicas, and terminal poison testing.
- Three additional boundary regressions are written after that checkpoint; not executed yet.
- Full workspace tests, strict all-target Clippy and license gate: pending coordinated build slot.

## Next

Run the added boundary tests, obtain independent source review, and run all mandatory workspace gates in an isolated
guarded lane. Update this note with exact results before publishing a PR. No own merge or acceptance claim.
