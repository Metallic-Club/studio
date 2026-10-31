# Multiplayer M2a (local multiplayer) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two processes on one PC or a LAN: a headless `metallic-server` runs the game with authority, `metallic-player`
clients join it over QUIC with locally signed tickets, see each other, predict their own character, call remotes, and
Studio starts a server plus N players with one click; a bot load test prints real per-process numbers.

**Architecture:** `metallic_core` journals what changed and knows what may replicate. `metallic_runtime` gets a
render-free `Simulation` (world, physics, characters, scripts, 60 Hz tick) with a `Role` and a `NetDriver` hook; `Game`
wraps a `Simulation` and keeps camera, avatars, GUI and render. New `metallic_net` (depends on `metallic_core` only)
holds transports, wire encoding, replication, snapshots, prediction buffers, tickets and the editor link. New
`metallic_server` (lib + bin) and `metallic_client` (lib) implement `NetDriver` on top of it; `metallic_player` and
Studio use them.

**Tech Stack:** Rust 1.99 (edition 2024); `quinn` 0.11.12 (`default-features = false`, features `runtime-tokio`,
`rustls-ring`, `log`), `rustls` 0.23.46 with `ring`, `rcgen` 0.14.10 (`default-features = false`, feature `ring`),
`tokio` 1 (`rt`, `net`, `time`, `sync`), `postcard` 1.1.3, `zstd` (BSD-3; version from `cargo add`), `sha2`,
`ed25519-dalek` 2 (BSD-3), `base64` (MIT/Apache), `proptest` 1.11.0 (dev), `libfuzzer-sys` (fuzz crate only),
`windows-sys` 0.60 (Studio Job Object). No `aws-lc-rs`: `cargo tree -i aws-lc-rs` must stay empty.

**Spec:** `docs/superpowers/specs/2026-10-30-multiplayer-m2-design.md` (sections 2-6, 7.3, 10 M2a). Background:
`docs/research/2026-10-30-networking-m2.md` (sections 3, 4, 5, 8). Where the two differ, the spec wins.

## Global Constraints

- Toolchain: Rust 1.99.1 (`rust-toolchain.toml` channel `1.99`), edition 2024, `resolver = "3"`; new crates use
  `version.workspace`, `edition.workspace`, `license.workspace`, `publish.workspace` and go in `[workspace.dependencies]`.
- Code (ARCHITECTURE.md "Quality bar"): `#![warn(missing_docs)]`, a `//!` crate doc, `///` on every public item; no
  `unwrap()`/`expect()` outside tests; typed errors (`CoreError`, `RuntimeError`, `ScriptError`, `StudioError`, new
  `NetError`, `ServerError`); update `ARCHITECTURE.md` (crate list, graph, frame loop) in the PR that changes a public
  interface.
- Checks before every PR, pasted into it: `cargo fmt --check`, `cargo test --workspace -j 4`,
  `cargo clippy --workspace --all-targets -j 4 -- -D warnings`, `cargo deny check licenses`. Daytime (08:00-24:00
  America/Chicago) every cargo command uses `-j 4`; use `tools/cargo_lane.py` with a per-lane target dir; run
  `cargo clean` (or clean the lane target) when the lane's work is done. One `pytest`/test runner at a time by day.
- Licences: permissive only (MIT, Apache-2.0, BSD-2/3, Zlib, ISC, Unicode-3.0, MPL-2.0); no GPL/LGPL/AGPL; no engine
  crates; no code copied from Roblox, Unity, Unreal or Godot.
- CI perf gate (`.github/workflows/performance.yml`, job `instructions`): `crates/metallic_core/examples/perf_ci.rs`
  reads, writes and clone may not grow more than 5 % in instructions. `classes::class_def` is a `match`; every new class
  adds a match arm in `CLASSES` order (test `class_def_matches_the_registry`).
- Git: never configure an identity; commit with
  `git -c user.name=oncoursemail -c user.email=84545815+oncoursemail@users.noreply.github.com commit -m "<msg>"`, the
  message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. One task = one issue = branch
  `studio/<issue#>-m2-a<N>-<slug>` from latest `origin/main` = worktree `../metallic-studio-worktrees/<issue#>-...` =
  one PR (`Closes #N`). Push only your own branch; no force-push; never push `main`. Keep
  `docs/agents/<issue#>-m2-a<N>.md` current. (Owner rule: finished lanes of one wave may be batched into one PR.)
- Secrets: no private IPs, host names, user names or local paths in git, logs, test names or PR text. Tests use
  `127.0.0.1` and `0.0.0.0` only. Tickets, ticket signing keys, join files and the editor-link token are never
  printed, logged or put on a command line visible to other users (token via environment variable); `Debug` of
  ticket and key types redacts.
- No pop-up windows in default test runs: networked players in tests are `--headless`; tests that open a window are
  `#[ignore]`.
- Protocol constants (spec, verbatim): server tick 60 Hz; snapshots every 2 ticks (30 Hz) near, 15 Hz mid, 5 Hz far;
  bands near <= 128 studs, mid <= 512, far <= 1,024, beyond not sent; grid cell 64 studs; datagram <= 1,200 bytes;
  32 baselines; client budget 50 KB/s typical, 100 KB/s peak, up < 10 KB/s; ALPN `metallic/1`; `Hello` within 2 s;
  idle timeout 10 s; reliable client message <= 64 KiB; <= 120 datagrams/s and <= 4 open streams per client;
  close after 100 unknown NetId/property errors in 10 s; `InputCmd` tick within `[last+1, last+8]`, move length <= 1.
- Status words: planned / implemented / tested / accepted. A task is "tested" when its acceptance check ran green;
  say exactly what ran.

## Review Focus

1. **Quantized input everywhere.** If the client predicts with raw `FrameInput` floats while the server steps the
   dequantized `InputCmd`, every frame drifts and corrections never stop. Expected: Combined, server and client all
   step `LocalInput::to_move_input()` of the same quantized bytes. Tests: Task A3 `combined_role_uses_quantized_input`,
   Task A9 `no_corrections_on_a_clean_link`.
2. **Hostile client bytes.** Truncated postcard, unknown message kind, unknown NetId or property index, NaN or
   over-length move, a tick outside `[last+1, last+8]`, a remote call on a non-remote NetId, a 70 KiB frame. Expected:
   drop or close per spec 7.3, never a panic, never a server-side error visible to other players. Tests: Task A6
   `decoders_reject_every_truncation`, Task A9 `bad_input_cmds_are_dropped`, Task A10
   `unknown_net_ids_close_after_100_in_10_s`, Task A5 `oversize_reliable_frame_closes`, Task A14 `bad-packets` bot.
3. **Instances changing visibility.** A Model moved ServerStorage -> Workspace -> ServerStorage, a Ref that points at
   an instance the client cannot see yet, a PlayerGui child reparented to another player's PlayerGui. Expected: the
   client tree matches the server tree minus never-sent parts, and the Ref resolves once the target arrives. Tests:
   Task A7 `reparent_across_visibility_converges`, `refs_resolve_when_target_becomes_visible`.
4. **A player leaving at a bad moment.** During the join snapshot, during an `InvokeClient`, while standing on a mover.
   Expected: `PlayerRemoving` fires once, the invoke errors, the character and its NetIds are gone, no per-client state
   leaks. Tests: Task A7 `disconnect_during_join_cleans_up`, Task A10 `invoke_client_errors_on_disconnect`.
5. **Run-to-run reproducibility.** HashMap iteration order (characters, clients, NetIds) makes seeded network tests
   flaky and server outcomes depend on join order. Expected: characters step in `Players` child order, clients in
   `ClientId` order; the same seed gives the same bytes. Tests: Task A3 `server_steps_characters_in_player_order`,
   Task A7 `same_seed_same_bytes`.

## Decisions this plan makes (spec silent or conflicting; the lead may overturn)

