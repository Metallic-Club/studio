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

1. Complete the bounded Luau host, physics and renderer work already assigned in M1.
2. Harden scene identity, loading and save recovery before creators depend on projects.
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

The existing [platform architecture document](2026-10-30-platform-architecture.md) covers product boundaries,
Launcher, asset service and matchmaker. This document supplies system-level evidence and acceptance criteria.
It does not resolve the console-layout question tracked in #29.

## 2. What the other engines demonstrate

These are observations from primary sources, followed by **proposed Metallic applications**. They are not feature
parity ratings, speed rankings or claims about undocumented internals.

| Ecosystem and documentation read | Sourced observation | Proposed Metallic application and tradeoff |
|---|---|---|
| Unreal Engine 5.8, [World Partition][S1] | Spatial cells load around streaming sources. World Partition works with One File Per Actor, data layers and HLOD. Actor references can cause actors to load together. | Later spatial cells should declare dependency groups. Keep M1's single scene file; changing authoring storage now would expand scope and require migration. |
| Unity Addressables 2.7.6, [asset memory][S2] and [dependencies][S3] | Asset and bundle lifetimes depend on references; shared dependencies and bundle layout affect memory and duplication. Releasing an asset does not necessarily unload its bundle immediately. | Use typed resource handles, explicit dependency tracking and measurable cache ownership. Hash-addressed bytes alone do not define resource lifetime. |
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
  specification values, not measured performance guarantees. Name whether MB is decimal or binary when implementing.
- Treat 0.1 s as an execution-abort ceiling. At 60 frames/s, one frame is about 16.67 ms; a script permitted to run
  for the full ceiling can stall several frames. A collection of individually bounded callbacks can still monopolize
  a frame. Measure total script time and queue length, then add fair per-frame scheduling under a separate design.
- Specify accumulator behavior for invalid deltas, long pauses and overload. Bound work per frame, record unsimulated
  time, and expose whether the simulation slowed. Do not silently claim deterministic catch-up after discarding time.
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

Before networking, finish local file resilience: limits for input size, nesting, object count and string sizes;
typed errors for invalid classes, duplicate fields and unresolved references; a temporary sibling file and safe
replacement strategy for saves; a recovery copy where replacement cannot be guaranteed. Test the documented behavior
on Windows first, including spaces and non-ASCII paths. Unsupported future scene versions must fail clearly.

Acceptance: corrupt/truncated scenes fail without altering the edit world; interrupted saves preserve either the
previous complete file or the new complete file under the supported failure model. Two consumers of the same asset
share one load; releasing one keeps the other valid; failed or canceled requests do not leak ownership.
Crash consistency against sudden power loss needs filesystem-specific verification and must not be inferred from
an ordinary rename test.

## 6. Multiplayer is an authority contract

**Proposal for #15.** Use server authority as the default for health, inventory, progression and shared world changes.
Godot's authority modes and Roblox's validation guidance in section 2 support making this explicit; they do not
provide a transport selection for Metallic.

Start with a headless process for one match, built from the same simulation and script crates. It must run on a
machine without a graphics device. Client prediction may later improve responsiveness; the server remains the
source of accepted state.

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
consistent baseline. Local two-client tests should be a normal Studio workflow, not an external manual ritual.

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

Defer global illumination, meshlets, GPU-driven visibility and advanced upscaling until a representative scene and
profiler identify the limiting resource. A technical choice is accepted only with before/after evidence on the
target quality settings, including memory and image correctness. The browser-engine sources demonstrate useful
tradeoffs, not that their APIs should be reproduced in Rust.

## 8. Reproducible acceptance and benchmarks

All numbers below are **proposed test parameters or existing spec targets**, not measurements. Store raw samples and
the command used with each report. A skipped test is `not verified`, never a passing result.

| Workload | Procedure | Evidence required |
|---|---|---|
| Existing M1 stress target | 5,000 Parts, 1920x1080, High; spec target is average at least 60 fps on the specified RTX 3060 Ti/Ryzen 5 5500 PC | At least 30 s after warmup, average fps, p50/p95/p99 frame ms, quality flags, screenshot and artifact checksum |
| Render scaling | Same seeded camera path with 1k, 5k and 20k Parts; repeat with 0%, 1% and 100% transforms changing | CPU sync, upload bytes, GPU time if supported, draw calls, memory; no extra acceptance target until baseline exists |
| Simulation scheduling | Same timestamped input at presentation rates 30, 60 and 144; inject a 500 ms stall | Tick count, accumulated time, overload policy and position error against reference |
| Scene resilience | Wide/deep fixtures, malformed and future-version inputs, interrupted saves | Typed errors, retained world/file state, elapsed time and peak process memory |
| Script isolation | Infinite loop, allocation growth, large host requests, callback/task flood, 100 Play/Stop cycles | Outer timeout, VM/host memory, queue counts, benign-script recovery and observer count |
| Multiplayer | Two clients; proposed profiles: 0/50/150 ms one-way delay, 0/1/5% loss, plus jitter/reorder cases | RTT, snapshot age, queue bytes, corrections, convergence, unauthorized-command rejection |
| Streaming | Seeded path, boundary oscillation and teleport; cold and warm cache | Time to usable region, resident objects/bytes, download bytes, failed references and visible stalls |
| Creator loop | Fresh project from template, edit a property and script, Play, Stop, save, reopen | Steps, elapsed time, error recovery and exact resulting project; test with a new creator before claiming ease of use |

Run release builds for performance; record commit, toolchain, OS, CPU/GPU/driver, resolution, power mode, quality,
backend, asset digest and seed. Use at least three runs and report spread. Separate cold-cache startup from warm-cache
startup. Count dropped or unavailable samples. Compare changes on the same machine and workload before interpreting
small differences. Integrated-graphics and other-OS targets remain unverified until tested on those machines.

## 9. Sequencing and ownership

| Stage | Existing issue path | Proposed addition | Exit condition |
|---|---|---|---|
| M1 foundations | #3 renderer, #4 physics, #6 sandbox; then #5/#7 | Lifecycle regressions, host-work limits, subsystem counters | Existing acceptance plus recorded failure-path tests |
| M1 runtime | #8 | Explicit frame scheduling and headless smoke fixture | Parkour completes the specified smoke test and manual playthrough |
| M1 authoring | #9 -> #10 -> #11 -> #12 | Recoverable saves, deterministic project output, session cleanup checks | New-project/edit/Play/Stop/save/reopen walkthrough succeeds |
| M1 diagnostics/languages | #22/#23 and #25 after their documented dependencies | Shared measurement schema; generated API conformance tests across languages | Debug/measurement features tested; a language claim requires compiling and running its fixture |
| M2 | #15, after its M1 dependencies | Authority protocol, headless server, two-process tests, then interest/streaming prototype | Reconnect, late join and impairment tests pass |
| M4/M5 | #17/#18 | Versioned import/cache handles and evidence-led renderer work | Bounded resource lifetime and measured quality/performance changes |

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
- The immediate baseline is small. Passing its current tests does not mean later systems are implemented, accepted
  or released.

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
