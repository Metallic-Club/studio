# Issue 293: core replication schema and play-client visibility

Status: implemented and tested; full workspace, strict Clippy, license and source-review gates passed; ready for review.
Branch: studio/293-replication-registry; initial base fbaa17d, rebased onto main 7c238f8 before expanded gates, then ebf359a (release Python files only).

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
- Outgoing properties are owned copies; properties excluded by replication/instance visibility return None and invisible/dead references become Ref(None).
  Authoritative values remain intact. An example prints only the registry hash for peer diagnostics.

## Validation plan and evidence

- Guarded final core suite: 54 passed, zero failed. Log: <TEMP>/metallic-293-core-final.log.
- The registry example was built in guarded lane122 and run as two separate processes; both printed
  fe04c70d4e3b4760ca8950572985b7fd5a2dbf2b8a476c3dcc5780c396da5607.
- Independent source review by engine_research at 458e118 found no actionable findings. It checked hash field
  boundaries, inherited/default metadata and service/viewer/source/reference privacy; this was source review only.
- Guarded cargo test --workspace -j 4 -- --test-threads=1: 1004 passed, zero failed, six baseline manual tests ignored.
  Three ignored tests open windows; three are manual allocation/timing probes. Log: <TEMP>/metallic-293-workspace-final.log.
- Guarded cargo clippy --workspace --all-targets -j 4 -- -D warnings: passed. The first attempt caught new registry
  items placed after an existing test module; moving them before the module resolved it, with no policy change.
  Final log: <TEMP>/metallic-293-clippy-final.log. The subsequent full test suite uses this final ordering.
- cargo deny check licenses: licenses ok; cargo fmt --all -- --check and git diff --check passed.
- Full workspace no-run prebuild also passed in 8m57s; provenance-guarded lane122 was used for all Rust gates.
- The final release-only rebase retained the exact tested crates tree ef2c2a5d672e43baf5952d254734209ea033fe7d.
  The updated Python release helper suite passed all 13 tests; format/whitespace checks passed again.
- The single new direct dependency is sha2 0.10.9, already in the workspace lockfile, MIT/Apache-2.0.

Root owns classes.rs/replication.rs and only the ServerStorage SERVICES/doc lines in instance.rs. The journal and
edit-kernel workers own other locations. Future composition must retain all changes; no other branch is modified.
No running backend/server changes, credentials, public exposure, capacity guarantee or own PR merge.

## Next

Publish one PR and move this issue to review; monitor CI and address review findings. Owner/lead merges only.
After all four foundation branches are ready, validate a private composition before planning a live-play claim.

## Current main compatibility refresh (not yet pushed)

Normal merged cb26593, then exact d1eecce. The latter really introduces boxed Value::CFrame and PropDef.hidden
through the pivot APIs. Earlier abef7fc CFrame math alone did not do so; those source observations are distinct.
The prior 1,004-test/CI evidence above remains historical and is not relabelled as this new-base gate.

The version-2 fingerprint encodes all 12 row-major CFrame components as little-endian numeric bits and includes
hidden bookkeeping metadata. Hidden is UI/Luau presentation metadata, not confidentiality: eligible bookkeeping
must still reach a play replica. Repl, server container/source and owner visibility remain the disclosure policy.
Both class constructors retain the independent Repl and hidden fields; default and enum constructors are preserved.
Independent Python SHA-256 fixtures, all-component/signed-zero sensitivity and owned outgoing CFrame/hidden-state
regressions were added. Independent source review found the encoding/constructor/policy logic coherent; it caught
and prompted correction of an ambiguous documentation phrase. Fresh core/workspace/Clippy gates remain pending.
