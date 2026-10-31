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

## Version-2 main compatibility refresh (published)

Normal merged cb26593, then exact d1eecce. The latter really introduces boxed Value::CFrame and PropDef.hidden
through the pivot APIs. Earlier abef7fc CFrame math alone did not do so; those source observations are distinct.
The prior 1,004-test/CI evidence above remains historical and is not relabelled as this new-base gate.

The version-2 fingerprint encodes all 12 row-major CFrame components as little-endian numeric bits and includes
hidden bookkeeping metadata. Hidden is UI/Luau presentation metadata, not confidentiality: eligible bookkeeping
must still reach a play replica. Repl, server container/source and owner visibility remain the disclosure policy.
Both class constructors retain the independent Repl and hidden fields; default and enum constructors are preserved.
Independent Python SHA-256 fixtures, all-component/signed-zero sensitivity and owned outgoing CFrame/hidden-state
regressions were added. Independent source review found the encoding/constructor/policy logic coherent; it caught
and prompted correction of an ambiguous documentation phrase. Fresh checks on merge 3cf9632 / main d1eecce passed: 64 core tests; 1,236 workspace tests, zero failures,
six existing manual ignores; strict full workspace/all-target Clippy (4m19s); licenses, format and whitespace.
The full workspace prebuild took 11m37s in the owned guarded lane123. Two independent example processes printed
c422e66b55f8ac4e354427bdfa7f7034110f65747fc855a15f8449dd7bbffaeb. This v2 value supersedes the historical
v1 diagnostic above. Logs: <TEMP>/metallic-293-v2-{core,workspace-build,workspace-tests,clippy,licenses}.log.
Normal append-only push and exact-head CI/review follow; no own PR merge.

## Lighting registry integration (version 3, tested)

The latest normal merge pins b4909b5, whose lighting work adds NumRange and six registered classes. Both normal
and hidden constructors now retain range, hidden and Repl independently. Existing main class indices0..33 stay
intact; the five replication-only classes append at 34..38. Published-preview indices are valid only under an
equal fingerprint and must not be reused across these schema changes.

Encoding v3 includes range presence, min/max f64 bits and clamp-versus-wrap, in addition to v2's hidden/CFrame
fields. The independently recomputed Bool fixture is f99a35223a1f029c7fb6a434291443c692c5e3fde105abbb814b322a76bd4809.
Tests cover range presence/bounds/wrap/signed-zero, existing lighting indices and authoritative ClockTime wrapping.
The earlier v2 checks above remain historical. Fresh gates at 563b2c7820f9996b571b8d8d036eca9fb15c02ad,
pinning exact main b4909b582241cd1f5c946aa69a7e91021a8271d4, passed in guarded lane123:

- Core: 78 passed, zero failed or ignored.
- Full workspace with serial test harnesses: 1,458 passed, zero failed, six existing manual ignores (116 groups).
- Strict full-workspace/all-target Clippy: passed; licenses, formatting and task-diff whitespace checks passed.
- Two separate example processes printed the v3 registry hash
  1b20a2fddd67fb759af608ce847b5e76d699214a8a787b8eb67436f22b95197e.
  Lane ownership identifies this worktree, and the executable came from the fresh workspace build.
- Independent source review checked the preserved main class prefix, inherited numeric ranges, all constructors
  and the v3 hash encoding, and independently recomputed the fixture. No actionable findings.

Logs: <TEMP>/metallic-293-v3-{core,workspace-build,workspace-tests,clippy,licenses}.log.
Only this evidence describes the v3 lighting integration. Append-only publication and exact-head CI follow;
the owner/lead remains responsible for merging.

## Character limits and Studio integration refresh

Main 060f1c2 required another normal merge because character safety limits add constants beside Repl. The
resolution retains both declarations and all inherited numeric ranges. The fingerprint encoding remains v3;
the actual schema hash changes because Humanoid tuning now has bounds. Source ccd7b721fbb4bc23773448a96036895ec77e5f95
passed a fresh workspace no-run build (1m58s), strict all-target Clippy (41.58s), licenses, format and whitespace.
Two separate rebuilt example processes printed
37ca80ce29f36fbecaee50550c68b3c242e9f3f4e3a903094957c7d3ee0b9839.
The full actual workspace suite passed: 1,534 passed, zero failed, six existing manual tests and one upstream
Studio documentation example ignored,
across 120 groups. These results include the newly merged Studio editing and character-limit tests; the earlier
1,458-test result above belongs to b4909b5. The actual suite ran alone in the shared Lua test window.
Logs for this refresh use <TEMP>/metallic-293-current-{workspace-build,workspace-tests,clippy,licenses}.log.