| # | Topic | Ruling | Why |
|---|---|---|---|
| R1 | Journal kinds | `ChangeKind::{Created, Destroyed, Reparented, AncestryChanged, Property(u16)}`: `set_parent` records the moved instance as `Reparented` and its descendants as `AncestryChanged` | Spec lists four kinds, but `changes_since` must keep reporting descendants (physics sync relies on it) and replication must send one reparent, not one per descendant |
| R2 | Script host roles | `ScriptHost` roles, `add_player`/`remove_player`/`set_local_player` and `PlayerRemoving` land in **A3**, not A10 | `Simulation`'s Server role (A3) cannot exist without them; A10 keeps remotes and the new classes' Luau API |
| R3 | Local tickets | A minimal `metallic_net::ticket` (Ed25519, same checks as spec 7.1 step 6, local byte format) lands in **A11** | Spec puts ticket verify in B1 (M2b) but Team Test and `--local` need it now; B1 swaps the byte format for the backend's test vectors |
| R4 | Cert pinning and LAN | Player takes `--cert-sha256 <hex>`; LAN uses `metallic-server --join-file F` / `metallic-player --join-file F` (address, port, cert hash, signing key; player mints its own 60 s ticket) | Spec 6.2's command line has no cert hash to pin, and A13's two-PC check has no way to get tickets |
| R5 | Parkour Finish | The per-player Finish fix and its `print` move into **A11** | A11's check ("server log shows 2 finishes") cannot pass with today's first-finish-only, silent Finish script; A13 does the rest of the audit |
| R6 | NetIds | Monotonic `u64` from 64, never reused; 1-10 reserved for built-in services | Stricter than "not reused for 10 s", no timer, still 2-3 bytes for normal places |
| R7 | Hello deadline | 2 s (spec 7.3) | Research said 5 s; the spec wins |
| R8 | PlayerScripts | Rule 2 of 4.2 applies to `PlayerGui`; there is no `PlayerScripts` class; client LocalScripts keep running from `StarterPlayerScripts` | Matches today's `script_context` |
| R9 | `require` | A10 adds Roblox-style `require(ModuleScript)` (per-VM cache, cycle error) | `ModuleScript` (A2) is useless without it; the sandbox removed only the file-based `require` |
| R10 | Up cap in Play | The 64 KiB client-to-server cap applies in the Combined role too | Play then catches what a real server would refuse |
| R11 | Studio UI | A12 lands `metallic_studio::team_test` (processes, Job Object, editor link, merged log) with its acceptance test; the toolbar button, count picker, Output tabs and network menu attach to the editor of Studio tasks #10/#11 and wait for them | Studio on `main` has no toolbar, Output or Play yet (`docs/STATUS.md`) |
| R12 | Order | A3, A4, A5 start at once (no dependency on A1/A2); A8 before A9; A10 beside A8/A9 | Spec says "A1-A2 first"; the code shows no dependency. A9 consumes A8's client snapshot handling |
| R13 | Bots | `metallic-player --headless --bot B --bots N`: N bot connections per process, no Luau, no window | 200 processes on one PC breaks the PC-load rule; per-connection behaviour is unchanged |
| R14 | Client events | Client role feeds no character/physics `Touched` to scripts; `Humanoid.Died` and `CharacterAdded` come from replicated state | The server is the source of truth; the Parkour HUD needs both |
| R15 | Remote avatars | `Game` draws one skinned avatar per character Model (remote ones from interpolated root, yaw and replicated state) | Character parts are invisible proxies; without this players cannot see each other |
| R16 | Studio warning | "Server Script in a replicated container" waits for the Explorer (#10) | Nothing to show it in yet |

## File map

```
crates/metallic_core/src/instance.rs            A1 (ChangeKind, detailed journal); A2 (SERVICES += ServerStorage)
crates/metallic_core/src/classes.rs             A1 (find_property_index); A2 (Repl, classes, hash, indices)
crates/metallic_core/src/replication.rs         A2 (Audience, filters)
crates/metallic_core/src/value.rs               A2 (orientation_to_quat / quat_to_orientation, moved from physics)
crates/metallic_core/tests/journal.rs, replication_rules.rs, tree.rs
crates/metallic_runtime/src/simulation.rs, net_driver.rs, local_input.rs   A3
crates/metallic_runtime/src/game.rs, lib.rs, camera_controller.rs, Cargo.toml   A3 (A9: avatars, time scale)
crates/metallic_runtime/tests/simulation.rs                                     A3
crates/metallic_script/src/host.rs              A3 (roles, players); A10 (outbox/inbox)
crates/metallic_script/src/api/remote.rs, module.rs, signals.rs, instance.rs    A10
crates/metallic_physics/src/character.rs, lib.rs, sync.rs                       A4 (A2 touches sync.rs once)
crates/metallic_net/                            A5 creates; then:
  src/transport/{mod,memory,quic,conditioner,limits,cert}.rs                    A5
  src/wire/{mod,varint,bits,quant,value,netid,frame,msg,datagram}.rs            A6
  fuzz/                                                                         A6
  src/replication/{mod,server,client,join}.rs                                   A7 (A8 edits server.rs routing)
  src/snapshot/{mod,interest,priority,delta}.rs                                 A8
  src/prediction.rs, interpolation.rs, time_sync.rs                             A9
  src/remote.rs                                                                 A10
  src/ticket.rs                                                                 A11
  src/editor_link.rs                                                            A12
  tests/{transport,wire,replication_events,snapshot,prediction,remote,ticket,editor_link}.rs, tests/support/mod.rs
crates/metallic_server/                         A3 creates the skeleton (lib + bin); then:
  src/{lib,driver,inputs,remotes,snapshots}.rs                                  A7 (stubs), A8/A9/A10 fill their file
  src/{main,args,run,gate,stats}.rs                                             A11 (stats: A14)
  tests/{join,bandwidth,parkour_split}.rs, examples/build_arena.rs              A7, A8, A13, A14
crates/metallic_client/                         A7 creates
  src/{lib,driver,snapshots,prediction,interpolation,remotes,script_events}.rs  A7 stubs; A8 snapshots; A9; A10
  tests/prediction.rs                                                           A9
crates/metallic_player/src/{args,main,net,bots}.rs                              A11, A12 (--editor-link), A14
crates/metallic_studio/src/team_test/{mod,job,link_server,log}.rs, tests/team_test.rs   A12
crates/metallic_runtime/examples/build_parkour/scripts.rs, templates/parkour/*   A11 (Finish), A13
templates/parkour/tests/route.input                                             A9
tools/m2/local_match.py, tools/loadtest/{run.py, arena/}, .github/workflows/ci.yml   A11, A12, A14
docs/luau-api.md                                                                A10
```

---

### Task A1: Core detailed change journal

**Files:**
- Modify: `crates/metallic_core/src/instance.rs` (journal storage, `record*`, `changes_since_detailed`), `src/classes.rs`
  (one function), `src/lib.rs` (exports)
- Test: `crates/metallic_core/tests/journal.rs`

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub enum ChangeKind { Created, Destroyed, Reparented, AncestryChanged, Property(u16) }`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct Change { pub id: InstanceId, pub kind: ChangeKind }`
  - `pub enum DetailedChanges<'a> { Entries(&'a [Change]), Overflow }`
  - `World::changes_since_detailed(&self, cursor: ChangeCursor) -> (DetailedChanges<'_>, ChangeCursor)`: every entry
    after `cursor` in order, not deduplicated; `Overflow` under the same rule as `changes_since` (16,384 entries).
  - `classes::find_property_index(class: &str, property: &str) -> Option<(u16, &'static PropDef)>`: index into
    `all_properties(class)` (base class first; `Name` is 0), found in the same chain walk as `find_property`.
  - `changes_since` keeps its exact contract (deduplicated ids, descendants of a reparent included).

- [ ] **Step 1: Write the failing tests** in `tests/journal.rs`:

```rust
#[test] fn detailed_journal_records_each_kind() {
    // create Part -> [Created(p)]; set Position -> [Property(1)] (Name 0, BasePart.Position 1);
    // same value again -> nothing; rejected set (NaN) -> nothing;
    // Model m with child kid, set_parent(m, None) -> [Reparented(m), AncestryChanged(kid)];
    // destroy(m) -> [Destroyed(m), Destroyed(kid)];
    // clone_subtree(model with 2 parts) -> Created for each copy in tree (pre-)order.
}
#[test] fn property_index_matches_registry_order() {
    assert_eq!(find_property_index("Part", "Name").unwrap().0, 0);
    assert_eq!(find_property_index("Part", "Position").unwrap().0, 1);
    assert_eq!(find_property_index("TextLabel", "Text").map(|p| p.0),
               all_properties("TextLabel").iter().position(|p| p.name == "Text").map(|i| i as u16));
}
#[test] fn detailed_overflows_like_changes_since() { /* 16,385 sets -> Overflow from an old cursor */ }
```

- [ ] **Step 2: Run** `cargo test -p metallic_core --test journal -j 4`. Expected: FAIL (no `changes_since_detailed`).
- [ ] **Step 3: Implement.** Store the journal as two parallel vectors (`journal_revisions: Vec<u64>`,
  `journal: Vec<Change>`) so `Entries` borrows a slice; `set` gets the index from `find_property_index` (no second
  walk); `record_subtree` takes the root kind and records descendants as `AncestryChanged`; `clone_subtree` records
  copies in pre-order (today it uses `HashMap` order). `changes_since` dedupes ids from the new storage.
- [ ] **Step 4: Run** the journal tests, all `metallic_core` tests, `cargo bench -p metallic_core --bench world`
  (compare with `main`: within 5 %), and `cargo build --release -p metallic_core --example perf_ci` locally. Expected:
  PASS; record bench numbers in the PR. The CI `instructions` job must stay within 5 % on writes.
- [ ] **Step 5: Commit** `core: detailed change journal (ChangeKind per entry)`.

### Task A2: Core replication rules, new classes, registry hash

**Files:**
- Modify: `crates/metallic_core/src/classes.rs`, `src/instance.rs` (`SERVICES`), `src/value.rs`, `src/lib.rs`;
  `crates/metallic_physics/src/sync.rs` (call the moved quaternion function)
- Create: `crates/metallic_core/src/replication.rs`
- Test: `crates/metallic_core/tests/replication_rules.rs`, `tests/tree.rs`

**Interfaces:**
- Consumes: A1 `find_property_index`.
- Produces:
  - `pub enum Repl { Replicated, ServerOnly }`; `PropDef { .., pub repl: Repl }` (default `Replicated` through `p()`;
    the seven explicit `PropDef` literals get `repl: Repl::Replicated`). No existing property is `ServerOnly` yet.
  - New classes (append to `CLASSES` and `class_def`, indices 28-31): `ServerStorage` (service, no props),
    `ModuleScript` (superclass `Instance`, prop `Source` String ""), `RemoteFunction` (`Instance`),
    `UnreliableRemoteEvent` (`Instance`; not a subclass of `RemoteEvent`, as in Roblox). `SERVICES` gets
    `"ServerStorage"` appended (after `RunService`).
  - `classes::class_index(name: &str) -> Option<u16>`, `classes::class_at(index: u16) -> Option<&'static ClassDef>`,
    `classes::property_at(class: &str, index: u16) -> Option<&'static PropDef>`.
  - `classes::registry_hash() -> u64`: FNV-1a 64 (offset 0xcbf29ce484222325, prime 0x100000001b3) over, in `CLASSES`
    order, each class name, superclass name, and each own property's name, `ValueType::name()` and `Repl`, with a `0`
    byte after every string. Pure function of the registry; no `std` hasher.
  - `replication::Audience { Never, Player(InstanceId), Everyone }`;
    `replication::audience(world: &World, id: InstanceId) -> Audience` (spec 4.2: under `ServerScriptService` or
    `ServerStorage` -> `Never`; under a `Player`'s `PlayerGui` -> `Player(p)`; under `Workspace`, `ReplicatedStorage`,
    `Players`, `Lighting`, `StarterGui`, `StarterPlayer` -> `Everyone`; parentless or anywhere else -> `Never`);
    `replication::property_replicates(world: &World, id: InstanceId, index: u16) -> bool` (false for `ServerOnly`,
    and for `Source` when the class is exactly `Script`).
  - `value::orientation_to_quat(o: Vec3) -> [f64; 4]` (moved verbatim from `metallic_physics::sync::quat_from_orientation`)
    and its inverse `value::quat_to_orientation(q: [f64; 4]) -> Vec3`.

- [ ] **Step 1: Write the failing tests:** `replication_rules.rs`: `server_containers_are_never_sent`,
  `player_gui_goes_to_its_player_only`, `script_source_never_replicates_but_localscript_and_modulescript_do`
  (Script in Workspace: `Everyone`, `Source` false; LocalScript in StarterPlayerScripts and ModuleScript in
  ReplicatedStorage: `Source` true), `parentless_is_never`; `registry_hash_is_stable` (equals a constant written into
  the test after the first run; same value twice in one run) and `registry_hash_changes_with_a_property` (unit test
  inside `classes.rs` over a copied slice with one extra `PropDef`, so the hash function takes `&[ClassDef]`
  internally); `orientation_round_trip` (100 seeded orientations, `quat_to_orientation(orientation_to_quat(o))` equals
  `o` as a rotation within 1e-9); `tree.rs`: `new_with_services_has_server_storage`.
- [ ] **Step 2: Run** `cargo test -p metallic_core -j 4`. Expected: FAIL.
- [ ] **Step 3: Implement** the items above; physics `sync.rs` calls `metallic_core::value::orientation_to_quat`.
- [ ] **Step 4: Run** `cargo test -p metallic_core -p metallic_physics -j 4` and the `perf_ci` build. Expected: PASS;
  `class_def_matches_the_registry` passes with the new arms.
- [ ] **Step 5: Commit** `core: replication rules, ServerStorage/ModuleScript/RemoteFunction/UnreliableRemoteEvent, registry hash`.

### Task A3: Runtime headless Simulation, roles, NetDriver; script host roles

**Files:**
- Create: `crates/metallic_runtime/src/simulation.rs`, `src/net_driver.rs`, `src/local_input.rs`,
  `crates/metallic_runtime/tests/simulation.rs`; `crates/metallic_server/{Cargo.toml, src/lib.rs, src/main.rs}`
  (skeleton: lib doc + `main` printing usage, exit 2)
- Modify: `crates/metallic_runtime/src/game.rs`, `src/lib.rs`, `src/camera_controller.rs`, `Cargo.toml`;
  `crates/metallic_script/src/host.rs`, `src/api/signals.rs`, `src/api/instance.rs`, `src/lib.rs`

**Interfaces:**
- Produces (runtime):
  - Feature `render` (default) = `dep:metallic_render`, `dep:egui`, `dep:wgpu`, `dep:metallic_anim`; `Game`, avatar,
    camera, GUI and locomotion are `#[cfg(feature = "render")]`. `metallic_server` depends on
    `metallic_runtime = { workspace = true, default-features = false }`.
  - `pub enum Role { Combined, Server, Client }`
  - `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)] pub struct LocalInput { pub move_x: i8, pub move_z: i8, pub jump: bool, pub run: bool, pub yaw: u16 }`;
    `LocalInput::from_frame(input: &FrameInput, camera_yaw: f32) -> LocalInput` (`move_* = round(clamp(v,-1,1)*127)`,
    `yaw = round(rem_euclid(yaw, TAU) / TAU * 65536) mod 65536`); `LocalInput::to_move_input(self) -> MoveInput`
    (dequantize: `m/127`, `y*TAU/65536`, then `move_direction`).
  - `pub fn move_direction(movement: [f32; 2], yaw: f32) -> Vec3` (today's `CameraController::move_direction` math,
    which now calls it).
  - `pub struct TickContext<'a> { pub tick: u64, pub role: Role, pub world: &'a Rc<RefCell<World>>, pub physics: &'a mut PhysicsWorld, pub characters: &'a mut CharacterController, pub scripts: &'a mut ScriptHost, pub moves: &'a mut HashMap<InstanceId, MoveInput>, pub local: LocalInput, pub local_character: &'a mut Option<InstanceId> }`
    with `TickContext::spawn_player(&mut self, user_id: u64, name: &str) -> Result<(InstanceId, InstanceId), RuntimeError>`
    (Player + PlayerGui, StarterGui copy, character, `CharacterAdded`, physics sync) and
    `TickContext::remove_player(&mut self, player: InstanceId)` (`PlayerRemoving`, then destroy character and player).
  - `pub trait NetDriver { fn receive(&mut self, cx: &mut TickContext<'_>) {} fn send(&mut self, cx: &mut TickContext<'_>) {} }`;
    `pub struct NoNet;` (implements it with the defaults).
  - `Simulation::new(world: World, role: Role, output: Rc<dyn OutputSink>) -> Simulation`; `start(&mut self) -> Result<(), RuntimeError>`
    (Combined: today's `Game::start` work incl. `Player1`; Server: server VM only, no player; Client: client VM
    starts after `set_local_player`); `stop`; `tick(&mut self, local: LocalInput, net: &mut dyn NetDriver) -> bool`;
    `tick_count() -> u64`; `time() -> f64`; `role() -> Role`; `world() -> Rc<RefCell<World>>`; `physics() -> &PhysicsWorld`;
    `characters() -> &CharacterController`; `scripts_mut() -> &mut ScriptHost`; `local_player() -> Option<InstanceId>`;
    `local_character() -> Option<InstanceId>`; `spawn_player`, `remove_player` (as on `TickContext`);
    `set_local_player(&mut self, player: InstanceId)`. Client role builds its physics with
    `PhysicsWorld::new_replica()` (A4; until A4 lands, `PhysicsWorld::new()` with a `TODO(A4)` comment).
  - Tick order: Combined inserts `moves[local_character] = local.to_move_input()`; `net.receive`; step characters
    (Combined and Server: every `Player.Character` in `Players` child order with `moves.get(c).copied().unwrap_or_default()`;
    Client: only the local character, and only if `moves` has it); physics step; feed events (not in Client role, R14);
    `time += TICK`; scripts step; physics re-sync; `net.send`; `tick += 1`.
  - `Game` holds `sim: Simulation` and `net: Box<dyn NetDriver>`; `Game::from_world` = Combined + `NoNet` (behaviour
    unchanged); new `Game::networked(name: String, sim: Simulation, net: Box<dyn NetDriver>) -> Game`.
    `Game::tick` passes `LocalInput::from_frame(input, camera_yaw)`.
- Produces (script): `pub enum ScriptRole { Combined, Server, Client }`;
  `ScriptHost::with_role(world, output, role: ScriptRole) -> ScriptHost` (`new` = Combined); Server: server VM only,
  no `Player1`; Client: client VM only. `ScriptHost::add_player(&mut self, player: InstanceId)` (queues `PlayerAdded`),
  `ScriptHost::remove_player(&mut self, player: InstanceId)` (fires `Players.PlayerRemoving(player)` at once, removes
  it from `GetPlayers`), `ScriptHost::set_local_player(&mut self, player: InstanceId)`. New `EventKind::PlayerRemoving`.

- [ ] **Step 1: Write the failing tests** in `crates/metallic_runtime/tests/simulation.rs`:

```rust
#[test] fn combined_simulation_matches_game() { /* Parkour, 600 ticks of the same FrameInput: Simulation(Combined)+NoNet
    root Position == Game root Position exactly */ }
#[test] fn combined_role_uses_quantized_input() {
    let li = LocalInput::from_frame(&FrameInput { movement: [0.5, 1.0], ..Default::default() }, 1.0);
    assert_eq!((li.move_x, li.move_z), (64, 127));
    // Game and Simulation fed the same input move the character identically (both go through LocalInput).
}
#[test] fn server_role_spawns_one_character_per_player() { /* spawn_player(1,"Ann"), spawn_player(2,"Bo"):
    two Player instances named Ann/Bo with UserId 1/2, two Models in Workspace, no "Player1" */ }
#[test] fn server_steps_characters_in_player_order() { /* two runs with the same inputs, players spawned in the same
    order but inserted into `moves` in different HashMap orders -> identical root positions */ }
#[test] fn player_removing_fires_once() { /* server Script counts PlayerRemoving; remove_player -> count 1,
    character Model destroyed */ }
#[test] fn client_role_runs_only_local_scripts() { /* Script in ServerScriptService and LocalScript in
    StarterPlayerScripts both print; Client role after set_local_player prints only the LocalScript line */ }
```

- [ ] **Step 2: Run** `cargo test -p metallic_runtime --test simulation -j 4`. Expected: FAIL.
- [ ] **Step 3: Implement** the interfaces above. `copy_starter_gui` moves from `game.rs` into `simulation.rs`.
- [ ] **Step 4: Run** `cargo test -p metallic_runtime -p metallic_script -j 4` (incl. `parkour_route`,
  `parkour`, `input_scripts`), `cargo build -p metallic_runtime --no-default-features -j 4`, and
  `cargo tree -p metallic_server -e normal | findstr /I "wgpu winit egui"` (Git Bash: `grep -Ei`). Expected: tests
  PASS, build OK, the tree check prints nothing.
- [ ] **Step 5: Commit** `runtime: headless Simulation with roles and NetDriver; script host roles`.

### Task A4: Physics CharacterState save/restore, replica physics

**Files:**
- Modify: `crates/metallic_physics/src/character.rs`, `src/lib.rs`, `src/avatar.rs`, `src/sync.rs`
- Test: `crates/metallic_physics/tests/character.rs`, `tests/physics.rs`

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq)] pub struct CharacterState { pub position: Vec3, pub vertical: f64, pub inherited: Vec3, pub state: HumanoidState, pub ground: Option<InstanceId>, pub yaw: f64, pub dead_time: f64, pub velocity: Vec3, pub relative: Vec3 }`
    (the private `Motion` plus the root position).
  - `CharacterController::save_state(&self, world: &World, character: InstanceId) -> Option<CharacterState>`
  - `CharacterController::restore_state(&mut self, world: &mut World, character: InstanceId, state: &CharacterState)`
    (sets the motion and places the whole avatar at `position`, `yaw`; leaves the touching set alone).
  - `pub fn place_character(world: &mut World, model: InstanceId, root: Vec3, yaw: f64)` (public wrapper of
    `avatar::place`; clients place remote characters' limbs with it).
  - `PhysicsWorld::new_replica() -> PhysicsWorld`: every part is a fixed or kinematic collider that follows the
    World's `Position`/`Orientation`; `step` never integrates bodies (clients never simulate loose parts, spec 4.6).

- [ ] **Step 1: Write the failing tests:** `character.rs`: `replay_600_inputs_from_saved_state_matches` (static test
  level; 600 seeded `MoveInput`s incl. jumps and runs; save at tick 0, run, record root positions; restore, run again:
  every position within 1e-4 stud and the same `HumanoidState`), `restore_moves_root_and_limbs`;
  `physics.rs`: `replica_physics_never_moves_loose_parts` (unanchored part 20 studs up, 60 ticks:
  `Position` unchanged; a raycast still hits it).
- [ ] **Step 2: Run** `cargo test -p metallic_physics -j 4`. Expected: FAIL.
- [ ] **Step 3: Implement.** If the replay differs, find the hidden state (iteration order, accumulated floats) rather
  than loosening the tolerance.
- [ ] **Step 4: Run** `cargo test -p metallic_physics -p metallic_runtime -j 4`. Expected: PASS.
- [ ] **Step 5: Commit** `physics: CharacterState save/restore and replica physics`.

### Task A5: metallic_net transport and channels

**Files:**
- Create: `crates/metallic_net/{Cargo.toml, src/lib.rs, src/error.rs}`,
  `src/transport/{mod.rs, memory.rs, quic.rs, conditioner.rs, limits.rs, cert.rs}`, `tests/transport.rs`
- Modify: root `Cargo.toml` (`[workspace.dependencies] metallic_net`)

**Interfaces:**
- Produces:
  - `pub enum Channel { Control, World, Remote, Bulk }` (reliable). Mapping (research 3.1): `Control` = one bidi
    stream opened by the client; `World` = one long-lived server->client uni stream; `Remote` = one long-lived
    client->server uni stream; `Bulk` = one server->client uni stream per payload. Each uni stream starts with a
    1-byte channel tag; frames on a stream are `varint length + bytes`.
  - `pub enum Incoming { Reliable { channel: Channel, frame: Vec<u8> }, Datagram(Vec<u8>), Closed(CloseReason) }`;
    `pub enum CloseReason { Normal, Timeout, ProtocolError(String), Kicked(String), TransportError(String) }`.
  - `pub trait Connection { fn send_reliable(&mut self, channel: Channel, frame: Vec<u8>) -> Result<(), NetError>; fn send_datagram(&mut self, bytes: Vec<u8>) -> Result<(), NetError>; fn poll(&mut self, out: &mut Vec<Incoming>); fn rtt(&self) -> Duration; fn stats(&self) -> LinkStats; fn close(&mut self, reason: CloseReason); fn remote_addr(&self) -> Option<SocketAddr>; }`
    (non-blocking; the game loop calls `poll` once per tick).
  - `pub trait Listener { fn accept(&mut self) -> Option<Box<dyn Connection>>; }`
  - `#[derive(Default)] pub struct LinkStats { pub bytes_sent: u64, pub bytes_received: u64, pub datagrams_dropped: u64, pub datagrams_received: u64 }`
  - `#[derive(Clone, Copy, Debug, Default, PartialEq)] pub struct LinkConditioner { pub latency: Duration /* one way */, pub jitter: Duration, pub loss: f32 /* 0..=1 */, pub seed: u64 }`
  - `pub struct Sequencer` with `accept(&mut self, seq: u16) -> bool` (wrap-aware: newer when `seq.wrapping_sub(newest) as i16 > 0`).
  - Limits (`limits.rs`, spec 7.3): `MAX_DATAGRAM = 1200`, `MAX_CLIENT_FRAME = 64 * 1024`, `MAX_SERVER_FRAME = 4 << 20`,
    `CLIENT_DATAGRAMS_PER_SEC = 120`, `CLIENT_MAX_STREAMS = 4`, `IDLE_TIMEOUT = 10 s`, `ALPN = b"metallic/1"`.
    The server side enforces them: oversize client frame closes with `ProtocolError`; excess datagrams are dropped
    and counted; `send_datagram` over 1,200 bytes returns `NetError::DatagramTooLarge`.
  - Memory: `MemoryNet::new(seed: u64) -> MemoryNet` (shared manual clock); `MemoryNet::listener(&self) -> Box<dyn Listener>`;
    `MemoryNet::connect(&self, conditioner: LinkConditioner) -> Box<dyn Connection>`; `MemoryNet::advance(&self, dt: Duration)`.
    Reliable sends lost with probability `loss` are re-sent after one RTT and delivered in order; datagrams are lost
    or reordered by jitter, never re-sent.
  - QUIC: `QuicServer::bind(addr: SocketAddr, conditioner: Option<LinkConditioner>) -> Result<QuicServer, NetError>`
    (own tokio runtime thread; fresh `rcgen` ECDSA P-256 self-signed cert valid 13 days; ALPN; idle timeout 10 s;
    migration on; `max_concurrent_uni_streams(4)`, bidi 1 for clients); `QuicServer::local_addr()`,
    `QuicServer::cert_sha256() -> [u8; 32]`, `impl Listener`.
    `QuicClient::connect(addr: SocketAddr, pin: [u8; 32], timeout: Duration) -> Result<Box<dyn Connection>, NetError>`
    with a rustls `ServerCertVerifier` that accepts exactly the end-entity DER whose SHA-256 is `pin` (no CA, no DNS;
    validity window checked). The conditioner on QUIC wraps the UDP socket (`quinn::AsyncUdpSocket`) so QUIC itself
    sees loss and delay.

