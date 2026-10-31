# Issue 301: authoritative edit-command kernel

## Goal and scope

Implement the in-memory command-kernel prerequisite for M2 C2 in `metallic_core::edit`, shared by a future Studio
document and edit server. This does not provide Team Create sessions, networking, persistent logs, authentication,
script leases, optimistic editing, undo, presence or saving. M1/Epic 15 acceptance is unchanged.

Branch: `studio/301-edit-kernel`. Initial base: `16ce1d7634690c90f2903d1d5aff1c2bb178eb19`.
The preceding `fbaa17d` core source is identical; the base advance adds the M2a plan.
The unpublished checkpoint rebased cleanly to `ebf359aa437d429023f4ef263a214ab1cba51c7e` before full gates;
that base advance contains only release Python changes. Its 13 release tests passed.

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
- Full workspace `cargo test --workspace --no-run -j 4`: passed at checkpoint `0454839` before the private helper fix.
- All 10 edit-kernel integration cases passed, including the additional service Name/cursor, stable-reference/replica
  sequence, numeric request bits, Source UTF-8 byte and revoked-duplicate boundary cases.
- Initial strict Clippy found the private property helper exceeded seven arguments. Stable/local target handles
  now travel together as a tuple; no public API/validation change or lint suppression. Afterward the complete core
  suite passed all 55 cases, and `cargo clippy --workspace --all-targets -j 4 -- -D warnings` passed (3m45s).
- `cargo fmt --all --check`, diff whitespace check and `cargo deny check licenses`: passed.
- Independent C2 and A1 compatibility source reviews found no additional actionable issue. Root's separate private
  four-foundation composition has its own pinned evidence; it is not this branch's full-workspace gate.
- Final guarded lane116 `cargo test --workspace -j 4 -- --test-threads=1` at `7c05e16`: **1,005 passed,
  zero failed, six existing manual probes ignored**. The command rebuilt necessary local artifacts first, then
  completed all runtime/Parkour, Lua and Studio tests in the coordinator-reserved timing window.
- The strict Clippy source is identical to this tested source; the remaining commit changes only this progress note.

## Next

Publish the reviewed, tested branch as the single PR for issue 301, attach it to the root task, and leave repository
CI/lead review visible. No own merge or acceptance claim. Networking, persistence and Studio client integration remain
separate multiplayer foundations.

## Normal merge of main after publication

Main advanced to `abef7fc` with the 0.1.1 template-packaging release and PR 304 CFrame math/Luau adapter. The
published branch is being refreshed with a normal merge, preserving its existing public history. The only conflicts
are additive core re-exports and adjacent architecture paragraphs; both APIs and both descriptions are retained.

Exact source inspection confirms CFrame is a standalone math type, not a Value/ValueType variant or registry
property. The Luau BasePart.CFrame adapter reads/writes registered Position and Orientation. EditValue's existing
exhaustive conversions, exact numeric equality and accounted size therefore need no invented CFrame variant or
escape hatch. A new regression applies CFrame-derived registered pose components through the kernel/replica and
proves that a later virtual CFrame property still rejects the complete batch without changing live state/cursor.

The earlier 1,005-test/Clippy/CI result at `4447974` is historical evidence for the old base. Fresh merged-source
focused/workspace/Clippy/license/format gates are pending; compilation is held for the coordinator's next slot.

Before those gates, main advanced further to exact `d1eeccee2c3d46ca61921baffa562af4bd9472e8`. PR 313 adds the
actual stored `Value::CFrame(Box<CFrame>)` and PivotOffset/WorldPivot properties; this differs from the earlier
standalone-only `abef7fc` source. A second normal merge is necessary before validation. The exhaustive edit value
conversion/snapshot paths now retain boxed CFrames, request equality compares all twelve floating-point bit patterns,
and limits include the separately owned CFrame payload, not only its Box pointer/native enum metadata.

Regressions cover stored-transform Create/serialization/replica replay alongside stable PrimaryPart references;
all twelve NaN/+infinity/-infinity components rejecting a late batch without values/cursor/signal changes; exact
request identity including signed zero; and boxed payload bytes enforcing both request rejection and history
eviction. The virtual BasePart.CFrame rejection case remains correct. Low-level Set continues delegating to
World::set, without adding hidden-property policy or high-level transform normalization/bookkeeping. All fresh
merged-source gates remain pending; no prior-base CI result is claimed for this update.

### Fresh verification on the stored-CFrame base

The fixed source is `60a8c9a43a01279c23b8e14271279708f5800039`, including exact main
`d1eeccee2c3d46ca61921baffa562af4bd9472e8`. The coordinator reviewed the boxed conversions, all-component bit
equality, owned payload accounting and unchanged hidden-field semantics without an actionable finding.

- Guarded lane116 `cargo test -p metallic_core -j 4 -- --test-threads=1`: **67 passed**, zero failed/ignored,
  including all **14 edit-kernel integration tests**.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed (1m44s), including canonical net/examples.
- `cargo deny check licenses`, `cargo fmt --all --check` and diff whitespace check: passed on that pinned source.
- Final guarded `cargo test --workspace -j 4 -- --test-threads=1`: **1,239 passed, zero failed, six existing manual
  probes ignored**, including all Lua/Parkour/network/Studio sections. The command performed the necessary rebuild
  and ran serially in the coordinator-reserved actual-test window after issue 311's suite exited.

The final evidence commit changes only this note. Published history is preserved through normal merges/pushes;
no rebase, force-push or own merge is used. Current-head GitHub CI is checked after pushing this refreshed input.
The earlier 1,005-test and green CI result at `4447974` remains historical old-base evidence, not this new-base proof.
