# Metallic engine systems: evidence and implementation priorities

Date: 2026-10-30. Status: **planned**, research proposal. Issue: #30.

This document compares seven engine ecosystems using their own technical documentation. It proposes changes to
Metallic's engineering backlog; it does not change the accepted M1 specification. No engine code was copied, no
engine dependency was added, and no performance comparison was executed. All sources were accessed on 2026-10-30.
The cited documentation URLs can change; the versions identified below describe what was read.

## Recommendation

Make the first complete creator loop reliable: create a project, edit it, Play, stop safely, save, reopen, then
publish an immutable version when platform integration arrives. Build measured rendering and simulation performance
around that loop. A larger feature inventory cannot establish that Metallic is better than another engine.

The strongest opportunity is a coherent creator experience: one world model, one typed API, reproducible projects,
useful errors, fast tests and built-in multiplayer validation. This is an engineering hypothesis, not a comparative
claim. Measure it with the creator tasks in section 8.

Priority order, preserving the existing dependency graph:

1. Preserve M1's Luau, physics and renderer contracts while closing tracked safety/correctness regressions.
2. Follow [#31](https://github.com/Metallic-Club/studio/issues/31) / [PR #45](https://github.com/Metallic-Club/studio/pull/45)
   for saver hardening; design persistent identity and any additional recovery-copy policy separately.
3. Add subsystem measurements while implementing runtime and Studio; keep the full profiler in #23.
4. Deliver the offline Parkour creation loop before platform services become required.
5. Make an authoritative headless server and multi-process local testing the entry gate for M2.
6. Introduce resource handles and streaming only with explicit lifetime and reference semantics.

## 1. What exists in this checkout

Audit baseline: `origin/main` commit `25955d8c43d050c54d39ac027b4ae65ca2cc31bf`.
The file observations in this section refer to that commit, rather than work in other sessions' branches.

| Area | Evidence in repository | Status at baseline |
|---|---|---|
| World | `crates/metallic_core/src/instance.rs`: arena slots, generational `InstanceId`, roots and children, property signals | Implemented; existing integration tests run |
| Classes and values | `crates/metallic_core/src/classes.rs` and `value.rs`: registry defaults and typed values | Implemented; existing tests run |
| Scene format | `crates/metallic_core/src/scene_file.rs`: RON, version 1, non-default properties, references as traversal indices | Implemented; round-trip and diff tests run |
| Other six crates | Renderer, physics, scripts, runtime and both apps contain scaffold entry points and build tests | Scaffold only on this baseline |
| Desired architecture | `ARCHITECTURE.md`, the M1 plan and design specification | Planned contracts; implementation varies by branch |

Baseline `cargo test --workspace -j 4` passed 22 tests: 16 core integration tests and six scaffold build tests.
Those tests do not demonstrate a playable game, graphical output, safe script execution or multiplayer. Open issues
and active PRs must be checked before assigning implementation; `docs/STATUS.md` still refers to the initial work.

Review update: while this document was prepared, physics PR #28 merged as
`ca1314e704891af4d837b7ccf0d8fb039600cd16`. The research branch was rebased and checks were repeated; results are
in the issue's progress note. The table above deliberately preserves the earlier audited snapshot.

Review amendment checked `origin/main` at `ebb94f3`: runtime/player and other M1 systems now have implementations.
The specific current-source observations in sections 4 and 6 are separate from the historical table above; merging
new main commits into this published research branch does not turn its original scaffold audit into a current one.

The existing [platform architecture document](2026-10-30-platform-architecture.md) covers product boundaries,
Launcher, asset service and matchmaker. This document supplies system-level evidence and acceptance criteria.
It does not resolve the console-layout question tracked in #29.

## 2. What the other engines demonstrate

These are observations from primary sources, followed by **proposed Metallic applications**. They are not feature
parity ratings, speed rankings or claims about undocumented internals.

| Ecosystem and documentation read | Sourced observation | Proposed Metallic application and tradeoff |
|---|---|---|
| Unreal Engine 5.8, [World Partition][S1] | Spatial cells load around streaming sources. World Partition works with One File Per Actor, data layers and HLOD. Actor references can cause actors to load together. | Later spatial cells should declare dependency groups. Keep M1's single scene file; changing authoring storage now would expand scope and require migration. |
| Unity Addressables 2.7.6, [asset memory][S2] and [dependencies][S3] | Asset and bundle lifetimes depend on references; shared dependencies and bundle layout affect memory and duplication. A released asset can remain resident until its containing AssetBundle unloads; other live assets can keep that bundle resident. | Use typed resource handles, explicit dependency tracking and measurable cache ownership. Hash-addressed bytes alone do not define resource lifetime. |
| Godot stable docs, [multiplayer][S4] and [interpolation][S5] | RPC authority, reliability and channels are explicit choices. Physics ticks and rendered frames are distinct, with interpolation used to smooth display between simulation states. | Make command authority and delivery semantics visible in Metallic's protocol. Preserve fixed simulation steps and keep display interpolation out of authoritative physics state. |
| Roblox Creator Hub, [streaming][S6] and [client/server boundary][S7] | Streaming changes which world instances are present on a client. Atomic and persistent models have different guarantees. Server code must validate client-triggered actions. | Define missing-instance behavior and server permissions before adding streaming. Templates should demonstrate validated requests; persistence must have an observable memory cost. |
| Bevy Quick Start, [ECS][S8] | Components group data for systems; independent systems can run in parallel. Explicit chaining is needed when execution order affects behavior. | Keep the Instance tree as the creator API. Add dense subsystem data and explicit schedule dependencies only where measurements justify them. No Bevy crate dependency. |
| PlayCanvas, [optimization guidelines][S9] and [MiniStats][S10] | Draw-call overhead, material variants and allocation patterns affect performance. Timings distinguish frame duration, CPU work and GPU work; those measurements can overlap. | Publish frame-time distributions and subsystem counters. Batch repeated shapes, reuse buffers and constrain shader variants before adding expensive rendering features. |
| Babylon.js, [thin instances][S11] | Thin instances reduce per-object CPU representation, but use group visibility and incur more cost when frequently adding or removing instances. | Partition instancing into bounded spatial/material groups. One global batch can submit too much invisible geometry and make editing expensive. |

A feature's presence in another engine does not establish the best implementation for Metallic. These sources
document mechanisms, not a controlled comparison on Metallic's target scenes or hardware.

## 3. Preserve one world API; specialize the data behind it

**Proposal for core, renderer and physics maintainers.** The current generational arena is an appropriate M1 starting
point. Creators should continue to address Instances through the registry and stable runtime handles. Renderer and
physics caches may store compact arrays indexed by those handles. Benchmark cache synchronization before replacing
the world model with a general ECS.

Define these invariants before growing the interfaces:

- A destroyed runtime handle must never address a later object. Test slot reuse and stale references.
- Runtime handles, persistent scene identity and network entity identity have different lifetimes. File traversal
  indices are acceptable for an atomic single-file load, but cannot become long-lived Team Create identifiers.
- A world mutation has one authoritative application point. Subsystems should queue commands and update caches at
  documented boundaries, avoiding a renderer and physics engine each rewriting the same property unpredictably.
- Creation, destruction, reparenting and property changes all need cache invalidation semantics. Property signals
  alone cannot describe the complete lifecycle of a renderable object.
- Event observers must not hold ownership that keeps deleted Instances alive. Play-session observers belong to a
  session and are discarded with it.

Acceptance: create, render-sync, destroy, reuse the slot, then sync again without a stale render or physics object.
Repeat after cloning a world and reconnecting observers. For future collaboration, introduce persistent IDs through
a versioned scene migration, with duplicate-ID and unresolved-reference diagnostics. This is later work, not a
requirement to change M1's RON representation now.

## 4. Simulation and scripts need aggregate budgets

Godot's [interpolation guide][S5] distinguishes physics ticks from presentation. Luau's [sandbox documentation][S12]
also makes a critical boundary explicit: removing dangerous libraries does not enforce host CPU or memory limits.
The host supplies interruption and allocation policy; a long host function can delay interruption. Separate VMs
are recommended where trusted and untrusted code require strong isolation.

**Proposal for #6, #7, #8 and later server work:**

- Retain the M1 constants: 1/240 s physics substeps, 0.1 s per script resume and 256 MB per VM. They are current
  specification values, not measured performance guarantees. The review checkout's `MEMORY_LIMIT_BYTES` is
  `256 * 1024 * 1024`, so the implemented allocator cap is 256 MiB.
- Treat 0.1 s as an execution-abort ceiling. At 60 frames/s, one frame is about 16.67 ms; a script permitted to run
  for the full ceiling can stall several frames. A collection of individually bounded callbacks can still monopolize
  a frame. Measure total script time and queue length, then add fair per-frame scheduling under a separate design.
- Keep the existing clamp for this proposal. At review commit `ebb94f3`, `PhysicsWorld::step` already accumulates
  accepted time, advances 1/240 s substeps and clamps each finite delta to `MAX_STEP = 0.25` seconds.
  [#33](https://github.com/Metallic-Club/studio/issues/33) is closed: non-finite deltas are ignored rather than poisoning
  the accumulator. `Game::frame` also has `MAX_FRAME_DT = 0.25` and a fixed 60 Hz tick accumulator.
  Proposed follow-up under #15: expose accepted, simulated, residual and unsimulated time, plus a separate invalid-delta
  count. A 500 ms pause currently discards 250 ms by design; this proposal adds accounting, not deterministic catch-up
  or a larger clamp. Count a rejected interval once at its rejecting layer, not again in the downstream physics ticks.
  Non-finite samples increment the invalid count rather than contributing a non-finite duration to time counters.
- Interpolate presentation from previous/current simulation states. A teleport resets interpolation history; cameras
  and colliders must not interpolate by mutating the authoritative world.
- Bound work exposed by host APIs too: traversal sizes, event payload bytes, pending tasks, callback count and log
  output. A VM allocation cap does not cap all Rust allocations made on behalf of a script.
- Stop invalidates a session generation before queued callbacks can execute. Restart creates fresh VM/task/signal
  state. Both normal shutdown and timeout paths should exercise the same cleanup contract.

Acceptance: run an infinite loop, allocation loop, callback flood and repeated stop/start in isolated tests with an
outer process timeout. A subsequent benign script must still execute. Record timeout overshoot and total host memory,
alongside the VM counter. A failed safety test must terminate the test subprocess, not hang the entire CI job.

Fixed timesteps alone do not guarantee deterministic floating-point replay across hardware, compiler builds or
physics versions. First target repeatability on a recorded build, with a fixed seed and identical input trace.
For a direct physics call with zero initial residual and `dt = 0.5`, expect 60 substeps (0.25 s simulated) and a
proposed unsimulated-time increment of 0.25 s. With a prior residual, check
`input_time = simulated_time + residual_change + unsimulated_time` to a provisional 1e-8 s accounting tolerance,
allowing for the current substep rounding guard without hiding a discarded 250 ms interval.
Compare motion to the accepted-time reference, not a reference that simulated the full pause. Section 8 assigns
the position tolerance and report owner; no lost-time counter is claimed to exist today.

## 5. Resources, loading and recovery

**Proposal for #17 and the platform integration milestone.** Use the Unity lifetime lessons in section 2 to separate
three things: asset metadata/permissions, immutable bytes, and decoded CPU/GPU resources. Each requires its own
failure and lifetime rules.

Suggested resource state machine: `Unloaded -> Loading -> Ready` or `Failed`; retry is explicit. Cancellation drops
only the requesting owner's interest. Shared loads are deduplicated; GPU upload happens on the permitted render
thread; a canceled upload must not install stale state into a newly reused handle.

A content manifest should identify schema version, content digest, byte size, media type, dependency digests and
compatibility requirements. Verify bytes before decoding. A digest verifies identity/integrity, not uploader trust
or authorization. Moderation and access checks remain platform responsibilities.

Keep bounded disk, decoded-memory and GPU caches. Evict unreferenced resources with observable counters; never
evict a live resource solely because its bytes are old. Store imported derivatives under an importer-version key
so changing an importer cannot silently reuse incompatible cached output.

Local saver hardening is already owned by [#31](https://github.com/Metallic-Club/studio/issues/31) and
[PR #45](https://github.com/Metallic-Club/studio/pull/45), both open at this review: bounded scene traversal, typed
validation, same-directory replacement, failure cleanup and Unicode/reference regression fixtures. Use their
implementation and acceptance tests; do not start a duplicate saver. Additional proposals here are persistent
scene IDs with migration (section 3), an explicit recovery-copy policy where required by the supported failure
model, and filesystem-specific power-loss verification. An ordinary rename test does not establish sudden-power-loss
durability. A recovery copy must not be advertised as a guarantee that #31/#45 already provide.

Resource acceptance, separate from the saver work: two consumers of the same asset share one load; releasing one
keeps the other valid; failed or canceled requests do not leak ownership. Reuse #31/#45's malformed-input and
replacement fixtures in the scene benchmark instead of assigning a second implementation.

## 6. Multiplayer is an authority contract

**Proposal for #15.** Use server authority as the default for health, inventory, progression and shared world changes.
Godot's authority modes and Roblox's validation guidance in section 2 support making this explicit; they do not
provide a transport selection for Metallic.

Start with a headless process for one match, built from the same simulation and script crates. Distinguish two
requirements against review commit `ebb94f3`:

- **Runtime initialization:** the implemented player `--headless` path calls `Game::frame` without creating a window
  or requesting a GPU adapter/device. In contrast, `RenderContext::headless` is an offscreen renderer constructor
  that does request an adapter and device; it is not a graphics-free server initializer.
- **Build graph:** `metallic_runtime/Cargo.toml` still unconditionally depends on `metallic_render`, `wgpu` and `egui`.
  A no-device execution path does not remove those dependencies. Before claiming a graphics-free server build,
  make presentation dependencies/modules optional or split shared simulation from client presentation. Then inspect
  the selected server dependency graph and run it on a host without a usable adapter. The proposed server graph
  must exclude `metallic_render`, `wgpu`, windowing and GUI crates, and its startup path must request no device.

This is the M2 prerequisite for the proposed `metallic_net` protocol crate and `metallic-server` executable in the
platform study's [shared-code boundary](2026-10-30-platform-architecture.md#14-shared-code-where-the-most-work-is-saved)
and [M2a build order](2026-10-30-platform-architecture.md#18-build-order-mapped-to-milestones).
Section 9 names the affected crates; no server crate or optional-runtime build mode is claimed to exist yet.
Client prediction may later improve responsiveness; the server remains the source of accepted state.

Before choosing UDP, QUIC or another transport, define and test the messages:

| Contract | Proposed minimum behavior |
|---|---|
| Handshake | Protocol/schema version, authenticated session, short-lived single-use join grant and rejection reason |
| Entity lifecycle | Session-scoped network ID with reuse protection; spawn before dependent updates; explicit despawn |
| Commands | Sender identity comes from the authenticated connection; validate permission, shape, size, finite numbers and rate |
| State | Snapshot sequence, simulation tick, bounded payloads and explicit missing-entity behavior |
| Delivery | Reliable lifecycle/control messages; sequenced state updates where older snapshots can be discarded |
| Disconnect/rejoin | Release observers and queues; take a new baseline rather than replay stale connection state |
| Abuse/load | Per-peer byte and work budgets; one slow receiver cannot grow queues without bound |

Separate control/lifecycle traffic from frequent state updates where the selected transport permits it. Test ordering
and head-of-line behavior rather than inferring them from a protocol's name. Never put platform bearer credentials
in logs or public process arguments; use the platform's approved local handoff mechanism when implemented.

Acceptance: run a server and two player processes; kill and reconnect one; inject delay, jitter, loss, duplication
and reordering; verify convergence and rejection of forged identity or authority. Late join must reconstruct a
consistent baseline. The provisional convergence/error limits and first-baseline ownership are in section 8.
Local two-client tests should be a normal Studio workflow, not an external manual ritual.

Streaming comes after these lifecycle semantics. Prototype bounded spatial cells with hysteresis and prefetch for
teleports. Define atomic groups, explicitly retained objects and references to absent objects. Measure retained memory
and queued bytes, because persistent objects and cross-cell dependencies can defeat the intended bounds. Test a
high-speed camera crossing cell boundaries and a script waiting on an object that streams out.

## 7. Rendering should expose its costs

**Proposal for #3, #18 and #23.** Keep M1's instanced primitive shapes, physically based materials, shadows, sky and fog.
Add counters while building those paths: visible instances, culled instances, draw calls per pass, instance bytes
uploaded, material/pipeline changes and persistent CPU/GPU resource bytes.

PlayCanvas's [measurement definitions][S10] warn against adding overlapping CPU and GPU timings. Metallic should
label frame interval, CPU preparation time and asynchronous GPU time separately. Unsupported GPU timestamps should
be `unavailable`, not zero. Record sample age and whether debug validation was enabled.

Partition batches by compatible pipeline/material and a spatial region. Use dirty ranges for changed transforms;
measure when fragmented updates cost more than a full upload. Sort transparent geometry separately and document
the visual limitations of the initial sorting method. Shadows and transparency can dominate work even when the
opaque draw-call count is low.

Defer runtime adoption of global illumination, meshlets, GPU-driven visibility and advanced upscaling until a
representative scene and profiler identify the limiting resource. Reuse the completed lighting research and WGSL
prototypes in [#37](https://github.com/Metallic-Club/studio/issues/37) and the [lighting study](2026-10-30-lighting.md);
this sequencing rule does not cancel that work or request a duplicate study. The complementary
[#42 crate/API feasibility issue](https://github.com/Metallic-Club/studio/issues/42) covers debugger/editor/installer/
profiler integration risks; it is open at this review, so its proposed report is not cited as an existing artifact.
A technical choice is accepted only with before/after evidence on the
target quality settings, including memory and image correctness. The browser-engine sources demonstrate useful
tradeoffs, not that their APIs should be reproduced in Rust.

## 8. Reproducible acceptance and benchmarks

All numbers below are **proposed test parameters/gates or existing spec targets**, not measurements. Proposed gates
need review after the first baseline and do not amend the M1 acceptance contract. An owner issue is named when a
numeric limit still needs that baseline; no such row can be reported as passing a performance budget until its
limit is recorded. Issue references are coordination anchors, not silent additions to active workers' scopes.
Create reports under `docs/benchmarks/engine-systems/YYYY-MM-DD/<commit>-<workload>/` on the first run: `README.md`
for environment, exact command, parameters, owner issue, threshold decision and verdict; raw CSV/JSON samples plus
artifact checksums alongside it. A skipped or uninstrumented measurement is `not verified`, never a passing result.

| Workload | Procedure and evidence | Provisional gate or first-baseline owner |
|---|---|---|
| Existing M1 stress target | Spec: 5,000 Parts on screen at 60 fps on the specified PC. **Proposed measurement settings:** 1920x1080, High; those settings come from the separate Parkour acceptance, not the stress sentence. At least 30 s after warmup; average fps, p50/p95/p99 frame ms, quality flags, screenshot/checksum | Keep the spec's 60 fps target; it does not define an average-only criterion. Proposed stutter gates: p99 <=25 ms and no frame >33.4 ms after warmup. Owner #18 reviews these gates after the first baseline; report each verdict separately |
| Parkour on integrated graphics | Separate spec target: at least 30 fps, 1920x1080, Low, Intel Iris Xe class; same sample/report procedure on that hardware | Spec target is 30 fps; proposed p99 <=50 ms and no frame >66.7 ms. Owner #18 reviews percentile gates after baseline. Without that machine, `not verified` |
| Render scaling | Same seeded camera path with 1k, 5k and 20k Parts; 0%, 1% and 100% transforms changing. CPU sync, upload bytes, GPU time when supported, draws and memory | Zero stale/missing instances. Set time, memory and upload regression limits after first baseline, owners #105 (sync) / #18 (render track); unavailable GPU timing stays unavailable |
| Simulation scheduling | Same tick-indexed input trace at presentation rates 30/60/144 for 10 simulated seconds; direct 500 ms stall and invalid-delta fixtures. Record input/accepted/simulated/residual/unsimulated time and tick counts | Preserve the 0.25 s clamp: from zero residual, stall simulates 0.25 s and accounts for 0.25 s unsimulated, each within 1e-8 s. Equal accepted tick counts; provisional position error <=1e-3 studs on the same build. Owner #15 for accounting follow-up, preserving closed #33's invalid-delta fix |
| Scene resilience | Reuse #31/#45 wide/deep, malformed/future-version and interrupted-save fixtures. Record typed errors, retained world/file state, elapsed time and peak memory | Zero accepted corrupt fixtures or unintended world changes; retain the previous complete file on the supported write/replace failures per #31/#45. Set latency/peak-memory limits after first baseline, owner #17; persistent IDs/recovery-copy/power-loss work remains separate |
| Script isolation | 100 isolated infinite-loop resumes, allocation growth, large host requests, callback/task flood, 100 Play/Stop cycles. Record abort overshoot from resume entry, parse/compile time separately, VM/host memory, queue/observer counts and benign-script recovery | Proposed single-resume p99 abort <=120 ms for the 100 ms budget on a quiet host; 2 s outer subprocess watchdog per attempt. Zero callbacks from stopped sessions; observer count returns to baseline. Host-memory/queue ceilings set after first baseline, owner #23; VM allocator cap remains 256 MiB |
| Multiplayer | Two clients; 0/50/150 ms one-way delay, 0/1/5% loss, documented jitter/reorder seed. Record RTT, snapshot age, queues, corrections and authoritative state | Proposed quiescent convergence within 2 s of link restoration: IDs/integers/booleans exact, positions within 1e-3 studs of the authoritative snapshot; zero forged commands accepted. Owner #15 sets queue caps, snapshot-age and jitter/reorder limits after first baseline |
| Streaming | Seeded path, boundary oscillation and teleport; cold/warm cache. Record time to usable region, resident objects/bytes, download/queue bytes, failed references and visible frame hitches | Zero stale references resolved to reused objects; missing objects have explicit behavior. Set usable-region delay and visible-hitch limits after first baseline, owner #15; resident/cache byte limits owner #17. No bounded-memory or stall-free claim before those limits exist |
| Creator loop | Fresh template project, edit property/script, Play, Stop, save, reopen. Record steps, times, recovery and exact project; include a new creator | All intended property/script changes round-trip; zero Play changes leak into the edit world. Step/time limits set after first creator baseline, owner #12. No comparative ease-of-use claim from an unobserved walkthrough |

Run release builds for performance; record commit, toolchain, OS, CPU/GPU/driver, resolution, power mode, quality,
backend, asset digest and seed. Use at least three runs and report spread. Separate cold-cache startup from warm-cache
startup. Count dropped or unavailable samples. Compare changes on the same machine and workload before interpreting
small differences. Integrated-graphics and other-OS targets remain unverified until tested on those machines.

## 9. Sequencing and ownership

| Stage | Existing issue path | Affected crates/build boundary | Proposed addition | Exit condition |
|---|---|---|---|---|
| M1 foundations | #3 renderer, #4 physics, #6 sandbox; then #5/#7 | `metallic_core`, `metallic_physics`, `metallic_script`, `metallic_render` | Lifecycle regressions, host-work limits, subsystem counters; reuse #31/#45 saver hardening | Existing acceptance and failure fixtures; benchmark gates/baseline owners in section 8 |
| M1 runtime | #8 (implemented/closed at review), closed #33 | `metallic_runtime`, `metallic_player`; render-linked headless execution already exists | Preserve scheduling; propose explicit lost-time accounting for the later #15 follow-up | Existing Parkour smoke/playthrough; clamp-aware accounting and position gates in section 8 |
| M1 authoring | #9 -> #10 -> #11 -> #12; #31/#45 saver ownership | `metallic_studio`, `metallic_runtime`, `metallic_core` | Reuse saver work; deterministic project output and session cleanup | Creator-loop correctness; first timing/step baseline owner #12 |
| M1 diagnostics/languages | #22/#23, #25 and complementary #42 feasibility | `metallic_script`, `metallic_runtime`, `metallic_render`, `metallic_studio` | Shared measurement schema; language conformance fixtures; crate/API risks reviewed in #42 | Debug/measurement fixtures tested; language claims require compile/run fixtures; script gate owner #23 |
| M2 | #15 after M1 dependencies; platform study sections 1.4/1.8 | Shared `metallic_core`/`metallic_physics`/`metallic_script`; render-optional/split `metallic_runtime`; proposed `metallic_net` and `metallic-server` | Graphics-free server build graph and no-device startup, authority protocol, multi-process tests, then interest/streaming | Graph excludes graphics/window/GUI dependencies; no adapter requested; section 8 convergence/lifecycle gates, remaining baseline limits owned by #15 |
| M4/M5 | #17/#18; completed #37 lighting research | Resource/cache owner to be designed under #17; `metallic_render` for #18 | Versioned import/cache handles; reuse lighting research before evidence-led renderer adoption | Explicit byte/lifetime limits, baseline-owned scaling gates and measured image/performance changes |

Create separate implementation issues after maintainers review this proposal. Do not silently add these acceptance
conditions to an active worker's issue or duplicate another session's work. Keep platform identity, uploads,
moderation, catalog and fleet policy in `Metallic-Club/plateform`; this research changes no files there.

## 10. Limits and decisions left open

- This is a targeted systems study, not a survey of every engine or all engine capabilities. It does not evaluate
  licensed console SDKs, confidential engine internals, current pricing or market share.
- No competitor benchmark was run. Different scenes, content pipelines and quality settings cannot establish a
  meaningful speed ranking without a separately designed comparison.
- Choose a network transport after testing required message behavior and checking platform constraints. Do not
  treat the earlier platform document's transport options as an accepted selection.
- Persistent IDs, collaboration merge policy, asset memory caps and streaming cell sizes need designs and migration
  plans. These recommendations do not override M1's existing scene/API contracts.
- The console-layout inconsistency remains #29. Browser documentation does not establish console support.
- Broad language support remains planned under #25. Toolchain availability is not evidence of full API, debugger
  or performance parity. Require compile/run/debug fixtures for each promised language.
- The original audited baseline was small. Passing its recorded tests does not establish the implementation,
  acceptance or release status of later systems.

## Primary sources

All accessed 2026-10-30. No source code or extended quotations reproduced. Recommendations and proposed benchmark
parameters above are the author's analysis, not statements from these sources.

1. [Epic Games: World Partition][S1], documentation identified as Unreal Engine 5.8.
2. [Unity: Managing Addressable asset memory][S2], Addressables 2.7.6.
3. [Unity: Addressable asset dependencies][S3], Addressables 2.7.6.
4. [Godot: High-level multiplayer][S4], stable documentation branch.
5. [Godot: Physics interpolation introduction][S5], stable documentation branch.
6. [Roblox: Instance streaming][S6], Creator Hub.
7. [Roblox: Securing the client-server boundary][S7], Creator Hub.
8. [Bevy: ECS][S8], Quick Start.
9. [PlayCanvas: General Guidelines][S9], optimization manual.
10. [PlayCanvas: MiniStats][S10], page documents Engine 2.23 and later.
11. [Babylon.js: Thin Instances][S11], current documentation; feature introduced in 4.2.
12. [Luau: Embedding a sandboxed virtual machine][S12].

[S1]: https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition-in-unreal-engine
[S2]: https://docs.unity3d.com/Packages/com.unity.addressables@2.7/manual/memory-assets.html
[S3]: https://docs.unity3d.com/Packages/com.unity.addressables@2.7/manual/AssetDependencies.html
[S4]: https://docs.godotengine.org/en/stable/tutorials/networking/high_level_multiplayer.html
[S5]: https://docs.godotengine.org/en/stable/tutorials/physics/interpolation/physics_interpolation_introduction.html
[S6]: https://create.roblox.com/docs/workspace/streaming
[S7]: https://create.roblox.com/docs/scripting/security/client-server-boundary
[S8]: https://bevy.org/learn/quick-start/getting-started/ecs/
[S9]: https://developer.playcanvas.com/user-manual/optimization/guidelines/
[S10]: https://developer.playcanvas.com/user-manual/optimization/mini-stats/
[S11]: https://doc.babylonjs.com/features/featuresDeepDive/mesh/copies/thinInstances/
[S12]: https://luau.org/sandbox/