- [ ] **Step 1: Write the failing tests** in `tests/transport.rs` (memory unless named `quic_*`):
  `reliable_stream_in_order_at_30_percent_loss` (1,000 frames, 40 ms latency, 10 ms jitter, loss 0.3: all arrive, in
  order), `stale_datagrams_dropped` (`Sequencer` over a jittered stream never accepts an older seq; wrap at 65,535),
  `datagram_rate_limited_to_120_per_second` (200 sent in 1 s: <= 120 received, `datagrams_dropped >= 80`),
  `oversize_reliable_frame_closes` (70 KiB from client -> `Closed(ProtocolError)`), `datagram_over_1200_refused`,
  `quic_round_trip_on_localhost` (bind `127.0.0.1:0`, connect with the right pin, both channels and datagrams work),
  `quic_wrong_pin_is_refused`, `quic_fifth_client_stream_refused`.
- [ ] **Step 2: Run** `cargo test -p metallic_net --test transport -j 4`. Expected: FAIL.
- [ ] **Step 3: Implement.** Check `cargo tree -p metallic_net -i aws-lc-rs` is empty.
- [ ] **Step 4: Run** the tests and `cargo deny check licenses`. Expected: PASS.
- [ ] **Step 5: Commit** `net: Transport trait, QUIC and in-memory transports, link conditioner, limits`.

