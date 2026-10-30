# Metallic performance and scale plan

Date: 2026-10-30. Status: proposal for the owner. Author: Claude (research session, branch
`studio/research-performance`).
Scope: engine runtime, load and join time, low-end PCs, networking (M2), platform backend scale, and how we measure
all of it. Budgets, design changes, a benchmark suite, a perf-CI gate and proposed issues.

Evidence labels: **Sourced** means I fetched the page (URL given) or read the source file named. **Measured** means I
ran it on the owner's PC tonight (numbers below, with the noise caveat in 2.1). **Judgment** means my own reasoning.
Calls I made without the owner are marked "assistant's call".

---

## 1. Summary

1. **Per-frame polling of the Instance tree was the biggest problem; half of it is fixed.** Reading seven properties
   of every part through `World::get` costs 7.2 ms for 5,000 parts on the pre-fix code (best of three rounds; 10.3 ms
   in the worst). The renderer lane merged a change journal (`World::changes_since`, PR #59) tonight, so `Scene::sync`
   now re-reads only changed parts. The physics step (#4, merged) still reads every part every frame and should move to
   the journal next.
2. **Fixed in this PR (now, cheap):** `classes::find_property` built a `Vec` of the whole class chain on every
   property read and write. Walking the chain without allocating cuts the 5,000-part read to 2.4 ms (3.0x faster) and a
   `set` from 656 ns to 145 ns (4.5x). Every `World::set` the renderer's stress test makes (500 per frame) and every
   property write a script makes gets cheaper. Ten lines, no interface change, all tests pass.
3. **Next (now, cheap): physics on the change journal.** A static 5,000-part level then costs the physics wrapper
   close to nothing per frame instead of a full rescan.
4. **Physics runs four full pipeline steps per frame, each with four solver substeps.** Rapier 0.36 already splits
   one step into `num_solver_iterations` substeps (Sourced, Rapier source). Our `dt = 1/240` therefore solves at
   960 Hz and runs collision detection four times per frame. One step of `dt = 1/60` with 4 iterations gives the
   240 Hz solver Roblox uses, at about a third of the cost while bodies move and a seventh to a tenth once they
   settle (Measured, 2.3). Behaviour changes too, so #5's character tests decide.
5. **The Luau time budget (#6, in review) calls `Instant::now()` at every interrupt** (each loop back-edge and
   call). A hot loop runs 2.4x to 3.1x slower than with no interrupt. Reading the clock every 1,024 interrupts
   brings that to 1.4x-1.65x; installing the interrupt only when a watchdog sees an overrun removes it (Measured, 2.4).
6. **Instance property reads from Luau will cost about 350 ns each** with the plain mlua userdata pattern, against
   17 ns for a table field (Measured, 2.4). #7 should set a budget and benchmark it.
7. Budgets for three PC tiers and the server are in section 3. The headline targets: Parkour at 60 fps on the
   dev PC with at least 2x CPU headroom, 30 fps (aim 60 at a lower internal resolution) on Iris Xe, client RAM
   at most 1.5 GB on the low tier, Play in Studio in under 250 ms, game join in 5 s with a warm cache.
8. Measurement: criterion benches per crate (started here), instruction-count benches as the PR gate on Linux CI
   (wall time on shared runners is too noisy), a nightly wall-time and GPU run on the owner's PC, and the
   `profiling` crate with puffin in-app and Tracy for deep dives. All MIT/Apache/BSD.

---

## 2. What the code does today

### 2.1 Measured: `metallic_core` (benches added in this PR)

`cargo bench -p metallic_core --bench world`, release profile, Ryzen 5 5500, Windows 11, 2026-10-30 01:20-01:40
Chicago. Before = `origin/main` code; after = with the `find_property` fix in this PR. Three interleaved rounds
(before, after, before, after, ...), 1 s warm-up and 4 s measurement per bench. The table shows the best median of
the three rounds.

**Noise caveat:** other sessions kept the CPU at 88-99 % load during every run (checked with `Get-Counter`). A first
single run measured `frame_read/.../5000` at 21.6 ms; the interleaved rounds ranged 7.2-10.3 ms before and 2.4-3.7 ms
after. Treat absolute values as rough and the before/after ratio as the finding. A quiet re-run is a proposed issue
(section 6).

| Bench | What it does | Before | After | Ratio |
|---|---|---|---|---|
| `property/get_set_value` | read a property that was set (Position) | 25.7 ns | 28.5 ns | same (noise) |
| `property/get_default_value` | read a property left at its default (Transparency) | 278 ns | 54.7 ns | 5.1x |
| `property/get_name` | read `Name` (clones a String) | 91.7 ns | 88.0 ns | same |
| `property/set_position` | write Position | 656 ns | 145 ns | 4.5x |
| `property/is_a_basepart` | class check | 16.5 ns | 16.3 ns | same |
| `frame_read/seven_props_per_part/300` | renderer/physics read of a Parkour-size level | 379 µs | 114 µs | 3.3x |
| `frame_read/seven_props_per_part/5000` | same for the 5,000-part stress level | 7.19 ms | 2.40 ms | 3.0x |
| `tree/find_first_child_miss_1000` | `FindFirstChild` scanning 1,000 children | 88.4 µs | 87.4 µs | same |
| `tree/full_name` | dotted path of a part | 390 ns | 363 ns | same |
| `tree/create_1000_parts` | create 1,000 parts and set 4 properties each | 4.29 ms | 1.09 ms | 3.9x |
| `tree/destroy_1000_siblings_one_by_one` | destroy every child of a 1,000-child Model | 765 µs | 750 µs | same |
| `tree/deep_clone_5000` | the Play clone | 5.38 ms | 4.74 ms | same (noise) |
| `scene/save_5000` | write a 5,000-part RON scene | 61.4 ms | 38.3 ms | 1.6x |
| `scene/load_5000` | parse and build it | 71.7 ms | 55.4 ms | 1.3x |

Allocation counts (Measured, counting global allocator, 5,000 parts with 4 non-default properties each):

| Measure | Before | After |
|---|---|---|
| Heap allocations per seven-property part read | 14 | 2 (the `Material` and `Shape` strings) |
| Heap allocations per `set(Position)` | 8 | 1 (the property-name key in the map) |
| Live heap per Part | 894 B | 894 B |

What is left after the fix: 2.4 ms per frame for each reader that polls 5,000 parts is still too much on the Low
tier. The change journal (PR #59) removed the polling from the renderer and can do the same for physics; typed
storage (4.2.1) removes the remaining allocations. `FindFirstChild` costs about 88 ns per child
scanned because each `Name` read clones a String; Luau code that indexes children by name (`script.Parent.Humanoid`)
pays that per lookup.

### 2.2 Findings in the code (Sourced: files read on `origin/main` and in the in-flight worktrees)

**metallic_core (merged, #2)**
- `find_property` allocated a `Vec` of the whole class chain per call (`all_properties`), and `get`/`set`/
  `default_value` call it one to three times. Fixed here.
- `get` returns an owned `Value`, so reading `Material`, `Shape` or `Name` allocates a `String` every time. Default
  values come from `fn() -> Value`, which allocates for strings too.
- Each node stores properties in a `BTreeMap<String, Value>`: one heap string per set property plus a B-tree leaf.
  `size_of::<Value>()` is 40 bytes.
- `set` builds a `(InstanceId, String)` key to look up property signals on every change. This PR skips that when no
  signal exists at all; once a script connects any `GetPropertyChangedSignal`, every `set` allocates again.
- `class_def` is a linear scan with string compares over 24 classes; `is_a` walks the chain the same way. Fine
  at 24 classes; switch to a perfect hash or class index when the registry passes about 100 classes.
- `destroy` runs `prop_signals.retain` over all signals per destroyed subtree, and `detach` is O(siblings). Destroying
  1,000 siblings one by one costs about 1 ms; fine for now. (#47 covers a panic in the same `retain`.)
- `save_scene` maps refs with `order.iter().position(..)`, O(n) per ref; harmless while only `Player.RespawnLocation`
  is a Ref, quadratic once Models carry `PrimaryPart` refs. Use a `HashMap` then.
- Change journal (added by PR #59 tonight): `World::changes_since(cursor)` returns the ids touched since a cursor,
  deduplicated with a fresh `HashSet` per call, bounded at 16,384 entries (`Changes::Overflow` forces a full re-read).
  It records ids, not property names, so a consumer re-reads the whole instance; that is fine for parts.

**metallic_physics (merged, #4)**
- `PhysicsWorld::new` sets `integration_parameters.dt = 1/240` and `step` calls `step_with_events` once per
  substep: four full pipeline runs (broad phase, narrow phase, islands, solver) per 60 fps frame.
- Rapier 0.36.1 `staged_island_solver/init.rs` sets `num_substeps = num_solver_iterations (+ extra)` and
  `dt: base_params.dt / num_substeps` (Sourced: crate source in the local cargo registry). So each Rapier step
  already substeps the solver; the default is 4 (Sourced: [IntegrationParameters docs](https://docs.rs/rapier3d/latest/rapier3d/dynamics/struct.IntegrationParameters.html)).
- `step` calls `read_part` for every body every frame (seven `World::get`, two of them allocating strings), static
  or not, to detect moves. With a change journal only moved parts need reading.
- Write-back skips sleeping bodies. Good: a settled level writes nothing.
- `rapier3d` is used with default features; `parallel` (rayon) and `simd-stable` are off. Rapier reports up to 2x
  native speed-up across 2024-2025 with `simd-stable` on (Sourced: [Dimforge 2025 review](https://dimforge.com/blog/2026/01/09/the-year-2025-in-dimforge/)).

**metallic_render (merged tonight, PR #59)**
- `Scene::sync` now reads the journal and re-reads only changed parts; a full rebuild happens only on overflow. The
  lane measured `Scene::sync` of 5,000 parts with 50 changed at 97.5 µs (best of 20) and asserts under 1 ms.
- The stress example now moves 500 parts per frame through `World::set` and syncs each frame: 349 fps average,
  p50 2.88 ms, p99 4.29 ms, sync 1.30 ms per frame including the 500 `set` calls (RTX 3060 Ti, 1080p High; from
  `docs/agents/m1-build.md`). Those numbers predate this PR; `set` is about 4.5x cheaper now, so the sync column
  should drop. Not re-measured here: the stress run opens a window, and the owner's rule forbids pop-ups.
- `Renderer::prewarm` exists. The lane found frame 0 costs 24-32 ms with or without it and blames mid-run spikes on
  other sessions' rustc processes.
- Pipelines are created with `cache: None`. wgpu's `PipelineCache` only works on Vulkan, and desktop drivers keep
  their own caches (Sourced: [wgpu PipelineCache](https://docs.rs/wgpu/latest/wgpu/struct.PipelineCache.html)), so on
  DX12 pre-warm is the right tool.
- Transparent parts are sorted every frame; fine, the sort covers only blended parts.

**metallic_script (in review, #6; read-only peek)**
- One `mlua::Lua` per context (server, client) with a fresh environment table per script whose `__index` is the
  shared read-only library. This matches the Luau sandbox model (Sourced: [luau.org/sandbox](https://luau.org/sandbox)).
- The interrupt callback calls `Instant::now()` on every invocation. Luau calls it "at safepoints (loop back edges,
  call/ret, gc)" (Sourced: `lua.h` in [luau-lang/luau](https://raw.githubusercontent.com/luau-lang/luau/master/VM/include/lua.h)).
  Cost measured in 2.4.
- Memory cap 256 MB per VM through `set_memory_limit`. Good as a safety cap; it is not a budget (section 3.3).

### 2.3 Measured: Rapier step layout (scratch program, not committed)

Rapier 0.36.1, default features, release build, same PC and load caveat. Level of fixed 4x1x4 boxes plus falling
dynamic 2x2x2 boxes, studs gravity 196.2. Cost per 1/60 s frame, averaged over 120 frames; "settled" is measured
after 10 s more of simulation. Two runs each; ranges show both.

| Layout | 300 fixed + 20 dynamic, moving | settled | 5,000 fixed + 200 dynamic, moving | settled | Bodies still awake (5,000 case) |
|---|---|---|---|---|---|
| Today: `dt` 1/240, 4 iterations, 4 steps per frame (solver at 960 Hz) | 0.13-0.14 ms | 0.05-0.09 ms | 1.01-1.45 ms | 0.48-0.59 ms | 105 |
| A: `dt` 1/60, 4 iterations, 1 step (solver at 240 Hz) | 0.04-0.07 ms | 0.01 ms | 0.35-0.42 ms | 0.05-0.07 ms | 61 |
| B: `dt` 1/120, 2 iterations, 2 steps (solver at 240 Hz) | 0.05-0.08 ms | 0.02-0.04 ms | 0.49-0.53 ms | 0.19-0.22 ms | 102 |

Layout A costs about a third while bodies move and about a tenth once they settle. It also changes behaviour: fewer
bodies were still awake after 10 s, so sleep timing differs, and contact response per frame differs too. The
character controller (#5) and moving-platform tests must be re-run before switching. These costs exclude our own
`sync`/`step` wrapper code, which reads the World (2.1).

### 2.4 Measured: Luau through mlua 0.12.2 (scratch program, not committed)

mlua 0.12.2 with `luau` and `vendored`, release build, same PC and load caveat. Two runs; ranges show both.

| Measure | Result | Notes |
|---|---|---|
| Hot loop, 20M iterations, no interrupt callback | 400-620 ms | baseline |
| Same, interrupt calls `Instant::now()` each time (what #6 does) | 1,224-1,475 ms | 2.4x-3.1x slower |
| Same, clock read every 1,024 interrupts | 661-844 ms | 1.4x-1.65x |
| Same, interrupt reads an `AtomicBool` set by a watchdog | 752-900 ms | 1.5x-1.9x; the mlua callback trampoline itself costs most of this |
| Empty sandboxed VM heap (`used_memory` after full GC) | 378 KB | |
| Per script: environment table + metatable + compiled small chunk + thread | 2.3 KB | 1,000 scripts measured together |
| Create and sandbox one VM | 204-257 µs | |
| Rust calls a Luau function | 104-160 ns per call | |
| Luau calls a Rust function | 116-158 ns per call | |
| Luau reads a userdata field (`p.X` through mlua `add_field_method_get`) | 340-382 ns | the shape Instance property reads will have in #7 |
| Luau calls a userdata method (`p:GetX()`) | 319-321 ns | |
| Luau reads a plain table field (`t.X`), for reference | 16.8-16.9 ns | 20x cheaper than the userdata read |

Two conclusions. The clock call per safepoint is the largest avoidable cost in #6; the cheapest design installs no
interrupt callback at all during normal running and lets a watchdog thread install one (through `mlua::ffi`, as
`lua_callbacks(L)->interrupt`) only when a resume overruns its deadline (Judgment; needs a test that the
cross-thread write is safe in Luau's VM). And Instance property access from Luau costs about 350 ns per read through
mlua userdata: a script reading `part.Position` for 1,000 parts each frame spends 0.35 ms on the lookups alone.
#7 should benchmark its API and consider a namecall fast path and per-class lookup tables.

---

## 3. Budgets

### 3.1 Hardware tiers (assistant's call)

| Tier | Reference machine | Why |
|---|---|---|
| Low | Intel Iris Xe (96 EU) laptop, 4 cores / 8 threads, 8-16 GB shared RAM | The spec's floor. About 2.0-2.15 FP32 TFLOPS and no dedicated memory (search snippets only: TechPowerUp, LaptopMedia; the pages returned 403). Roughly 1/8 of an RTX 3060 Ti |
| Mid | GTX 1650 / RX 6500 class, 6 cores, 16 GB | Steam, September 2026: 16 GB RAM 37.8 %, 6 cores 27.2 %, 1080p 47.9 % of primary displays (Sourced: [Steam survey](https://store.steampowered.com/hwsurvey/)) |
| Dev | RTX 3060 Ti, Ryzen 5 5500, 32 GB (owner's PC) | The spec's acceptance machine |
| Server | One core of an AMD Zen 4 host per match (for example Hetzner AX42, 8 cores, EUR 79/month) | M2 game servers (3.5) |

Steam over-represents gaming PCs. Integrated GPUs show up as about 4.8 % (Intel) and 3.75 % (AMD) there (Sourced, same
page), but a Roblox-like audience plays far more on school and family laptops (Judgment). Design for Low; test on Mid.

### 3.2 Frame budget per subsystem

CPU main-thread time per frame in milliseconds. Each tier keeps at least 20 % in reserve for spikes. Values are
assistant's calls, set from the measurements in section 2 and the Roblox guidance below.

| Subsystem | Dev, 60 fps (16.7) | Mid, 60 fps (16.7) | Low, 30 fps (33.3) | Server tick, 60 Hz (16.7) |
|---|---|---|---|---|
| Input, window events | 0.2 | 0.3 | 0.5 | 0.5 (network receive) |
| Scripts, all VMs | 2.0 | 2.5 | 5.0 | 4.0 |
| Physics + character | 2.0 | 2.5 | 5.0 | 4.0 |
| Render sync (World to GPU data) | 0.3 | 0.5 | 1.0 | none |
| Render CPU (cull, encode, submit) | 1.5 | 2.5 | 4.0 | none |
| Game UI | 0.5 | 0.7 | 1.5 | none |
| Replication (M2) | 0.5 | 0.5 | 1.0 | 3.0 |
| Profiling and telemetry | 0.1 | 0.1 | 0.2 | 0.2 |
| **Planned total** | **7.1** | **9.6** | **18.2** | **11.7** |
| Reserve | 9.6 | 7.1 | 15.1 | 5.0 |

GPU time per frame in milliseconds:

| Pass | Dev, 1080p High | Mid, 1080p Medium | Low, 720p-900p internal, Low |
|---|---|---|---|
| Shadows | 2.0 (3 cascades, 2048) | 2.0 (2 cascades, 1024) | 3.0 (1 cascade, 1024) |
| Opaque parts | 3.0 | 5.0 | 12.0 |
| Sky, fog, transparent | 1.0 | 1.5 | 4.0 |
| Resolve / post / upscale | 1.0 (4x MSAA) | 1.5 | 3.0 (no MSAA, bilinear or FSR 1 upscale) |
| UI | 0.3 | 0.5 | 1.0 |
| **Total** | **7.3** | **10.5** | **23.0 of 33.3** |

Reference points: Roblox targets 60 fps (16.67 ms) and gives 1,000 draw calls and 1M triangles as an illustrative
baseline (Sourced: [Roblox performance design](https://create.roblox.com/docs/performance-optimization/design)). A
community guest post on the Roblox forum works with about 500 draw calls, 500K triangles and 60 fps on 90 % of phones
(Sourced, not staff: [DevForum guest post](https://devforum.roblox.com/t/real-world-building-and-scripting-optimization-for-roblox/3127146)).
egui itself expects 1-2 ms per frame (Sourced: [egui README](https://github.com/emilk/egui)), which is why Game UI gets
0.5-1.5 ms and Studio panels sit outside the game budget.

Hitch rule (all tiers): after the first 2 s, no frame over 2x the target frame time; p99 frame time at most 1.5x target.

### 3.3 Memory budgets

| Item | Budget | Today / basis |
|---|---|---|
| Client process, Low tier | at most 1.5 GB RAM including shared GPU memory | Roblox guest post: under 1.3 GB to support 2 GB phones (Sourced, not staff, link above). 8 GB laptops share RAM with the GPU (Judgment) |
| Client process, Dev tier, Parkour | at most 800 MB | Judgment |
| Studio idle with Parkour open | at most 500 MB, idle CPU near 0 % | egui repaints only on input or animation: "if your app is idle, no CPU is wasted" (Sourced: egui README) |
| Instance tree, per Part | at most 256 B average | Measured 894 B today (2.1) |
| Luau VM baseline (sandboxed, empty) | at most 512 KB | Measured 378 KB (2.4) |
| Luau per script (environment + function + thread) | at most 8 KB plus the script's own data | Measured 2.3 KB (2.4). A Luau thread is about 1-1.5 KB (Judgment from `lstate.cpp` sizes, research agent) |
| Luau heap, whole game, client | 64 MB soft budget (warning in developer mode), 256 MB hard cap per VM | Spec cap; soft budget is assistant's call |
| Textures | 512x512 default, 1024x1024 opt-in; texture pool sized from device memory | Roblox: keep most textures at or below 512, minor ones under 256; 1024 costs 4x of 512 (Sourced: [Roblox improve performance](https://create.roblox.com/docs/performance-optimization/improve)) |
| GPU memory, Low tier | at most 512 MB | Judgment (Iris Xe takes it from system RAM) |

Track memory by category from day one, the way Roblox's Developer Console splits Instances, Signals, LuaHeap,
PhysicsParts, GraphicsTexture and others (Sourced: [Roblox memory usage](https://create.roblox.com/docs/studio/optimization/memory-usage)).
Luau supports 256 memory categories per state (`LUA_MEMORY_CATEGORIES`, Sourced: `luaconf.h`), which gives per-script
heap numbers for the profiler (#23) at no extra cost.

### 3.4 Start-up, load and join targets

| Moment | Target (Dev) | Target (Low) | Today / basis |
|---|---|---|---|
| Studio cold start to start screen | 2.0 s | 4.0 s | not measured (Studio not built yet) |
| Studio warm start | 1.0 s | 2.0 s | Judgment |
| Open Parkour project (300 parts) | 0.3 s | 0.6 s | load of 5,000 parts measured 55-72 ms |
| Load a 5,000-part scene file | 100 ms | 200 ms | measured 55-72 ms; RON parse dominates (Judgment) |
| Press Play to first simulated frame | 250 ms | 500 ms | `deep_clone` of 5,000 parts measured about 5 ms; Luau compiles about 950K lines/s on one core (Sourced: [luau.org/performance](https://luau.org/performance)) |
| Player start to first frame, cached game | 2 s | 4 s | Judgment |
| Join a server, warm cache (M2) | 5 s | 8 s | Judgment |
| Join a server, cold cache, 50 MB place on 50 Mbit/s | 10 s to playable, rest streams | 15 s | Roblox texture streaming cut City template time-to-baseline-quality from 20.6 s to 3.0 s (Sourced: [Roblox texture streaming](https://devforum.roblox.com/t/introducing-texture-streaming/4144855)) |

### 3.5 Network budgets (M2)

| Item | Budget | Basis |
|---|---|---|
| Server simulation tick | 60 Hz | Matches the client frame and the physics step (2.3) |
| Snapshot send rate to clients | 20-30 Hz, interpolate 2 snapshots behind | Judgment; Valve's classic setup interpolates 100 ms (could not fetch the Valve page: 403) |
| Client download, typical / peak | 50 KB/s / 100 KB/s | Roblox community guidance: keep receive under 50 KB/s; physics throttles above about 40 KB/s (Sourced, not staff, DevForum guest post) |
| Client upload | 10 KB/s | Judgment |
| Cost per moving part | at most 0.2 KB/s at 20 Hz | Gaffer On Games: a cube's state drops from 321 bits to 80 bits with quantization, and delta-encoded position averages 26.1 bits; 901 cubes at 60 Hz start at 17.37 Mbit/s against a 256 kbit/s target (Sourced: [snapshot compression](https://gafferongames.com/post/snapshot_compression/)). Roblox's guest post measures 0.4-0.9 KB/s per moving assembly |
| RemoteEvent rate | 500 per second per client, shared per type | Roblox's documented throttle (Sourced: [RemoteEvent](https://create.roblox.com/docs/en-us/reference/engine/classes/RemoteEvent.md)) |
| Interest management | Stream by distance: always 64 studs, target 1,024 studs | Roblox defaults `StreamingMinRadius` 64, `StreamingTargetRadius` 1024 (Sourced: [Roblox streaming](https://create.roblox.com/docs/workspace/streaming)) |

---

## 4. Design changes

Each item names the task it touches: #3 render, #4 physics (merged), #5 character, #6 Luau VM, #7 Luau API,
#8 runtime/player, #9-#11 Studio, #13 installer, #23 profiler.

### 4.1 Now, cheap

1. **Allocation-free property lookup** (core; done in this PR). Helps every consumer: render sync, physics, scripts
   (#7) and Studio edits.
2. **Change journal in `World`** (core; done tonight in PR #59 by the renderer lane). Assistant's call, now also
   the lane's: keep the arena (generational slots) and add tracking rather than adopting an ECS. Bevy tracks change
   ticks per component (Sourced: [bevy_ecs change_detection](https://docs.rs/bevy_ecs/latest/bevy_ecs/change_detection/index.html));
   at Parkour scale an id journal does the same job, and the spec forbids engine crates. Remaining work: physics
   (#4 follow-up) and later replication (M2) read it too; add a `changes/changes_since_50_of_5000` bench.
3. **Physics: one Rapier step per frame with 4 solver substeps** (#4 follow-up, affects #5 tuning). Keeps a 240 Hz
   solver like Roblox (Sourced: [adaptive timestepping](https://create.roblox.com/docs/physics/adaptive-timestepping):
   240 Hz default, adaptive mode up to 2.5x faster) and cuts collision detection to once per frame. Keep the
   accumulator so a 30 fps frame runs two steps. Re-check the character controller (#5) and the moving-platform tests
   after the switch. Then read only journaled parts in `step` instead of all of them.
4. **Renderer: incremental instance updates** (#3; done in PR #59 for the CPU side). Still open: patch only changed
   slots with ranged `queue.write_buffer` instead of re-uploading a whole mesh-kind buffer when one part moves, if the
   stress sync column shows the upload matters.
5. **Present settings in the player** (#8). Pre-warm landed in PR #59. Use `desired_maximum_frame_latency = 2`
   (wgpu's documented default and "reasonable default for most applications", Sourced:
   [SurfaceConfiguration](https://docs.rs/wgpu/latest/wgpu/type.SurfaceConfiguration.html)) and `Fifo` (vsync) in the
   player by default; uncapped only behind a developer flag. Vsync is also the cheapest battery saver on laptops.
6. **Luau budget check without a clock call per safepoint** (#6). Simplest step: read the clock every 1,024
   interrupts (2.4x-3.1x slowdown becomes 1.4x-1.65x, a few lines). Best step: no interrupt callback while scripts
   run within budget; a watchdog thread installs one only when a resume overruns (2.4). Budget for #7: a Luau read of
   `part.Position` at most 150 ns (today's mlua userdata pattern measures about 350 ns).
7. **GC in frame slack** (#6, #8). After the frame's work, if time remains, call an incremental GC step with a small
   KB budget instead of letting allocation assists land inside script time. Luau's GC is incremental and
   non-moving, driven by assists or explicit steps (Sourced: `lgc.cpp`, research agent). Keep the default goal of
   200 % (Sourced: `lua.h`).
8. **Luau API shape** (#7). Use the native Luau `vector` type for `Vector3` where the API allows it (16-byte tagged
   values, no allocation, Sourced: [luau.org/performance](https://luau.org/performance)); give Instances a namecall
   fast path for methods; avoid `getfenv`/`setfenv` (they disable import optimisation, Sourced: same page). Measure
   `part.Position` reads in a bench before and after.
9. **Frame profiler scopes from the start** (#8, #23). Put `profiling::scope!` (crate `profiling`, MIT OR
   Apache-2.0) around input, each script VM step, physics, character, render sync and render encode. Back it with
   puffin (MIT OR Apache-2.0, `puffin_egui` for the in-app view) and optionally Tracy (`tracy-client`, MIT/Apache-2.0;
   the Tracy profiler itself is BSD 3-clause, Sourced: [Tracy LICENSE](https://raw.githubusercontent.com/wolfpld/tracy/master/LICENSE)).
   Zero cost when the feature is off.
10. **Release profile for shipped builds** (#12, #13). Add a `dist` profile: `lto = "thin"`, `codegen-units = 1`,
    keep `panic = "unwind"` (Studio must survive a panic in a game), `debug = "line-tables-only"` for symbolised
    crash reports. LTO gives 10-20 % speed or more and `codegen-units = 1` helps speed and size (Sourced:
    [Rust perf book](https://nnethercote.github.io/perf-book/build-configuration.html)). Measure binary size and
    start-up before and after; Velopack deltas are per-file zstd patches (Sourced:
    [Velopack deltas](https://docs.velopack.io/packaging/deltas)), so a smaller, more stable exe also means smaller
    updates.
11. **Studio idle cost** (#9, #10). Repaint the viewport continuously only while playing or while a gizmo drags;
    everything else repaints on input. Build the Explorer with `ScrollArea::show_rows` so 5,000 instances lay out
    only the visible rows (Judgment; egui's immediate mode lays out whatever you call each frame).

### 4.2 Later

1. **Typed property storage** (core, after M1). Give each class a fixed property list indexed by a small `PropId`
   and store hot properties (Position, Orientation, Size, Color, Material, Shape, Transparency) in dense arrays by
   slot. Materials and shapes become `u8` enums. This removes the `BTreeMap<String, Value>` per node and every string
   allocation on reads. Do it once the Luau API (#7) and Studio Properties panel exist, because both read by name.
2. **Parallel scripts** (after M2). Roblox runs one Luau VM per Actor; scripts in one Actor run in sequence, the docs
   recommend more Actors than cores, instance writes happen only in the serial phase, and messages between VMs are
   copied (Sourced: [Roblox multithreading](https://create.roblox.com/docs/scripting/multithreading),
   [Actor](https://create.roblox.com/docs/en-us/reference/engine/classes/Actor.md)). The same shape fits mlua: one
   `Lua` per actor on a rayon pool, read-only world access in parallel, writes queued for the serial phase.
3. **Adaptive physics rate per island** like Roblox (60/120/240 Hz, Sourced above), and the `simd-stable` and
   `parallel` Rapier features once we measure them on our scenes.
4. **Binary scene cache.** RON stays the source of truth for diffs and Team Create; the player loads a
   content-hashed binary cache built on publish.
5. **Asset streaming and a content-addressed CDN.** Assets addressed by hash with immutable cache headers; low mips
   first, texture budget from device memory, like Roblox's texture streaming (Sourced above). Distance-based instance
   streaming for big worlds.
6. **Dynamic resolution** on the Low tier: lower the internal resolution when GPU time exceeds budget for a few
   frames and upscale (bilinear first, AMD FSR 1 later; I believe FSR 1 is MIT-licensed but did not fetch it, Judgment). Frame pacing matters as much as average fps: a game
   at 30 fps on a 60 Hz display that presents unevenly shows 49/16/33 ms frames (Sourced:
   [Android frame pacing](https://developer.android.com/games/sdk/frame-pacing)); vsync plus a fixed frame cap avoids
   that.
7. **Networking** (M2): quantized delta snapshots, interest management by grid cells. Unreal built its Replication
   Graph because per-actor, per-connection relevancy checks bottleneck the server at 100 players and about 50,000
   replicated actors (Sourced: [Replication Graph](https://dev.epicgames.com/documentation/en-us/unreal-engine/replication-graph-in-unreal-engine)).
   Group by cell from the start; mark static parts dormant.
8. **Platform backend** (`plateform`, for the owner and the platform team):
   - Hot services: presence (a heartbeat every 30-60 s per online user is 17,000-33,000 writes/s at 1M concurrent;
     keep it in an in-memory store, never in SQL), the game list (read-heavy; serve as JSON from the CDN with a 10-30
     s TTL and stale-while-revalidate), join/matchmaking (bursty; queue it), and asset downloads (bytes dominate cost).
     All Judgment.
   - Asset bytes: Cloudflare R2 charges USD 0.015 per GB-month, USD 0.36 per million reads and nothing for egress
     (Sourced: [R2 pricing](https://developers.cloudflare.com/r2/pricing/)). Game servers: Hetzner AX42 (8-core Zen 4,
     64 GB) is EUR 79/month with unlimited traffic on a 1 Gbit/s port (Sourced:
     [Hetzner AX](https://www.hetzner.com/dedicated-rootserver/matrix-ax/)). A fixed pool of such hosts with a small
     allocator is the cheap start (matches the platform architecture document). Agones (Apache-2.0) is the
     Kubernetes route if we outgrow it (Sourced: [Agones](https://github.com/googleforgames/agones)).
   - Roblox's 73-hour outage in October 2021 hit a single Consul cluster serving 18,000+ servers and 170,000
     containers (Sourced: [Roblox postmortem](https://about.roblox.com/newsroom/2022/01/roblox-return-to-service-10-28-10-31-2021)).
     Lesson: no single service-discovery cluster for everything, and monitoring that does not depend on what it watches.

---

## 5. Measurement

### 5.1 Benchmark suite

Criterion 0.8 (Apache-2.0 OR MIT), `default-features = false` to skip plotting. One bench file per crate, names fixed
so results can be compared over time. Status: ✓ = added in this PR.

| Crate | Bench id | Measures |
|---|---|---|
| core ✓ | `property/get_set_value`, `get_default_value`, `get_name`, `set_position`, `is_a_basepart` | single property access |
| core ✓ | `frame_read/seven_props_per_part/{300,5000}` | what render and physics read per frame |
| core ✓ | `tree/find_first_child_miss_1000`, `full_name`, `create_1000_parts`, `destroy_1000_siblings_one_by_one`, `deep_clone_5000` | tree operations, Play start |
| core ✓ | `scene/save_5000`, `scene/load_5000` | file I/O plus RON |
| core | `changes/changes_since_50_of_5000` | journal read cost (journal landed in PR #59) |
| physics | `step_frame/{parkour_300_20, stress_5000_200}/{falling,settled}` | one 1/60 s frame (2.3 as a bench) |
| physics | `sync_from/5000`, `raycast/5000` | rebuild cost, queries |
| render | `sync/{5000_static,5000_500_moving}`, `upload/5000` | CPU side of rendering (the lane has a release-mode test for 50 changed) |
| render | stress example with `--frames 3600 --json` | p50/p95/p99 frame time on a real GPU (nightly only) |
| script | `vm/create_sandboxed`, `vm/run_script_small`, `vm/hot_loop_20m`, `vm/call_rust_from_luau_1m` | VM costs (2.4 as benches) |
| script | `api/part_position_get_1m`, `api/touched_dispatch_1000` (#7) | API overhead |
| runtime | `game/frame/parkour_headless`, `game/load/parkour`, `game/play_clone/parkour` | whole frame without GPU |
| studio | `--measure-startup` flag printing time to first painted frame | start-up target 3.4 |

### 5.2 Perf-CI gate (proposal)

Wall time on shared CI runners is noise: criterion's author reported "differences of as much as 50 %" on a CI
provider, while Cachegrind instruction counts varied by about 12 instructions in 2.27 billion across VMs (Sourced:
[pythonspeed.com](https://pythonspeed.com/articles/consistent-benchmarking-in-ci)). Tonight's runs on a busy desktop
moved by up to 2.5x. So:

1. **PR gate (GitHub-hosted Linux runner):** instruction-count benches with `gungraun` (formerly iai-callgrind,
   Apache-2.0 OR MIT; needs Valgrind, so Linux only) for core, physics and script. Compare with `main`'s stored
   result. Fail at +5 % instructions on any bench, warn at +2 %. GPU-free crates only. CodSpeed offers the same idea
   as a service with a criterion compatibility crate and claims under 1 % variance (search snippet only); check its
   price for a private repo before choosing it over self-hosting.
2. **Nightly (owner's PC, 00:00-08:00 Chicago, or a dedicated self-hosted runner):** full criterion wall-time suite
   with `--save-baseline nightly-YYYYMMDD`, plus the render stress run. A regression over 10 % against the 7-day best
   opens an issue automatically.
3. **Absolute budgets** (section 3) checked by the runtime bench on the nightly machine: Parkour headless frame p99,
   load time, Play time. Over budget fails the nightly job.

### 5.3 Profiling and telemetry

- In-app: puffin timeline in Studio's developer mode (#14, #23); the same scopes stream to Tracy when built with the
  `tracy` feature. Roblox's MicroProfiler shows the same kind of view (main, worker and GPU threads; 30 frames by
  default) and is the bar to meet (Sourced: [MicroProfiler](https://create.roblox.com/docs/performance-optimization/microprofiler)).
- Frame-time telemetry (player, opt-in, after M2): per session send a histogram of frame times (1 ms buckets to
  50 ms, then coarse), p50/p95/p99, the count of frames over 2x target, tier (GPU name, RAM, cores), resolution and
  quality setting. No per-frame streams, no personal data. Aggregate per game and per tier on the platform side.
- Developer overlay (#14): fps, frame time graph, per-subsystem ms from the profiling scopes, Luau heap per VM, part
  and draw counts.

---

## 6. Proposed issues (not created)

1. **Core: borrow-based property reads.** Add `World::with_value(id, prop, |&Value| ..)` (or typed getters) so hot
   paths read Material/Shape/Name without allocating. Measure with `frame_read`.
2. **Physics: one Rapier step per frame with 4 solver substeps.** Switch from four `dt = 1/240` steps to one
   `dt = 1/60` step (solver substeps stay at 240 Hz); re-run character and platform tests. Numbers in the perf doc 2.3.
3. **Physics: read only changed parts in `step`.** Use the change journal instead of seven `get` calls per body per
   frame (the journal from PR #59 already exists). Bench `step_frame` with a settled 5,000-part level.
4. **Player: present settings and frame cap.** Default to Fifo (vsync) with frame latency 2 and call
   `Renderer::prewarm` before the first visible frame; uncapped only in developer mode (#8).
5. **Script: cheap execution-budget check.** Replace `Instant::now()` per interrupt (2.4x-3.1x slower hot loops)
   with a counter-gated clock now and a watchdog-installed interrupt later; keep the timeout tests green.
6. **Script API: Instance property read budget.** Bench `part.Position` from Luau (about 350 ns today through mlua
   userdata, 17 ns for a table field) and bring it under 150 ns with namecall and per-class lookup tables (#7).
7. **Script: GC steps in frame slack and per-script memory categories.** Step the GC with leftover frame time and tag
   allocations per script for the profiler. Needs `mlua::ffi` for `lua_setmemcat`.
8. **Runtime: profiling scopes and frame budgets.** Add `profiling` scopes (puffin/Tracy back ends behind features)
   and a headless Parkour frame bench that reports p50/p99 per subsystem.
9. **Infra: instruction-count PR gate and nightly wall-time run.** gungraun benches on a Linux runner fail a PR at
    +5 % instructions; nightly criterion and GPU stress on the owner's PC open an issue at +10 %.
10. **Release: `dist` profile and size/start-up tracking.** Thin LTO, one codegen unit, line-table debug info; record
    exe size and cold start in each release note.
11. **Studio: idle CPU and large trees.** Reactive repaint outside Play and drags; virtualised Explorer rows. Target:
    under 1 % CPU idle, 5,000-instance Explorer under 1 ms.
12. **Perf: quiet re-run of the core benches.** Tonight's numbers ran under 88-99 % CPU load from other sessions;
    re-run on an idle PC and update the table in the perf doc.

---

## 7. What I could not verify

- **Iris Xe performance:** not run on such a machine; tier budgets for Low are estimates. TechPowerUp pages returned
  403; TFLOPS figures come from search snippets.
- **Quiet benchmark numbers:** all measurements ran on a PC at 88-99 % CPU load (other sessions building).
- **GPU frame times per pass:** the renderer has no GPU timestamps yet; the GPU table in 3.2 is a plan, not a
  measurement.
- **Roblox internals:** no official per-player bandwidth cap, server heartbeat rate or Parallel Luau worker count;
  the bandwidth figures come from a community guest post, not staff.
- **Valve Source networking page:** 403. The 100 ms interpolation figure is from memory (Judgment).
- **mlua call overhead from others:** no published numbers; only my own scratch measurements in 2.4.
- **Overwatch netcode and Unity DOTS lessons:** the session's web-search budget ran out before I could fetch them.
- **CodSpeed price:** not fetched.
- **Rapier `parallel` and `simd-stable` on our scenes:** not measured.

## 8. Sources

Fetched pages (Sourced):
- Rapier: [IntegrationParameters](https://docs.rs/rapier3d/latest/rapier3d/dynamics/struct.IntegrationParameters.html), [Dimforge 2025 review](https://dimforge.com/blog/2026/01/09/the-year-2025-in-dimforge/); crate source `rapier3d-0.36.1/src/dynamics/solver/staged_island_solver/init.rs` (local cargo registry).
- wgpu: [PipelineCache](https://docs.rs/wgpu/latest/wgpu/struct.PipelineCache.html), [SurfaceConfiguration](https://docs.rs/wgpu/latest/wgpu/type.SurfaceConfiguration.html).
- egui: [README](https://github.com/emilk/egui), [docs.rs](https://docs.rs/egui/latest/egui/) (custom allocator ~20 % faster).
- Luau: [performance](https://luau.org/performance), [sandbox](https://luau.org/sandbox), [lua.h](https://raw.githubusercontent.com/luau-lang/luau/master/VM/include/lua.h), [luaconf.h](https://raw.githubusercontent.com/luau-lang/luau/master/VM/include/luaconf.h), [lgc.cpp](https://raw.githubusercontent.com/luau-lang/luau/master/VM/src/lgc.cpp), [lstate.cpp](https://raw.githubusercontent.com/luau-lang/luau/master/VM/src/lstate.cpp).
- mlua: [Lua struct docs](https://docs.rs/mlua/latest/mlua/struct.Lua.html), [README](https://github.com/mlua-rs/mlua).
- Roblox: [multithreading](https://create.roblox.com/docs/scripting/multithreading), [Actor](https://create.roblox.com/docs/en-us/reference/engine/classes/Actor.md), [Parallel Luau v1](https://devforum.roblox.com/t/full-release-of-parallel-luau-v1/1836187), [native codegen](https://devforum.roblox.com/t/luau-native-code-generation-preview-studio-beta/2572587), [adaptive timestepping](https://create.roblox.com/docs/physics/adaptive-timestepping), [sleep system](https://create.roblox.com/docs/physics/sleep-system), [assemblies](https://create.roblox.com/docs/physics/assemblies), [network ownership](https://create.roblox.com/docs/physics/network-ownership), [streaming](https://create.roblox.com/docs/workspace/streaming), [RemoteEvent](https://create.roblox.com/docs/en-us/reference/engine/classes/RemoteEvent.md), [remote events](https://create.roblox.com/docs/scripting/events/remote), [improve performance](https://create.roblox.com/docs/performance-optimization/improve), [performance design](https://create.roblox.com/docs/performance-optimization/design), [MicroProfiler](https://create.roblox.com/docs/performance-optimization/microprofiler), [memory usage](https://create.roblox.com/docs/studio/optimization/memory-usage), [texture streaming](https://devforum.roblox.com/t/introducing-texture-streaming/4144855), [2021 outage postmortem](https://about.roblox.com/newsroom/2022/01/roblox-return-to-service-10-28-10-31-2021), [guest post on budgets](https://devforum.roblox.com/t/real-world-building-and-scripting-optimization-for-roblox/3127146) (community, not staff).
- Networking: [Gaffer On Games, snapshot compression](https://gafferongames.com/post/snapshot_compression/), [Unreal Replication Graph](https://dev.epicgames.com/documentation/en-us/unreal-engine/replication-graph-in-unreal-engine).
- Hardware and frame pacing: [Steam hardware survey, September 2026](https://store.steampowered.com/hwsurvey/), [Android frame pacing](https://developer.android.com/games/sdk/frame-pacing).
- Build and updates: [Rust perf book, build configuration](https://nnethercote.github.io/perf-book/build-configuration.html), [min-sized-rust](https://github.com/johnthagen/min-sized-rust), [Velopack deltas](https://docs.velopack.io/packaging/deltas).
- Benchmarking: [pythonspeed, benchmarking in CI](https://pythonspeed.com/articles/consistent-benchmarking-in-ci), [Tracy](https://github.com/wolfpld/tracy).
- Bevy: [change detection](https://docs.rs/bevy_ecs/latest/bevy_ecs/change_detection/index.html).
- Backend: [Cloudflare R2 pricing](https://developers.cloudflare.com/r2/pricing/), [Hetzner AX](https://www.hetzner.com/dedicated-rootserver/matrix-ax/), [Agones](https://github.com/googleforgames/agones).
- Crate licences and versions from the crates.io index (`cargo info`): criterion 0.8.2, puffin 0.20.0, puffin_egui 0.30.0, profiling 1.0.18, tracy-client 0.19.0, tracing-tracy 0.12.0, divan 0.1.21, gungraun 0.20.0, iai-callgrind 0.16.1 (all MIT/Apache-2.0), slotmap 1.1.1 (Zlib).

Search snippets only (not fetched): Iris Xe TFLOPS and bandwidth (TechPowerUp, LaptopMedia, Tom's Hardware), CodSpeed
variance claim, Rapier 0.18 small-steps solver notes.
