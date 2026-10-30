# Issue 293: core replication schema and play-client visibility

Status: implemented; initial core checks passed; final gates pending.
Branch: studio/293-replication-registry, from main fbaa17d.

## Scope

Independent M2 A2 foundation authorized by the owner's multiplayer/Team Create request. This does not accept M1,
unblock Epic 15, provide a transport or create live multiplayer sessions. Existing combined VMs still share a World;
these filters must be used by the future replicator before any data is sent.

## Implemented

- Add PropDef.repl and default Replicated eligibility. Play-client filtering also checks container ancestry, script
  context and the trusted viewer Player. Source of a server Script is never eligible; LocalScript and ModuleScript
  source stays eligible only in a visible subtree. This is not the edit-session snapshot policy.
- Register core-only ServerStorage, ModuleScript, RemoteFunction, UnreliableRemoteEvent and PlayerScripts. Only
  ServerStorage becomes a standard built-in root. Luau require/new remote method implementations remain separate.
- Keep existing class indices by appending definitions. Class-relative property indices use inherited declaration
  order and a one-time immutable lookup table; they are meaningful only under an equal registry fingerprint.
- SHA-256 fingerprint has an explicit versioned encoding: ordered class names, inheritance, effective property
  order, types, defaults, enum values and replication policy. No native memory layout or randomized map iteration.
- Visibility queries are uncached so reparenting changes the result immediately. Only trusted standard public
  roots are eligible. PlayerGui/PlayerScripts require the authenticated viewer's owner at every private boundary.
  The caller must derive that viewer from a trusted same-world connection roster; this library authenticates nobody.
- Outgoing properties are owned copies; hidden properties return None and invisible/dead references become Ref(None).
  Authoritative values remain intact. An example prints only the registry hash for peer diagnostics.

## Validation plan and evidence

The first guarded core suite passed before the final trusted-root/viewer hardening and additional schema tests.
Final focused tests, separate-process fingerprint checks, expanded workspace tests, strict Clippy, license, format
and whitespace gates remain pending. Initial log: <TEMP>/metallic-293-core.log. One additional direct dependency is
sha2 0.10.9, already present in the workspace lockfile; it is MIT/Apache-2.0 and the license gate will verify it.

Root owns classes.rs/replication.rs and only the ServerStorage SERVICES/doc lines in instance.rs. The journal and
edit-kernel workers own other locations. Future composition must retain all changes; no other branch is modified.
No running backend/server changes, credentials, public exposure, capacity guarantee or own PR merge.

## Next

Complete final gates in guarded lane122 after a team slot releases, record exact results and limits, obtain source
review, publish one PR and move this issue to review. Keep source/base evidence pinned if main advances.