### Task A6: metallic_net messages and encoding

**Files:**
- Create: `crates/metallic_net/src/wire/{mod.rs, varint.rs, bits.rs, quant.rs, value.rs, netid.rs, frame.rs, msg.rs, datagram.rs}`,
  `crates/metallic_net/fuzz/{Cargo.toml (own [workspace]), fuzz_targets/reliable_frame.rs, snapshot_datagram.rs, input_datagram.rs}`,
  `tests/wire.rs`
- Modify: `crates/metallic_net/src/lib.rs`, `Cargo.toml` (postcard, serde, zstd; dev proptest)

**Interfaces:**
- Consumes: A2 `class_index`, `property_at`, `registry_hash`, `orientation_to_quat`/`quat_to_orientation`.
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)] pub struct NetId(pub u64)`;
    `pub const FIRST_DYNAMIC_NET_ID: u64 = 64`; `netid::service_net_id(class: &str) -> Option<NetId>`: Workspace 1,
    Players 2, ReplicatedStorage 3, ServerScriptService 4, StarterPlayer 5, StarterGui 6, Lighting 7, RunService 8,
    ServerStorage 9, StarterPlayerScripts 10.
  - `pub enum NetValue { Nil, Bool(bool), Number(f64), Str(Vec<u8>), Vector3(Vec3), Vector2([f64; 2]), Color3(Color3), UDim2(UDim2), Ref(NetId), Table(Vec<(NetValue, NetValue)>) }`
    with `NetValue::from_value(v: &Value, net: impl Fn(InstanceId) -> Option<NetId>) -> NetValue` and
    `NetValue::to_value(&self, ty: ValueType, local: impl Fn(NetId) -> Option<InstanceId>) -> Option<Value>`.
  - Reliable frame (`frame.rs`): `[kind: u8][source: u8 = 0][varint body length][postcard body]`;
    `encode_server(msg: &ServerMsg) -> Vec<u8>`, `decode_server(bytes: &[u8]) -> Result<Decoded<ServerMsg>, WireError>`,
    same for `ClientMsg`; `Decoded::{Msg { source: u8, msg }, UnknownKind(u8)}` (unknown kinds are skipped and logged
    once; spec/research 3.3).
  - `ClientMsg` kinds: 1 `Hello { ticket: String, protocol_minor: u16, engine_version: String, registry_hash: u64 }`,
    2 `FireServer { remote: NetId, args: Vec<NetValue> }`, 3 `InvokeServer { call: u32, remote: NetId, args: Vec<NetValue> }`,
    4 `InvokeClientReply { call: u32, result: Result<Vec<NetValue>, String> }`, 5 `Bye`.
  - `ServerMsg` kinds: 1 `Welcome { player: NetId, tick: u64, protocol_minor: u16 }`, 2 `UpdateRequired { registry_hash: u64 }`,
    3 `Kick { reason: String }`, 4 `JoinSnapshot { tick: u64, zstd: Vec<u8> }` (body = postcard `Vec<Create>`, zstd
    level 3), 5 `Create { net: NetId, class: u16, parent: Option<NetId>, props: Vec<(u16, NetValue)> }`,
    6 `Destroy { net: NetId }`, 7 `Reparent { net: NetId, parent: Option<NetId> }`, 8 `Set { net: NetId, props: Vec<(u16, NetValue)> }`,
    9 `FireClient { remote: NetId, args: Vec<NetValue> }`, 10 `InvokeClient { call: u32, remote: NetId, args: Vec<NetValue> }`,
    11 `InvokeServerReply { call: u32, result: Result<Vec<NetValue>, String> }`.
    `PROTOCOL_MINOR: u16 = 0`.
  - `bits.rs`: `BitWriter::{new, write(value: u64, bits: u8), write_bool, finish() -> Vec<u8>}`, `BitReader::{new(&[u8]), read(bits: u8) -> Result<u64, WireError>, read_bool}`; `varint.rs`: LEB128 `u64` and zigzag `i64`.
  - `quant.rs`: `QuantPos { cell_x: i32, cell_z: i32, x: u16 /*12 bits*/, z: u16 /*12 bits*/, y: QuantY }` with
    `QuantY::{Fixed(i32 /*20-bit signed, 1/64 stud*/), Raw(f32)}`; `quantize_position(Vec3) -> QuantPos`,
    `dequantize_position(&QuantPos) -> Vec3` (64-stud cells, 1/64 stud, rounding to nearest: error <= 1/128);
    `quantize_orientation(Vec3) -> u32` (smallest-three: 2-bit index + 3 x 10 bits),
    `dequantize_orientation(u32) -> Vec3`; `quantize_velocity(Vec3) -> [i16; 3]` (12 bits signed, 1/8 stud/s, clamped
    to +-255.875), `dequantize_velocity`.
  - `datagram.rs`: header `minor: 4 bits, kind: 4 bits, seq: u16`; kinds 1 `Snapshot`, 2 `Input`, 3 `UnreliableRemote`.
    `pub struct InputCmd { pub tick: u32, pub move_x: i8, pub move_z: i8, pub jump: bool, pub run: bool, pub yaw: u16, pub actions: u16 /* reserved, 0 in M2a */ }`;
    `pub struct InputPacket { pub seq: u16, pub ack: u16, pub ack_bits: u32, pub cmds: Vec<InputCmd> /* 1..=8, consecutive ticks */ }`;
    `pub struct OwnState { pub position: [f32; 3], pub vertical: f32, pub inherited: [f32; 3], pub state: u8, pub ground: Option<NetId>, pub yaw: f32, pub dead_time: f32, pub velocity: [f32; 3], pub relative: [f32; 3] }`;
    `pub struct EntityState { pub net: NetId, pub pos: QuantPos, pub rot: EntityRot, pub vel: Option<[i16; 3]>, pub at_rest: bool }`
    with `EntityRot::{Part(u32), Character { yaw: u16, state: u8 }}`;
    `pub struct SnapshotPacket { pub seq: u16, pub source: u8, pub server_tick: u32, pub last_input_tick: u32, pub buffer_hint: i8, pub own: Option<OwnState>, pub entities: Vec<EntityState> }`;
    `encode_snapshot(&SnapshotPacket) -> Vec<u8>`, `decode_snapshot(&[u8]) -> Result<SnapshotPacket, WireError>`,
    `encode_input`, `decode_input`; `UnreliableRemotePacket { seq: u16, remote: NetId, args: Vec<NetValue> }`
    (args postcard, total <= 1,000 bytes).
  - `pub enum WireError { Truncated, UnknownKind(u8), TooLarge, BadValue(&'static str), Postcard(String) }`.

- [ ] **Step 1: Write the failing tests** in `tests/wire.rs` (proptest): `server_and_client_messages_round_trip`,
  `bits_round_trip`, `varint_round_trip`, `position_error_at_most_1_128` (positions in +-100,000 studs XZ,
  +-8,000 Y), `orientation_error_under_quarter_degree` (as a rotation angle), `velocity_round_trip_within_1_16`,
  `snapshot_and_input_packets_round_trip`, `decoders_reject_every_truncation` (every prefix of a valid frame or
  datagram returns `Err`, never panics), `unknown_kind_is_skipped`, `values_convert_both_ways`
  (`Value` -> `NetValue` -> `Value` for every `ValueType`; a Ref without a NetId becomes `Ref(None)`).
- [ ] **Step 2: Run** `cargo test -p metallic_net --test wire -j 4`. Expected: FAIL.
- [ ] **Step 3: Implement.** Fuzz targets call the three decoders on arbitrary bytes.
- [ ] **Step 4: Run** the tests and `cargo check --manifest-path crates/metallic_net/fuzz/Cargo.toml -j 4` (stable;
  running `cargo fuzz` nightly is M2b). Expected: PASS.
- [ ] **Step 5: Commit** `net: postcard messages, NetValue, NetIds, bit packing, quantization, fuzz targets`.

### Task A7: Replication: join snapshot, event path, driver skeletons

**Files:**
- Create: `crates/metallic_net/src/replication/{mod.rs, server.rs, client.rs, join.rs}`, `tests/replication_events.rs`,
  `tests/support/mod.rs`; `crates/metallic_server/src/{driver.rs, inputs.rs, remotes.rs, snapshots.rs}`;
  `crates/metallic_client/{Cargo.toml, src/lib.rs, src/driver.rs, src/snapshots.rs, src/prediction.rs, src/interpolation.rs, src/remotes.rs, src/script_events.rs}`;
  `crates/metallic_server/tests/join.rs`
- Modify: `crates/metallic_server/src/lib.rs`, `Cargo.toml`; root `Cargo.toml`

**Interfaces:**
- Consumes: A1 `changes_since_detailed`; A2 `audience`, `property_replicates`, `class_index`; A3 `NetDriver`,
  `TickContext`, `Simulation`; A5 `Connection`, `Listener`, `MemoryNet`; A6 messages, `NetId`, `NetValue`.
- Produces (`metallic_net::replication`):
  - `pub struct ClientId(pub u32)` (ordered; iteration always by `ClientId`).
  - `ReplicationServer::new(world: &World) -> Self`;
    `add_client(&mut self, client: ClientId, player: InstanceId, world: &World, tick: u64) -> Vec<u8>` (the
    `JoinSnapshot` frame: every instance visible to `player`, pre-order, non-default replicated props only; the
    client's event stream continues from `tick + 1`);
    `remove_client(&mut self, client: ClientId)`;
    `collect(&mut self, world: &World, tick: u64)` (once per tick after scripts: read the detailed journal, drop
    never-sent (4.2), route, coalesce to the last value per `(instance, property)`, compute per-client visibility
    transitions against the client's known set (unknown -> visible: `Create` the subtree; known -> invisible:
    `Destroy`; known -> visible: `Reparent`/`Set`), encode each message once and copy the bytes into each client's
    queue; on `Overflow` do a full compare for every client and count it);
    `queue_for(&mut self, client: Option<ClientId>, frame: Vec<u8>)` (A10 puts `FireClient` frames here so they stay
    ordered with tree changes; `None` = every client);
    `drain_frames(&mut self, client: ClientId, out: &mut Vec<Vec<u8>>)`; `net_id(&self, InstanceId) -> Option<NetId>`;
    `instance(&self, NetId) -> Option<InstanceId>`; `metrics(&self) -> ReplMetrics { overflows: u64, frames: u64, bytes: u64 }`.
    NetIds per R6. Routing hook `fn route(&self, world, id, index) -> Route::{Event, Snapshot, Drop}` returns `Event`
    for everything in A7 (A8 changes it).
  - `ReplicaApplier::new(world: &mut World) -> Self` (maps service NetIds to the replica's built-in roots);
    `apply(&mut self, world: &mut World, msg: ServerMsg) -> Result<Applied, ApplyError>`
    (`Applied::{None, Welcome { player: InstanceId, tick: u64 }, Remote(ServerMsg)}`; `World` frames that arrive
    before the `JoinSnapshot` are buffered and applied after it; every Ref property remembers its target `NetId` and
    resolves when that NetId is created later); `local(&self, NetId) -> Option<InstanceId>`; `net(&self, InstanceId) -> Option<NetId>`.
- Produces (drivers):
  - `metallic_server::JoinGate` trait: `fn check(&mut self, ticket: &str, now: SystemTime) -> Result<JoinClaims, JoinRefused>`;
    `pub struct JoinClaims { pub user_id: u64, pub name: String }`; `AcceptAllGate` for tests (ticket `"<user_id>:<name>"`).
  - `ServerDriver::new(listener: Box<dyn Listener>, gate: Box<dyn JoinGate>, config: ServerConfig) -> Self`,
    `ServerConfig { max_players: usize, hello_deadline: Duration /* 2 s */ }`; `impl NetDriver`. `receive`: accept,
    `Hello` within 2 s else close, registry hash mismatch -> `UpdateRequired` + close, gate -> `Kick`, full ->
    `Kick("server full")`, else `cx.spawn_player`, `Welcome`, join snapshot on `Bulk`; decode `Input` datagrams and
    hand `InputCmd`s to `inputs.rs` (A7: newest cmd per player becomes `moves[character]`); closed connection ->
    `cx.remove_player`. `send`: `repl.collect`, flush frames, call the `snapshots.rs` and `remotes.rs` hooks (stubs).
  - `ClientDriver::new(conn: Box<dyn Connection>, hello: ClientMsg) -> Self`; `status(&self) -> ClientStatus::{Joining, Playing, Closed(CloseReason)}`;
    `impl NetDriver`: `receive` applies frames to `cx.world`, on `Welcome` calls `cx.scripts.set_local_player`, keeps
    `*cx.local_character` = the local `Player.Character`; `send` sends one `InputPacket` per tick with
    `InputCmd` from `cx.local` (A9 adds prediction and redundancy).

- [ ] **Step 1: Write the failing tests** in `crates/metallic_net/tests/replication_events.rs` (harness in
  `tests/support/mod.rs`: seeded xorshift edit generator, `tree_equal(server, client, player) -> Result<(), String>`
  comparing visible trees by class, name, order, replicated props, Refs by identity, Refs to unseen instances as nil):

```rust
#[test] fn three_clients_converge_after_random_edits() {
    for latency_ms in [0, 100, 250] { for loss in [0.0, 0.02, 0.10] {
        // 3 clients, 10,000 edits over 600 ticks (create/destroy/reparent incl. ServerStorage <-> Workspace,
        // set Name/Position/Color/Anchored/Source/Player.RespawnLocation refs), then 60 quiet ticks:
        for k in 0..3 { support::tree_equal(&server, &clients[k], players[k]).unwrap(); }
    }}
}
#[test] fn server_only_content_never_on_the_wire() { /* sentinels in ServerStorage names and Script.Source never
    appear in any frame, join snapshot decompressed */ }
#[test] fn player_gui_goes_only_to_its_player() {}
#[test] fn reparent_across_visibility_converges() {}
#[test] fn refs_resolve_when_target_becomes_visible() {}
#[test] fn late_join_equals_early_join() {}
#[test] fn overflow_falls_back_to_full_compare() { /* 20,000 sets in one tick: converge; metrics().overflows == 1 */ }
#[test] fn same_seed_same_bytes() {}
```

  and `crates/metallic_server/tests/join.rs`: `two_clients_join_parkour_and_see_each_other` (server `Simulation`
  + `ServerDriver` on `MemoryNet`, two client `Simulation`s with `ClientDriver`; after 120 ticks each replica has
  two character Models; client 1 walks forward 60 ticks; client 2's copy of client 1's root is within 0.01 stud of the
  server's after 30 quiet ticks), `wrong_registry_hash_gets_update_required`, `no_hello_closes_at_2_s`,
  `disconnect_during_join_cleans_up` (connection dropped right after `Welcome`: `PlayerRemoving` once, no Player,
  no character, `ReplicationServer` has no state for the client).
- [ ] **Step 2: Run** `cargo test -p metallic_net --test replication_events -j 4` and
  `cargo test -p metallic_server --test join -j 4`. Expected: FAIL.
- [ ] **Step 3: Implement.** Stubs: `metallic_server::{snapshots, remotes}` and
  `metallic_client::{snapshots, prediction, interpolation, remotes, script_events}` each expose one no-op struct with
  the hook methods the drivers call, so A8/A9/A10 change only their own file.
- [ ] **Step 4: Run** both tests plus `cargo test --workspace -j 4`. Expected: PASS.
- [ ] **Step 5: Commit** `net: replication event path and join snapshot; server and client driver skeletons`.

### Task A8: Snapshots: deltas, acks, priority, interest grid, bands

**Files:**
- Create: `crates/metallic_net/src/snapshot/{mod.rs, interest.rs, priority.rs, delta.rs}`, `tests/snapshot.rs`,
  `crates/metallic_server/tests/bandwidth.rs`
- Modify: `crates/metallic_net/src/replication/server.rs` (routing), `tests/support/mod.rs` (pump snapshots),
  `crates/metallic_server/src/snapshots.rs`, `crates/metallic_client/src/snapshots.rs`

**Interfaces:**
- Consumes: A6 `SnapshotPacket`, `EntityState`, quantization; A7 `ReplicationServer`, driver hooks.
- Produces:
  - Routing (`route`): `Position`/`Orientation` of a BasePart under Workspace go to `Snapshot` when it is a character
    `HumanoidRootPart`, unanchored, or a mover (transform changed in each of the last 2 ticks; stops being one after
    60 ticks without change, then its last value also goes once on the event path); other character parts' transforms
    are `Drop` (clients place limbs, spec 4.5); everything else `Event`.
  - `InterestGrid::new(cell: f64 /* 64 */)`, `update(&mut self, net: NetId, pos: Vec3)`, `remove`,
    `query(&self, focus: Vec3, radius: f64 /* 1,024 */, out: &mut Vec<(NetId, Band)>)` nearest cells first (focus =
    the client's character root; `Player.ReplicationFocus` is not in the registry in M2a);
    `pub enum Band { Near /* <=128, every 2 ticks */, Mid /* <=512, every 4 */, Far /* <=1,024, every 12 */ }`.
  - `PriorityAccumulator`: per client per entity `add(weight)`; weight = class (character root 4, unanchored 2,
    mover 1) x band (near 1.0, mid 0.5, far 0.2) x boost (2.0 within 10 studs of the focus or touched the client's
    character in the last 60 ticks, else 1.0); entities are candidates only on their band's ticks; send highest first
    until the budget is used; reset what was sent.
  - `DeltaTracker`: per client a ring of the last 32 sent snapshots (seq -> per-entity quantized state); `ack(ack: u16, ack_bits: u32)`;
    an entity whose quantized state equals the newest acked baseline is omitted; no acked baseline -> full values;
    `at_rest` set once, then the entity is silent until it moves.
  - `SnapshotServer::build(&mut self, client: ClientId, focus: Vec3, own: Option<OwnState>, last_input_tick: u32, buffer_hint: i8, world: &World, repl: &ReplicationServer, velocity: &dyn Fn(InstanceId) -> Option<Vec3>, out: &mut Vec<Vec<u8>>)`:
    every 2 ticks, one datagram <= 1,200 bytes, a second if the budget allows; budget 50,000 B/s = 1,666 bytes per
    snapshot. The own character is always first and full precision.
  - Client `snapshots.rs`: decode, `Sequencer`, ack bookkeeping (`ack`, `ack_bits` go out in the next
    `InputPacket`), apply remote entities to the replica World (A9 replaces direct apply with interpolation).

- [ ] **Step 1: Write the failing tests:** `crates/metallic_net/tests/snapshot.rs`: `bands_by_distance` (127 near,
  129 mid, 513 far, 1,025 none), `nothing_starves` (300 far entities, tiny budget: every entity sent within 10 s),
  `acked_unchanged_entities_are_omitted`, `delta_survives_10_percent_loss` (client state equals server quantized state
  after a quiet second), `datagrams_never_exceed_1200`; `crates/metallic_server/tests/bandwidth.rs`:

```rust
#[test] fn twenty_four_bots_on_parkour_stay_under_budget() {
    // Server Simulation + 24 in-process input bots (walk the route or wander) over MemoryNet, 60 s of play,
    // measured after every bot joined (join snapshots excluded):
    for c in &clients { assert!(c.avg_down_bytes_per_sec() <= 50_000.0); assert!(c.peak_down_1s() <= 100_000.0);
                        assert!(c.avg_up_bytes_per_sec() < 10_000.0); }
}
```

  The A7 test `three_clients_converge_after_random_edits` must still pass with snapshots pumped.
- [ ] **Step 2: Run** `cargo test -p metallic_net --test snapshot -j 4` and
  `cargo test -p metallic_server --test bandwidth -j 4`. Expected: FAIL.
- [ ] **Step 3: Implement.** Print the measured averages and peaks in the bandwidth test output.
- [ ] **Step 4: Run** both plus `cargo test -p metallic_net -p metallic_server -j 4`. Expected: PASS.
- [ ] **Step 5: Commit** `net: delta snapshots with acks, priority accumulator, interest grid and bands`.

### Task A9: Prediction and interpolation

**Files:**
- Create: `crates/metallic_net/src/{prediction.rs, interpolation.rs, time_sync.rs}`, `tests/prediction.rs`;
  `crates/metallic_client/tests/prediction.rs`; `templates/parkour/tests/route.input`
- Modify: `crates/metallic_server/src/inputs.rs`, `crates/metallic_client/src/{prediction.rs, interpolation.rs, driver.rs}`
  (driver: only the calls into these two), `crates/metallic_runtime/src/game.rs` (avatars per character, time scale,
  render offset), `src/net_driver.rs` (two default methods), `crates/metallic_runtime/tests/parkour_route.rs`

**Interfaces:**
- Consumes: A3 `LocalInput`, `NetDriver`; A4 `save_state`/`restore_state`, `place_character`; A6 `InputCmd`,
  `OwnState`; A8 client snapshot decode.
- Produces:
  - `InputHistory<S: Copy>`: ring of 120 entries (2 s) of `(tick: u32, cmd: InputCmd, state: S)`;
    `push`, `discard_through(tick)`, `iter_after(tick)`.
  - Client sends 30 input packets/s (every 2 ticks) carrying the 2 newest cmds plus up to 6 older unacknowledged.
  - Server `inputs.rs`: `InputBuffer` per player, target 2 ticks; one cmd per tick; validation (spec 7.3): finite,
    `sqrt(mx^2+mz^2) <= 127` (move length <= 1), tick in `[last+1, last+8]` else dropped and counted (10 drops in a
    second -> kick "bad input"); empty buffer -> reuse last, count a starve; `buffer_hint = target - depth` clamped
    to +-8; `last_input_tick` per player.
  - `ServerClock`: filtered estimate of server tick from snapshots; the client's sim leads by `RTT/2 + input buffer`;
    `time_scale()` in `0.95..=1.05` from `buffer_hint`. `Game` multiplies frame `dt` by
    `NetDriver::time_scale(&self) -> f64 { 1.0 }`.
  - `Predictor` (client `prediction.rs`): each tick builds `InputCmd` from `cx.local`, sets
    `moves[local character] = LocalInput::to_move_input()` of exactly those bytes (Review Focus 1), stores
    `(tick, cmd, save_state)` after the step. On a snapshot with `own`: compare with history at `last_input_tick`;
    within 0.01 stud and the same `HumanoidState` -> discard through it; else `restore_state`, replay later inputs
    (drain and drop the controller's events, R14), count a correction, and add the visual difference to a render
    offset that decays 10 % per frame (`NetDriver::render_offset(&self) -> Vec3`). Mover rule (spec 4.7.5): when
    the predicted character's ground part is a mover, extrapolate that part to the predicted tick from its replicated
    velocity before stepping.
  - `Interpolator` (`metallic_net::interpolation`): per NetId buffer of `(server_tick, pos, vel, rot)`;
    `sample(render_tick: f64) -> Option<Sample>`: Hermite position with velocity, slerp orientation, extrapolate at
    most one interval (33 ms) then hold. Delay = 2 snapshot intervals + measured jitter, clamped 50-200 ms.
    Client `interpolation.rs` applies samples every frame: parts' `Position`/`Orientation`, remote characters via
    `place_character`; exposes `NetDriver::character_motion(&self, character: InstanceId) -> Option<(HumanoidState, Vec3)>`.
  - `Game`: `avatars: HashMap<InstanceId, Avatar>` for every character Model (R15); local one from
    `CharacterController`, remote ones from `character_motion`.
  - `templates/parkour/tests/route.input`: the closed-loop route bot's recorded input script, written by
    `parkour_route.rs` when `METALLIC_WRITE_ROUTE=1`; a test asserts the committed file finishes the course in Combined.

- [ ] **Step 1: Write the failing tests:** `crates/metallic_net/tests/prediction.rs`: `history_keeps_two_seconds`,
  `interpolation_hermite_and_hold` (hold after 33 ms of missing data), `interp_delay_clamped_50_200_ms`;
  server `inputs.rs` unit tests `bad_input_cmds_are_dropped` (cmd fields are integers, so "finite" holds by
  construction; test move length > 1, tick `last+9`, a duplicate tick), `starve_reuses_last_and_counts`; `crates/metallic_client/tests/prediction.rs`:

```rust
#[test] fn no_corrections_on_a_clean_link() { /* 0 ms, 0 % loss, 10 s flat walk: corrections == 0 */ }
#[test] fn parkour_run_at_100ms_2pct_loss_matches_server() {
    // MemoryNet 50 ms each way, loss 0.02, seed 7; client replays templates/parkour/tests/route.input;
    // server's root enters the Finish pad's bounds; at the end the client's predicted root equals the server's root
    // at the same tick within 0.01 stud; corrections whose ground part is static: < 10 per minute.
}
#[test] fn remote_player_is_interpolated_smoothly() { /* no frame-to-frame jump > 2x walk speed * dt */ }
```

  and in `crates/metallic_runtime/tests/parkour_route.rs`: `committed_route_input_finishes_the_course`.
- [ ] **Step 2: Run** `cargo test -p metallic_net --test prediction -j 4`, `cargo test -p metallic_client -j 4`.
  Expected: FAIL.
- [ ] **Step 3: Implement.** Generate the route file once with `METALLIC_WRITE_ROUTE=1 cargo test -p metallic_runtime
  --test parkour_route -j 4` and commit it.
- [ ] **Step 4: Run** `cargo test -p metallic_net -p metallic_client -p metallic_server -p metallic_runtime -j 4`.
  Expected: PASS; print correction counts in the test output.
- [ ] **Step 5: Commit** `client: prediction with replay, input buffer, time sync, interpolation, remote avatars`.

### Task A10: Script host roles and remotes over the wire

**Files:**
- Create: `crates/metallic_script/src/api/module.rs`, `crates/metallic_net/src/remote.rs`, `tests/remote.rs`
  (in `metallic_net`), `crates/metallic_server/tests/remotes.rs`
- Modify: `crates/metallic_script/src/api/{remote.rs, signals.rs, instance.rs, mod.rs}`, `src/host.rs`, `src/lib.rs`,
  `crates/metallic_script/tests/{remote.rs, limits.rs}`, `crates/metallic_server/src/remotes.rs`,
  `crates/metallic_client/src/{remotes.rs, script_events.rs}`, `docs/luau-api.md`

**Interfaces:**
- Consumes: A3 `ScriptRole`; A6 messages; A7 `queue_for`, `ReplicaApplier` `Applied::Remote`.
- Produces (script):
  - `pub use api::remote::Wire` (made `pub`, unchanged variants).
  - `pub struct OutgoingRemote { pub remote: InstanceId, pub call: RemoteCall, pub args: Vec<Wire> }`,
    `pub enum RemoteCall { FireServer, FireClient(InstanceId), FireAllClients, InvokeServer(u32), InvokeClient(u32, InstanceId), Reply(u32), ReplyError(u32, String), UnreliableFireServer, UnreliableFireClient(InstanceId), UnreliableFireAllClients }`;
    `ScriptHost::drain_outgoing(&mut self, f: impl FnMut(OutgoingRemote))` and
    `ScriptHost::deliver_incoming(&mut self, sender: Option<InstanceId>, remote: InstanceId, call: RemoteCall, args: Vec<Wire>)`.
    Combined role keeps in-process delivery.
  - Luau (Roblox names): `RemoteFunction:InvokeServer/InvokeClient`, `OnServerInvoke`, `OnClientInvoke` (callback
    properties); `UnreliableRemoteEvent:FireServer/FireClient/FireAllClients`, `OnServerEvent`, `OnClientEvent`;
    `require(module: ModuleScript)` (R9). Limits (spec 4.9, verbatim): 64 KiB per call up (all roles, R10; error
    "remote payload too large" on the caller), 1 MiB down; `UnreliableRemoteEvent` 1,000 bytes, never queued,
    sequenced per remote; `InvokeServer` 30 s timeout error; `InvokeClient` 10 s timeout, errors on timeout or
    disconnect.
- Produces (net): `RemoteLimiter` per client: 500 calls/s per remote type and 50 KB/s with a 200 KB burst; excess
  queued up to 1 s, then dropped with one warning per second (client Output); `PendingHandlers`: no handler
  connected -> queue 256 calls or 1 MiB per remote for 30 s, then drop with an error; `ViolationCounter`: unknown
  NetId, property index or non-remote target -> drop and count; 100 in 10 s -> close ("protocol error").
- Produces (client): `script_events.rs` fires `Humanoid.Died` when a replicated Humanoid's `Health` goes from > 0 to
  <= 0, and `CharacterAdded` when the local or a remote `Player.Character` changes to a Model (R14). A `Wire::Instance`
  the receiver cannot see arrives as nil.

- [ ] **Step 1: Write the failing tests** (`crates/metallic_server/tests/remotes.rs`, two in-process clients on
  `MemoryNet`): `fire_client_arrives_after_the_part_it_names` (server creates a Part and fires it in the same tick:
  the client handler sees a non-nil Part), `reliable_firing_order_across_remotes`, `fire_server_throttled_to_500_per_second`
  (client fires 1,000 in one second: <= 500 handled in that second, rest within the next, one warning per second),
  `fire_server_over_64_kib_fails_on_client`, `invoke_server_times_out_after_30_s`, `invoke_client_errors_on_disconnect`,
  `invisible_instance_arrives_as_nil` (server fires a ServerStorage part), `unreliable_over_1000_bytes_fails_and_stale_dropped`,
  `no_handler_queue_256_then_error`, `unknown_net_ids_close_after_100_in_10_s`, `require_returns_module_value_and_caches`
  (+ cycle error), `humanoid_died_and_character_added_fire_on_client`, `server_script_source_never_reaches_a_client`
  (sentinel). Update `metallic_script/tests/limits.rs` for the 64 KiB up cap.
- [ ] **Step 2: Run** `cargo test -p metallic_server --test remotes -j 4`, `cargo test -p metallic_script -j 4`.
  Expected: FAIL.
- [ ] **Step 3: Implement**; add the API lines and the creator guidance of spec 4.9 to `docs/luau-api.md`.
- [ ] **Step 4: Run** `cargo test -p metallic_script -p metallic_net -p metallic_server -p metallic_client -p metallic_runtime -j 4`.
  Expected: PASS.
- [ ] **Step 5: Commit** `script: remotes over the network, RemoteFunction, UnreliableRemoteEvent, require`.

### Task A11: metallic-server --local and metallic-player --local

**Files:**
- Create: `crates/metallic_net/src/ticket.rs`, `tests/ticket.rs`; `crates/metallic_server/src/{args.rs, run.rs, gate.rs}`;
  `crates/metallic_player/src/net.rs`; `tools/m2/local_match.py`
- Modify: `crates/metallic_server/src/main.rs`, `Cargo.toml`; `crates/metallic_player/src/{args.rs, main.rs}`,
  `Cargo.toml`; `crates/metallic_runtime/examples/build_parkour/scripts.rs` (`FINISH`) and the regenerated
  `templates/parkour/main.mscene`, `tests/smoke.mscene`; `.github/workflows/ci.yml` (one step)

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `ticket.rs` (R3): `pub struct TicketClaims { pub kid: String, pub jti: [u8; 16], pub user: u64, pub name: String, pub place: String, pub server: String, pub kind: TicketKind /* Play|Edit|Test */, pub iat: u64, pub exp: u64, pub aud: String }`;
    `sign(key: &SigningKey, claims: &TicketClaims) -> String` = `base64url(postcard(claims)) + "." + base64url(signature)`;
    `TicketVerifier::new(keys: Vec<(String, VerifyingKey)>)`, `verify(&mut self, ticket: &str, server: &str, now: u64) -> Result<TicketClaims, TicketError>`:
    signature by `kid`, `aud == "metallic-server"`, `exp > now`, `iat <= now`, `exp - iat <= 60`, `jti` unseen
    (replay set kept until `exp`), `server` matches. `TicketError` variants are internal; the client only ever sees
    one generic kick reason. `Debug` redacts.
  - `metallic-server --local --listen <addr> --project <dir> [--scene F] [--join-file F] [--max-players N] [--exit-when-empty] [--sim-latency-ms N] [--sim-jitter-ms N] [--sim-loss-pct P]`;
    public key from env `METALLIC_TICKET_PUBKEY` (base64url, 32 bytes) unless `--join-file`; `--hosted` exits 2
    ("hosted mode is M2b"). After binding it prints one line to stdout:
    `METALLIC_SERVER_READY addr=<ip:port> cert_sha256=<64 hex>`. Real-time loop at 60 Hz (sleep to the next tick;
    late ticks run back to back, at most 15 per catch-up). Server log lines are prefixed by nothing; script output
    goes to stdout. Exit 0 on Ctrl-C or, with `--exit-when-empty`, when the last player has left after at least one join.
  - Join file (R4, TOML, written with user-only permissions, never committed): `address`, `port`, `cert_sha256`,
    `signing_key` (base64url). `metallic-player --join-file F [--name N]` mints a 60 s ticket.
  - `metallic-player --local <host:port> --cert-sha256 <hex> --ticket <t> [--name N] [--headless --frames N] [--input-script F]`;
    no project argument in network modes; prints `metallic-player: joined as <name> in <ms> ms`; on a lost connection
    prints `metallic-player: disconnected: <reason>` and exits 3 (reconnect UI is M2b). `net.rs` builds
    `Simulation(Client)` + `ClientDriver` + `Game::networked`.
  - Parkour `FINISH` (R5): first finish per player counts (`finished[player]`), time measured from that player's first
    spawn, and `print(player.Name .. " finished in " .. string.format("%.1f", seconds) .. " s")` on the server.
  - `tools/m2/local_match.py`: starts the server (`--exit-when-empty --join-file <temp>`), waits for the ready line,
    starts player A headless with `templates/parkour/tests/route.input`, player B the same 5 s later, waits up to
    180 s; passes when the server output has exactly 2 `finished in` lines, both joins took < 5 s, all three exit with
    0, and none of the three PIDs is alive afterwards. CI step (Windows job, after `cargo build -p metallic_server -p metallic_player`).

- [ ] **Step 1: Write the failing tests:** `crates/metallic_net/tests/ticket.rs`: `valid_ticket_passes`,
  `tampered_expired_future_wrong_aud_unknown_kid_reused_jti_over_60_s_fail` (one assertion each), `debug_redacts`;
  `crates/metallic_player/src/args.rs` unit tests for the new flags (`--local` needs `--cert-sha256` and `--ticket`;
  `--join-file` alone is enough; a project dir with `--local` is an error); `crates/metallic_server` args tests.
- [ ] **Step 2: Run** `cargo test -p metallic_net --test ticket -j 4`, `cargo test -p metallic_player -p metallic_server -j 4`.
  Expected: FAIL.
- [ ] **Step 3: Implement**; regenerate the Parkour scenes with `cargo run -p metallic_runtime --example build_parkour -j 4`.
- [ ] **Step 4: Run** the tests, `cargo test -p metallic_runtime -j 4` (Parkour tests still pass), and
  `python -I tools/m2/local_match.py`. Expected: PASS; paste the script output into the PR.
- [ ] **Step 5: Commit** `server/player: --local mode with signed tickets; Parkour finish per player; local match check`.

### Task A12: Studio Team Test and editor link

**Files:**
- Create: `crates/metallic_net/src/editor_link.rs`, `tests/editor_link.rs`;
  `crates/metallic_studio/src/team_test/{mod.rs, job.rs, link_server.rs, log.rs}`, `crates/metallic_studio/tests/team_test.rs`
- Modify: `crates/metallic_server/src/{args.rs, run.rs}`, `crates/metallic_player/src/{args.rs, net.rs}` (`--editor-link`),
  `crates/metallic_studio/{src/lib.rs, src/error.rs, Cargo.toml}`, `.github/workflows/ci.yml` (one step)

**Interfaces:**
- Produces:
  - Editor link (spec 6.2): TCP on `127.0.0.1` only; frames = `u32` little-endian length + postcard; the process sends
    `Hello { token }` first (token from env `METALLIC_EDITOR_LINK_TOKEN`, 32 random bytes hex; wrong token -> Studio
    closes). `ToProcess::{ListChildren { id: Option<u64> }, GetProps { id: u64 }, SetProp { id: u64, name: String, value: Value }, Insert { parent: u64, class: String }, Delete { id: u64 }, SetSource { id: u64, source: String }, Pause, Step { ticks: u32 }, Resume, Screenshot, SetConditioner(LinkConditioner)}`
    (ids are the process's own `InstanceId::to_bits`); `FromProcess::{Log { level: LogLevel, text: String }, Children(Vec<(u64, String, String)>), Props(Vec<(String, Value)>), Inserted(u64), Png(Vec<u8>), Done, Error(String)}`.
    `LinkClient::connect(addr, token) -> Result<LinkClient, NetError>`, `poll(&mut self) -> Option<ToProcess>`,
    `send(&mut self, FromProcess)`; `apply_edit(world: &mut World, cmd: &ToProcess) -> Option<FromProcess>` for the
    tree and property commands. Edits do not persist. Screenshot only in a windowed player (else `Error`);
    `SetConditioner` only on the server.
  - `TeamTestConfig { pub players: u8 /* 1..=8, default 2 */, pub conditioner: LinkConditioner, pub headless: bool, pub server_exe: PathBuf, pub player_exe: PathBuf }`
    (exes default to siblings of `current_exe()`, overridable by `METALLIC_SERVER_EXE`/`METALLIC_PLAYER_EXE`);
    `TeamTestSession::start(world: &World, project_name: &str, cfg: TeamTestConfig) -> Result<TeamTestSession, StudioError>`:
    saves the place to a temp folder, makes a throwaway Ed25519 key pair, starts the server with
    `--local --listen 127.0.0.1:0 --exit-when-empty --editor-link 127.0.0.1:<port>` (public key in the environment),
    reads the ready line, signs one ticket per player (user ids 1..=N, names `Player1`..`PlayerN`), starts N players,
    all in one Windows Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; non-Windows: kill children on drop);
    `poll_logs(&mut self, out: &mut Vec<LogLine>)` with `LogLine { source: LogSource::{Server, Player(u8)}, level, text }`
    (stdout/stderr and link logs, tagged); `link(&mut self, source: LogSource) -> Option<&mut LinkPeer>`;
    `process_ids(&self) -> Vec<u32>`; `stop(self) -> Result<(), StudioError>` (closes the job; every process gone
    within 2 s; temp folder removed).
  - UI (R11): Team Test button with count, Output with one tag per process, Network simulation menu (latency, jitter,
    loss -> `SetConditioner`). Lands with Studio tasks #10/#11.

- [ ] **Step 1: Write the failing tests:** `crates/metallic_net/tests/editor_link.rs`: `wrong_token_closes`,
  `set_prop_and_list_children_round_trip`, `pause_step_resume` (server loop runs exactly N ticks on `Step { ticks: N }`),
  `logs_stream`; `crates/metallic_studio/tests/team_test.rs` (`#[ignore]`, run by the CI step with the exe env vars set):

```rust
#[test] #[ignore] fn team_test_starts_server_and_three_players_and_stops_clean() {
    let s = TeamTestSession::start(&parkour_world(), "Parkour", TeamTestConfig { players: 3, headless: true, ..cfg() })?;
    // within 15 s: log lines tagged Server, Player(1), Player(2), Player(3) (each player's "joined as")
    let pids = s.process_ids(); assert_eq!(pids.len(), 4);
    let t = Instant::now(); s.stop()?; assert!(t.elapsed() <= Duration::from_secs(2));
    assert!(pids.iter().all(|p| !process_alive(*p))); // `tasklist /FI "PID eq p"` on Windows
}
```

- [ ] **Step 2: Run** `cargo test -p metallic_net --test editor_link -j 4`; build the two binaries, then
  `METALLIC_SERVER_EXE=... METALLIC_PLAYER_EXE=... cargo test -p metallic_studio --test team_test -j 4 -- --ignored`.
  Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same commands. Expected: PASS. Add the CI step (Windows job).
- [ ] **Step 5: Commit** `studio: Team Test sessions (server + N players, Job Object, editor link, tagged logs)`.

### Task A13: Templates on split worlds

**Files:**
- Create: `docs/agents/<issue#>-m2-a13-templates.md` (audit), `crates/metallic_server/tests/parkour_split.rs`
- Modify: `crates/metallic_runtime/examples/build_parkour/scripts.rs`, regenerated `templates/parkour/*.mscene`

**Interfaces:**
- Consumes: A11 binaries and Finish fix; A10 client events.
- Produces: an audit of every template on `main` (today only Parkour) listing logic that assumed one shared M1 world.
  Known items to fix in Parkour: `SMOKE_SERVER` waits for `Players:WaitForChild("Player1")` (use the first player
  from `PlayerAdded` / `GetPlayers()`), the smoke client/server pair works when the client is a separate VM, the HUD's
  `Humanoid.Died` and `CharacterAdded` rely on A10's client events, Lava/Checkpoints are per player already. Every
  later template on `main` gets the same audit row.

- [ ] **Step 1: Write the failing test** `crates/metallic_server/tests/parkour_split.rs`:
  `two_players_finish_parkour_on_split_worlds` (server + two in-process clients with Luau, `MemoryNet` 30 ms one way,
  1 % loss; both replay `route.input`, the second 5 s later; the server output has 2 `finished in` lines; each
  client's HUD text starts with `Finished in`); `smoke_scene_passes_networked` (`tests/smoke.mscene`, one client:
  output has `PARKOUR_SMOKE_OK`).
- [ ] **Step 2: Run** `cargo test -p metallic_server --test parkour_split -j 4`. Expected: FAIL (smoke waits for
  `Player1`).
- [ ] **Step 3: Implement** the script fixes, regenerate scenes, write the audit.
- [ ] **Step 4: Run** the test, `cargo test -p metallic_runtime -p metallic_player -j 4` (Combined Parkour and smoke
  unchanged), `python -I tools/m2/local_match.py`. Expected: PASS.
- [ ] **Step 5: Owner check (needs the owner):** two PCs on a LAN: PC 1 runs
  `metallic-server --local --listen 0.0.0.0:<port> --project templates/parkour --join-file <file>`; the join file is
  copied to PC 2 by hand; both run `metallic-player --join-file <file>` windowed and finish Parkour together. Record
  the result (no addresses) in the audit file.
- [ ] **Step 6: Commit** `templates: Parkour on split worlds (audit, smoke and finish fixes)`.

### Task A14: Bot load test

**Files:**
- Create: `crates/metallic_player/src/bots.rs`, `crates/metallic_server/src/stats.rs`,
  `crates/metallic_server/examples/build_arena.rs`, `tools/loadtest/arena/{metallic.toml, main.mscene}`,
  `tools/loadtest/run.py`, `tools/loadtest/test_run.py`, `docs/agents/<issue#>-m2-a14-loadtest.md`
- Modify: `crates/metallic_player/src/args.rs`, `main.rs`; `crates/metallic_server/src/{args.rs, run.rs}`

**Interfaces:**
- Produces:
  - `metallic-player --headless --bot <walk-route|wander|spam-remotes|bad-packets> --bots N --local ... | --join-file F`
    (R13): N `ClientDriver` connections in one process, prediction on, no Luau, no window. `walk-route` loops
    `route.input`; `wander` picks a new direction every 2 s and jumps 20 % of the time; `spam-remotes` fires
    `ReplicatedStorage.LoadTestPing` 1,000 times a second with 1 KB payloads; `bad-packets` sends truncated frames,
    unknown kinds, unknown NetIds, out-of-window input ticks and oversize frames, expects to be kicked, reconnects.
    Every 10 s prints `METALLIC_BOT_STATS {"bots":N,"corrections_per_min":x,"rtt_ms":x,"down_bps":x,"up_bps":x}`.
  - `metallic-server ... --stats-every <s>` prints `METALLIC_SERVER_STATS {"players":n,"tick_p50_ms":x,"tick_p99_ms":x,"tick_max_ms":x,"cpu_pct_one_core":x,"rss_mib":x,"down_bps_per_client":x,"up_bps_per_client":x,"input_starves":n,"overflows":n}`
    (CPU and memory via `GetProcessTimes`/`GetProcessMemoryInfo` on Windows, `/proc/self` elsewhere).
    `DEFAULT_MAX_PLAYERS` in `args.rs` = the measured maximum (largest count with p99 <= 16.7 ms), at most 200.
  - Arena place (`build_arena`): 512 x 512 floor, spawn ring for 200, 50 unanchored crates, 20 movers,
    `ReplicatedStorage.LoadTestPing` RemoteEvent with a server Script that counts calls.
  - `tools/loadtest/run.py --counts 25,50,100,150,200 --places parkour,arena`: per row start a server
    (`--max-players 250 --stats-every 10`), ceil(N/25) bot processes (mix: 70 % walk-route on Parkour / wander on
    arena, 20 % wander, 5 % spam-remotes, 5 % bad-packets), warm 30 s, measure 60 s, stop, then write the markdown
    table (players vs tick p50/p99/max, CPU per core, memory, bytes per client up/down, starves, corrections/min) to the
    `docs/agents` file. `test_run.py` unit-tests the stats parsing and table writer.

- [ ] **Step 1: Write the failing tests:** `tools/loadtest/test_run.py` (`parses_stats_lines`, `writes_table`,
  `pass_rule_is_p99_at_most_16_7_at_50`); player args tests for `--bot`/`--bots`; server unit test
  `tick_percentiles` in `stats.rs`; `crates/metallic_server/tests/bandwidth.rs` gains `bad_packets_bot_is_kicked_and_server_keeps_ticking`.
- [ ] **Step 2: Run** `python -I -m unittest discover -s tools/loadtest -p "test_*.py"` and
  `cargo test -p metallic_player -p metallic_server -j 4`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the tests (PASS), then `python -I tools/loadtest/run.py --counts 25,50` by day and the full
  `--counts 25,50,100,150,200` between 00:00 and 08:00 Chicago. Expected: the table is written; the 50-player rows
  have p99 <= 16.7 ms (if not, record the numbers and stop: B9 decides, do not tune silently). Set
  `DEFAULT_MAX_PLAYERS` from the table.
- [ ] **Step 5: Commit** `loadtest: bots, server stats, arena place, results table`.

---

## Order of work and parallel lanes

```
A1 journal ──> A2 rules/classes/hash ──> A6 wire ──┐
A3 Simulation/roles (+ script host roles) ────────┤
A4 CharacterState/replica physics ────────────────┼──> A7 replication + drivers ──┬──> A8 snapshots ──> A9 prediction ──┐
A5 transport ─────────────────────────────────────┘                               └──> A10 remotes ───────────────────┼──> A11 --local ──┬──> A12 Team Test
                                                                                                                       │                 ├──> A13 templates
                                                                                                                       │                 └──> A14 load test
```

- **Wave 1, start now in parallel:** core lane A1 -> A2 -> A6; runtime lane A3; physics lane A4; net lane A5 (R12).
  Shared files: A5 and A6 both edit `crates/metallic_net/src/lib.rs` and `Cargo.toml` (A5 creates them; A6 adds
  `pub mod wire;` and its dependency lines; rebase). A2 changes one function in `metallic_physics/src/sync.rs`; A4 edits
  other functions there; whoever lands second rebases. A3 touches `metallic_script/src/host.rs`; nothing else in wave 1
  does.
- **Wave 2:** A7 alone (needs A1-A6). It creates every hook file so the next wave's lanes do not share files.
- **Wave 3, two lanes:** lane X = A8 then A9 (A9 consumes A8's client snapshot handling and edits `game.rs`);
  lane Y = A10 (`metallic_script`, `metallic_net/src/remote.rs`, the `remotes.rs`/`script_events.rs` hook files).
  Only A8 edits `replication/server.rs` and `tests/support/mod.rs` in this wave.
- **Wave 4:** A11 alone (binaries, tickets, Parkour Finish, CI step).
- **Wave 5, three lanes:** A12 (Studio, editor link), A13 (templates), A14 (bots, stats). A12 and A14 both add flags
  to `metallic_server/src/args.rs` and `metallic_player/src/args.rs` (independent lines; rebase). A13 and A11 touch
  `build_parkour/scripts.rs`; A13 runs after A11. A12's UI step waits for Studio #10/#11 (R11). A13 Step 5 and the
  night run of A14 need the owner or the night window.
- Per the owner's rule, a wave's finished lanes may go up as one combined PR.

## Out of scope for M2a

Edge, host agent, backend tickets and hosted mode (M2b), lag compensation (B7), parallel snapshot building (B9),
reconnect UI (M2b), Team Create (M2c), voice (M2d), StreamingEnabled, client network ownership, zone sharding, live
script co-editing.