## Final test-isolation compatibility refresh

GitHub identified one further real conflict against exact main d34c50fcfd9b3e768013526ea7b8ea27bb3e25e9:
adjacent Cargo.lock entries for this task's existing sha2 dependency and upstream's tempfile test dependency.
The normal merge at f7871419f988b10e444418ebbdeaca67b737e7e9 preserves both. Core policy and hash encoding
are unchanged; upstream's test-folder isolation, out-of-world spawn filtering and touch/inset changes are retained.

Fresh guarded lane123 results at that frozen source: full workspace 1,544 passed, zero failed, seven upstream
ignores across 120 groups; workspace no-run build 1m44s; strict full-workspace/all-target Clippy 22.36s; licenses,
format and task-range whitespace passed. The actual suite ran alone in the shared Lua test window. Two separate
rebuilt registry-example processes again returned
37ca80ce29f36fbecaee50550c68b3c242e9f3f4e3a903094957c7d3ee0b9839.
Logs: <TEMP>/metallic-293-final-{workspace-build,workspace-tests,clippy,licenses}.log.
Later commits only record evidence. Publish append-only, monitor exact-head CI and leave merging to the owner/lead.

## GUI and normalization compatibility refresh

Normal merge/source checkpoint `52a187ee192cab2b39640c47a7120a11639223c3` integrates exact main
`a0bb1bfc0e84ffd637fe8d1759f4822322601cd8`. The upstream GUI prefix stays intact (GuiButton 34,
TextButton 35, ImageButton 36); this branch's five additional classes follow at 37..41. Upstream lighting,
exposure ranges and LookOverride normalization are preserved.

Fingerprint encoding is now v4. Each normalizer has a nonempty declared `normalize_schema_id`; the hash
encodes that identifier and presence without executing callbacks or hashing addresses. Missing, orphaned
and empty identifiers fail closed. This is an authored semantic contract, not proof of executable behavior;
normalization rule/alias changes must bump the identifier. LookOverride declares `look-override/ascii-alnum-v1`.
The independent Python fixture first verifies the historical v3 golden and then computes the new v4 golden
`62f765561636489bab87910b221e0adca58a3c9d0bc65f1ce29b7c75fdf80e9a`.

Core tests passed at 52a187e: 102 passed, zero failures or ignores across 13 groups in guarded lane123.
Source-only peer review found no actionable concern in v4
normalizer encoding or GUI/class index preservation. The actual workspace gate remains pending. No earlier result is attributed to
this refresh. Evidence logs are retained outside Git under `integration-evidence/305-v4-*`.

The first workspace prebuild exited 101 before actual execution when the system drive reached zero free bytes.
LLVM reported `no space on device`; the linker also failed writing a PDB. The complete failed log is preserved
as `305-v4-prebuild.log`. This is not a test result. All managed builds were held. Package-scoped Cargo cleanup
of known rebuildable first-party outputs began to recover capacity; source, raw logs and named benchmark
domains remain intact. A new full prebuild and the remaining gates are required before publication.

After native Cargo cleanup recovered capacity, the unchanged-source workspace prebuild passed in 2m53s and
strict full-workspace/all-target Clippy passed in 31.56s. Licenses, formatting and task-range whitespace checks
passed. Two guarded example invocations produced the same current registry hash:
`6699764a6ca5e949b9070b4f21f11dd23848220de244f1eb52eac44366697392`.
The independent fixture was also rerun successfully. Logs use `305-v4-prebuild-recovery.log`,
`305-v4-clippy.log`, `305-v4-licenses.log` and `305-v4-hash-{one,two}.log`.
The full actual workspace command will restore the full workspace feature selection after the example runs;
no cached executable is invoked directly. Publication remains held until that gate completes.

The complete serial workspace command then passed at unchanged executable source 52a187e: **1,866 passed,
zero failed, eight upstream ignores across 138 result groups**, including doc tests. It used the normal guarded
Cargo workspace invocation with `--locked -j 4 -- --test-threads=1`; the raw log is
`305-v4-workspace-tests.log`. The suite ran alone in the managed actual Lua/GPU window, which was released
immediately after verified exit zero. This branch adds no ignored test. The earlier disk-full prebuild remains
recorded separately and was not an actual test failure.

All local publication gates are now complete. The final append records evidence only; executable source stays
identical to 52a187e. Normal push, current-source PR body and exact-head CI follow. The owner/lead retains merge
authority. No live multiplayer, Team Create or broader milestone acceptance is claimed.
