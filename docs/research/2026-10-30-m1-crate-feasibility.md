# M1 crate and API feasibility: Studio, installer, profiler, debugger, Luau VM

Date: 2026-10-30. Issue: [#42](https://github.com/Metallic-Club/studio/issues/42). Author: Claude (session
metallic-1e-work). Status of this document: research. It adds no dependency and changes no interface.

**Purpose.** Give the remaining M1 tasks known crate choices, API names and risks, and check the tasks already merged
against the same sources.

**Baseline.** `origin/main` at `96aceb0` (2026-10-30), after waves 1 and 2, Task 8 (#184), the avatar (#266),
the per-VM script budget (#270) and the Launcher L1 plan (#188) [R2]. Status words follow CLAUDE.md:

This is a dated research snapshot, rather than a current feature inventory. The publication pass merged
`5c84cf7` and checked two conclusions against that source: Task 10's editor is now implemented through PR #319,
including viewport, Explorer, Properties and command history; the updater uses an exact-tag source, so Velopack's
ten-release discovery window is no longer a Metallic download blocker. The remaining sections retain their
`96aceb0` observations unless explicitly marked otherwise. Check current source and issue status before starting
a recommended task.

| Task | Issue | Status at baseline `96aceb0` | Crates it uses |
|---|---|---|---|
| 5 Humanoid character | #5 closed | implemented | rapier3d 0.36.1 |
| 6 Luau VM, 7 Luau API | #6, #7 closed | implemented, tested | mlua 0.12.2 (`luau`, `vendored`), luau0-src 0.22.0+luau740 |
| 8 Runtime, player, Parkour | #8 closed | implemented | own winit 0.30.13 loop, egui + egui-wgpu 0.37.0 overlay, gilrs 0.11.2 |
| 9 Studio shell | #9 closed | implemented | eframe/egui 0.37.0 (wgpu backend, `default_fonts`), rfd 0.17.2 |
| 13 Installer and update | #13 open | implemented (code, tested with mocks); owner round trip pending | velopack 1.3.39-g8d2844f, ureq 3.5.0, keyring 3.6.3 |
| 10 Editing, 11 Play/Stop, 14 Developer mode | open | planned | none yet |
| 15 Luau debugger | #22 open | planned | none yet |
| 16 Profiler and debug views | #23 open | planned | none yet |

**Scope and neighbours.** This is crate and API level. It does not repeat:
- the engine-to-engine comparison (#30, PR #46);
- the product split (`2026-10-30-platform-architecture.md`) [R5];
- the Studio UI design (`2026-10-30-studio-ui-design.md`, PR #127) [R9], which already decides egui 0.37,
  egui_tiles docking, own gizmo code, fonts and icons;
- `2026-10-30-performance-and-scale.md` [R10], which measured mlua call and interrupt costs.

Where this document disagrees with one of them, it says so.

**How to read it.**
- **Observations (sourced)** say only what a source says. Tags such as [L4] point to the Sources list. Every source
  was accessed 2026-10-30. **Recommendations (judgment)** are my calls.
- "Unverified" marks background knowledge not re-checked against a 2026-10 source.
- Nothing was measured for this document. Any figure is labelled as an estimate, as arithmetic, or as someone
  else's measurement.

## Summary

- **Core value left: Tasks 15 and 16.** Raw `mlua::ffi` exposes debugger primitives, but pause/resume integration
  still needs prototypes. Safe mlua has no Luau line hooks and treats `LUA_BREAK` as an error; neither a raw-resume
  adapter nor a yield-based hook has been validated here. A debugger must explicitly retain paused threads.
  For the profiler, `puffin_egui` is pinned to egui 0.33, so Studio needs its own timeline view.
- **Task 13 is implemented**, with a GitHub App instead of the plan's OAuth app. Three findings remain in this snapshot:
  - ~~The download path still uses Velopack's `GithubSource`, which reads only the 10 newest releases.~~ Fixed
    by #237 (closes #202): the updater now passes an exact-tag source to Velopack.
  - Velopack is pinned to a prerelease.
  - The keyring entry roams and is not named as documented.
  - The plan and spec still describe an OAuth app with `repo` scope.
- **Licence rules and recorded decisions disagree in this snapshot.** `deny.toml` allows OFL-1.1 and Ubuntu-font-1.0
  for every crate (a conductor ruling, "R11"), and adds a CDLA-Permissive-2.0 exception for `webpki-roots`. Its own
  comment says the CDLA exception "Needs a lead/owner decision". CLAUDE.md's licence list is unchanged; this
  research did not locate the R11 decision text (section 8). Verify the current ruling before changing policy.
- **The Launcher plan puts its releases in the same repo.** The exact-tag updater fix above removes the
  ten-release download-window concern. Release location still needs a product decision for channel ownership.
- **Tasks 6-7 cover the sandbox and budget risks** this research found, with tests. Two gaps remain: long single Rust
  calls cannot be interrupted, and the compiler debug level is not yet switchable for the debugger.

---

## 1. Tasks 9-11: Studio stack

### Observations (sourced)

**What is merged.**
- `metallic_studio` runs on `eframe` 0.37.0 with `default-features = false` and features `wgpu` and `default_fonts`.
  `main.rs` calls `velopack::VelopackApp::build().run()` first, then `eframe::run_native` with
  `Renderer::Wgpu` [R11].
- There is no docking, viewport, Explorer or Properties yet; those belong to Task 10.
- `metallic_render` offers `RenderContext::from_eframe(device, queue)`. When it creates its own device it requests
  default features only [R3].
- `metallic-player` (Task 8) already runs its own winit `ApplicationHandler` and draws its game GUI with an
  `egui_wgpu::Renderer` over the 3D frame, without eframe [R22]. Runtime and player also enable egui's
  `default_fonts`, citing "Ruling R11" [R21].

**egui facts.**
- egui-wgpu 0.37.0 requires `wgpu ^30.0` and `winit ^0.30.13`; 0.36.2 also requires `wgpu ^30.0` [E2][E20]. Locked
  versions are wgpu 30.0.1 and winit 0.30.13 [R2].
- A paint callback's `paint` draws into egui's own render pass. The official demo uses egui's target format, no
  depth-stencil and default multisampling [E4][E5].
- egui ships a semver-breaking minor every 6-10 weeks [E1][E3].
- `epaint_default_fonts` 0.37.0 is licensed `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0` [E17].
  - egui 0.37.0's default feature list is empty [E19].
  - eframe 0.37.0's default features include `default_fonts` [E18].
  - The UI research says 0.37 "drops `default_fonts` from default features". That is true for egui, not for eframe.

**Crates the UI research does not cover.**
- `egui_ltreeview` 0.10.0 (MIT, egui ^0.37) offers multi-select, arrow keys and a drag-and-drop frontend. Its README
  says nothing about virtualization [E12][E13][E20]. Issue #124 asks for an Explorer that handles 50k Instances
  [R9].
- `egui_code_editor` 0.4.2 (MIT) and `egui-probe` 0.13.0 require egui ^0.36 [E14][E15][E20].
- Highlighting:
  - egui_extras' `syntect` feature pulls `syntect ^5.3` [E16].
  - syntect 5.3.0, `fancy-regex` 0.20.0 and `onig` 6.5.3 are MIT; onig builds the Oniguruma C library.
  - tree-sitter 0.27.1 is MIT. `tree-sitter-luau` 1.2.0 is MIT and was last updated 2024-12-22 [E21].
- `undo` 0.52.0 is MIT OR Apache-2.0 [E21].
- egui 0.36.0 changed drag detection, and 0.36.1 fixed `Sense::drag` on overlapping widgets [E3].

**glam.** The build has glam 0.30.10 (`metallic_render`) and 0.33.12 (rapier3d 0.36.1 through glamx 0.3.1). The
lockfile also lists 0.31.1 and 0.32.1 [R2]. The UI research's choice of own gizmo code avoids transform-gizmo's
`glam ^0.32` [E20][R9].

### Recommendations (judgment)

1. **Viewport.** Render into offscreen textures owned by `metallic_render`: HDR colour, depth, MSAA and an
   `R32Uint` object-ID target. Resolve to sRGB and show the result with `register_native_texture`, which the UI
   research also names [R9]. Recreate at physical pixel size on resize, debounced. Use paint callbacks only for
   overlays: a callback inherits egui's format, sample count and depth [E4][E5].
2. **Device features before Task 10.** Tasks 16 and 10 need `TIMESTAMP_QUERY` and, optionally,
   `POLYGON_MODE_LINE` [P10]. Under eframe the device is eframe's, and `from_eframe` accepts it as given [R3].
   - Either configure eframe's wgpu setup to request these features when the adapter has them (eframe API
     unverified),
   - or move Studio to its own winit loop driving egui-winit + egui-wgpu, as `metallic-player` already does for its
     overlay [R22]. That shares window and loop code with the player.

   Decide in Task 10, because it changes how `RenderContext` is built.
3. **Lockstep.** Move `wgpu`, `winit` and `glam` into `[workspace.dependencies]`, since today `metallic_render` and
   eframe each name their own. Fail CI when `cargo tree -d` shows two wgpu majors. Bump egui and wgpu together.
   Convert glam types at the physics boundary until Rapier and the renderer share a major.
4. **Explorer.** Use flattened visible rows drawn with `ScrollArea::show_rows` (uniform row height) and keep
   expansion state in editor data. That is the only route with a known O(visible) cost for #124's 50k Instances.
   egui_ltreeview is fine for a prototype, but it advertises no virtualization. Use row thirds for
   before/into/after, refuse drops onto descendants, and record one `Reparent` per drop.
5. **Properties.** Drive the panel from `metallic_core`'s class registry, not from derive macros (egui-probe would
   also force egui 0.36). Make one undo transaction per drag or focus session.
6. **Script editor.** egui_code_editor needs egui 0.36, so write our own `TextEdit` with a cached `LayoutJob`
   layouter and a painted gutter. Task 15 needs that gutter for breakpoints. tree-sitter-luau is a candidate only (no
   release since 2024-12; compiles C). With syntect, prefer `fancy-regex`.
7. **Undo.** Use the plan's `History`/`Command`:
   - A serialized subtree alone cannot guarantee restoring the same generational `InstanceId`s after destruction.
     The later Task 10 implementation at `5c84cf7` preserves identity by parking detached instances until their
     history entries are discarded (`crates/metallic_studio/src/commands.rs`, `History::is_parked`). Reattach those
     same live instances on undo; do not promise identity reuse through scene deserialization.
   - History is read-only during Play.
   - Hand-written history is preferred over the `undo` crate.
8. **Picking.** Use the GPU ID buffer with 1×1 (click) and rectangle (marquee) readback. If `PhysicsWorld::raycast`
   is used, it must include sensors, because `CanCollide=false` parts are Rapier sensors in our physics (Rapier's
   default filter is unverified).

---

## 2. Task 13: installer, updates and sign-in

### Observations: what is merged

From the merged code, read 2026-10-30 [R12][R13][R14]:
- **Sign-in.** `github_auth.rs` signs in with a **GitHub App** through the device flow, written directly on
  `ureq` 3:
  - no `scope` is sent;
  - it honours `slow_down` and retries 5xx;
  - refresh is serialised under a lock;
  - the session is stored as JSON (`access_token`, `refresh_token`, `expires_at`, `refresh_expires_at`).

  The client ID comes from `config/github_app_client_id.txt`, which is still a placeholder.
- **Update check.** `updater.rs::check` lists releases itself: `per_page=50`, up to 5 pages. It picks the newest
  `vX.Y.Z-<channel>` tag and treats a body line `required: true` as required.
- **Download and apply.** `download` builds `velopack::sources::GithubSource::new(repo, Some(token), true)` with
  `ExplicitChannel`, calls `check_for_updates`, and fails if the version Velopack offers differs from the version the
  check found. `apply_and_restart` calls `apply_updates_and_restart`.
- **Dependencies.** `velopack = "1.3.39-g8d2844f"`, `keyring = { version = "3", features = ["windows-native"] }`
  (locked 3.6.3), `ureq = "3"` (locked 3.5.0). `tools/release.py` pins `VPK_VERSION = "1.3.39-g8d2844f"`
  [R2][R13].
- **Not yet wired.** The progress note says the check on start and every 6 h, and the sign-in UI, are still left for
  the lead [R14].
- **Licences.** `deny.toml` has `exceptions = [{ crate = "webpki-roots", allow = ["CDLA-Permissive-2.0"] }]`, with
  the comment "Needs a lead/owner decision (task 13)" [R1].
- **Planned next (Launcher L1 plan, Task 1)** [R20]:
  - The updater moves into a shared crate `metallic_update`, which settles where it lives.
  - The keyring service becomes a constructor parameter.
  - A `LAUNCHER_FEED` publishes Launcher releases to the **same repo** with tags `launcher-vX.Y.Z-<channel>` and
    Velopack channel `launcher-<channel>`, still on velopack 1.3.39-g8d2844f.

### Observations: sources

- **Velopack.**
  - 1.2.161 (2026-09-29) is the latest stable release. 1.3.39-g8d2844f (2026-10-17) is a prerelease [I1][I2].
  - The update calls block [I5]. There is no required field in `VelopackAsset`/`UpdateInfo` [I7].
  - `wait_exit_then_apply_updates` is marked as the recommended apply call [I5].
  - Setup.exe installs per user; the docs name Add/Remove Programs entries only for MSI builds [I10].
  - Unsigned apps are often flagged by SmartScreen [I13].
- **`GithubSource` in 1.3.39-g8d2844f** (crate source from the crates.io registry, read locally) calls
  `releases?per_page=10&page=1` and filters prereleases afterwards [I35]. 1.2.161 and `develop` do the same [I17].
- **ureq and webpki-roots.** velopack needs `ureq ^3.4` with default features; rustls brings `webpki-roots` 1.0.11,
  licensed CDLA-Permissive-2.0 [I1][I3].
- **`keyring` 3.6.3 on Windows** (crate source, read locally) [I36]:
  - The target name is `format!("{user}.{service}")`, so `Entry::new("MetallicStudio", "github")` is stored as
    `github.MetallicStudio`.
  - Credentials are written with `CRED_PERSIST_ENTERPRISE`.

  Enterprise persistence roams with roaming profiles; a blob is capped at 2,560 bytes [I34]. keyring 4.x moved to
  `keyring-core` plus per-platform store crates [I31][I32][I33].
- **GitHub.**
  - The `repo` scope "Grants full access to public and private repositories including read and write access to
    code" [I24].
  - New organisations restrict OAuth Apps until an owner approves them [I25].
  - GitHub App user tokens use fine-grained permissions, with an optional `repository_id`. `ghu_` tokens last 8 h
    and `ghr_` refresh tokens 6 months; device-flow tokens refresh without the client secret [I26][I27].
  - Release endpoints need Contents: read. The rate limit is 5,000 requests/h per user [I18][I20].

### Findings against merged code

1. **(Fixed by #237, which closed #202, after this was written; kept for the record.)**
   **The 10-release window is still there, in the download path.** `check` pages through 250 releases, but
   `download` asks Velopack's `GithubSource`, which reads only the 10 newest [I35]. Once `stable` exists beside
   per-merge `dev` releases, a stable client will see "update available" from `check`. After 10 newer dev releases,
   its `download` then fails with "the release feed no longer offers an update".
   - **The Launcher plan makes this sooner.** Its releases go to the same repo [R20], so Studio and Launcher dev
     releases share the same 10 slots.
   - A Studio `dev` client stops finding its own release once 10 Launcher releases are newer, and the Launcher
     likewise. That can happen in M1, not only after `stable`.
   - Fix in Launcher Task 1: a separate releases repo per product, an `HttpSource` feed, or a custom `UpdateSource`
     that pages like `check`.
2. **The Velopack pin is a prerelease, with a caret requirement.** `"1.3.39-g8d2844f"` lets `cargo update` move to
   a newer 1.3.x, while `vpk` stays pinned exactly in `release.py` [R13]. The crate and the CLI can drift apart.
   Recommendation: `=1.2.161` (stable) for both, or `=1.3.39-g8d2844f` exactly if a 1.3 feature is needed. Record
   which.
3. **The keyring entry does not match the docs and roams.** The doc comments in `github_auth.rs` and
   `docs/releasing.md` say `MetallicStudio/github` [R12][R13]. keyring 3.6.3 writes target `github.MetallicStudio`
   with Enterprise persistence [I36], so the session roams to other machines that share the user's roaming profile.
   Recommendation: correct the docs, and use keyring 4 (`keyring-core` + `windows-native-keyring-store`, `Local`
   persistence) or a target/persist override. Launcher Task 1 makes the keyring service a constructor parameter,
   which is a good place to fix it [R20].
4. **The plan and spec still describe an OAuth app with `repo` scope.** Plan Task 13 says "scope `repo` read only"
   and `config/github_oauth_client_id.txt`; spec 1C says "register a GitHub OAuth app" [R4]. The code does the right
   thing (a GitHub App), but the source-of-truth documents contradict it. `repo` can never be read-only [I24].
5. **The CDLA exception is marked as undecided** in `deny.toml` itself [R1]. See owner decision 1.
6. **Apply call.** `apply_updates_and_restart` is used where Velopack recommends `wait_exit_then_apply_updates`
   [I5]. This is minor, but the recommended call lets Studio save and exit cleanly first.
7. **Wiring still open.** The update check, background thread and sign-in UI are not wired into `main`/`app` yet
   [R14]. `check` and `download` block [I5], so they must run off the egui thread.
8. **Not verified yet:** the uninstall entry and SmartScreen behaviour. `docs/releasing.md` promises an uninstaller
   [R13], but Velopack's docs only confirm one for MSI [I10]. Check `HKCU\...\Uninstall` in the owner round trip.

---

## 3. Tasks 14 and 16: developer mode, profiler, debug views (planned)

### Observations (sourced)

**CPU profiling.**
- puffin 0.20.0 was released 2026-03-18, about 20 months after 0.19.1. The repo is not archived. Its README gives
  50-200 ns per scope and about 1 ns when off (the authors' measurement on an M1 MacBook Pro) [P1][P2][P3].
- **`puffin_egui` 0.30.0 requires `egui ^0.33.3`** [P4]. Studio is on egui 0.37 [R2].
- Others:
  - `profiling` 1.0.18 [P8], already pulled in by egui-wgpu [E2];
  - tracy-client 0.19.0 and tracing-tracy 0.12.0 [P6][P7];
  - puffin_http 0.17.0 [E21];
  - tracing-chrome 0.7.2, last released 2024-03-15 [P19].

**GPU timing (wgpu 30).**
- `TIMESTAMP_QUERY` works on Vulkan, DX12, Metal, GL (with `GL_ARB_timer_query`) and WebGPU.
- `_INSIDE_ENCODERS` and `_INSIDE_PASSES` are native-only; inside passes is "generally not available on tile-based"
  GPUs.
- `PIPELINE_STATISTICS_QUERY` covers Vulkan and DX12; `POLYGON_MODE_LINE` covers DX12, Vulkan and Metal.

[P10]
- wgpu-profiler 0.28.0 (2026-07-31) targets wgpu 30 and puffin 0.20. Its releases have followed each wgpu major by
  weeks to about four months. It times whole passes with only `TIMESTAMP_QUERY`, never stalls the device, and
  exports Chrome traces [P11][P12].

**Memory.**
- wgpu `generate_allocator_report` returns an `Option` that depends on the backend. `get_internal_counters` needs
  the `counters` feature. `on_uncaptured_error` takes a handler [P13].
- mlua has `used_memory`, plus the Luau-only `set_memory_category` (at most 255, "main" reserved) and
  `heap_dump` [P14].
- The merged VM uses `set_memory_limit` but no categories [R15].

**Physics draw.** rapier3d 0.36.1's `DebugRenderPipeline` and `DebugRenderMode` cover colliders, axes, joints,
contacts, AABBs and soft bodies. There is **no velocity flag** [P15][P16].

**Overlay and logs.** egui_plot 0.38.0 (egui ^0.37) [P17][E20], egui_logger 0.13.0 (MIT, egui ^0.37) [E20][E21],
zip 9.1.3 (MIT) [P18], tracing 0.1.44 / tracing-subscriber 0.3.23 (MIT) [E21]. `png` 0.18.1 is already used by
Studio [R2].

**Reference.** Roblox's MicroProfiler has frame bars coloured by bottleneck, nested labels per thread,
`debug.profilebegin/end`, HTML dumps, flame and diff graphs, and a 60-frame limit on server captures [P20].

### Recommendations (judgment)

1. **Scopes.** Use the `profiling` facade with a puffin backend and an optional Tracy feature, as the performance
   research proposes [R10]. Wrap it in the plan's `Profiler::scope` so engine code has one call.
2. **In-app view: our own egui timeline, not `puffin_egui`.** This corrects the performance research, which names
   `puffin_egui` for the in-app view [R10]. puffin_egui 0.30 needs egui 0.33, so its `Context` type cannot take
   Studio's (egui 0.37). Read puffin's frame data and draw it with the shared Timeline widget the UI research
   plans [R9].
3. **Granularity.** One scope per system, per batch and per script resume; none per Part. Arithmetic from puffin's
   own figure: 2,000 scopes × 50-200 ns ≈ 0.1-0.4 ms per frame.
4. **GPU lanes.** Use wgpu-profiler 0.28, pinned with wgpu 30. Request `TIMESTAMP_QUERY` only when the adapter
   reports it (Iris Xe unverified), and fall back to CPU timing. This depends on recommendation 2 in section 1.
5. **Memory panel.**
   - CPU: a tagged `GlobalAlloc` wrapper with per-subsystem atomics.
   - GPU: the allocator report, plus our own byte count on every buffer and texture.
   - Luau: `used_memory` per VM per frame, and `set_memory_category` per script asset with `heap_dump` on demand.
6. **Physics view.** Feed Rapier's pipeline into one `LineList` buffer plus a depth-off x-ray pass (the backend trait
   signature is unverified for 0.36). Draw velocity arrows and sleeping tint ourselves.
7. **Render views.** One `DebugViewMode` uniform:
   - barycentric-shader wireframe (portable; `PolygonMode::Line` only when supported);
   - normals as colour;
   - additive overdraw into `R16Float` with a heat ramp;
   - a tint per cascade.
8. **Developer mode.**
   - Overlay: egui_plot lines at 16.6 and 33.3 ms.
   - Logs: our own `tracing_subscriber::Layer` writing into a bounded ring drawn with `show_rows`. Script output
     uses target `script::<name>`, and wgpu errors come in via `on_uncaptured_error`.
   - Bug report: a `zip` with log, screenshot, `system.json` and a Chrome-trace JSON written in-house on
     `serde_json` (tracing-chrome is stale). The user reviews it before sending, and the test
     `bug_report_has_no_tokens` stays.
9. **Better than Roblox (concepts, no copied code):**
   - automatic per-script scopes;
   - real per-pass GPU lanes;
   - clicking a scope selects its Instance or source line;
   - an always-on ring buffer with auto-pause on spikes;
   - memory lanes on the same timeline;
   - save and diff captures.

---

## 4. Task 15: Luau debugger (planned)

Publication audit: the hook/pause conclusions below were checked against mlua v0.12.2 at
`a186d461c7772aed4cb0a9fa1d1899121c6450e7`, Luau 0.740 at `c0e346edd89066b44dca174c9f54ce84c746a540`,
and Metallic's scheduler at `5c84cf7` [R23]. This is source analysis; no debugger prototype ran.

### Observations (sourced)

**Versions.**
- mlua 0.12.2 (MIT) depends on mlua-sys 0.13.0, which depends on luau0-src 0.22.0+luau740. Those are exactly the
  locked versions [L1][L2][L3][R2].
- luau0-src 0.22.1+luau744 was yanked on 2026-10-24 [L3].
- mlua's LICENSE is MIT with a preamble [L23].

**Safe mlua under `luau`.**
- No line or call hooks: `set_hook` is `cfg(not(feature = "luau"))` [L4][L5].
- `inspect_stack` sees only the current thread and returns no locals [L4][L5].
- `Compiler::set_debug_level(2)` is "necessary for debugger". The default is 1, and optimisation level 2 "harm[s]
  debuggability" [L9][L17].
- Thread `on_resume`/`on_yield` callbacks fire only for mlua-driven resumes [L6].
- `Thread::state()`, `exec_raw` and `get_or_init_from_ptr` give raw access [L4][L6].

**Raw API.**
- `mlua::ffi` binds `lua_breakpoint`, `lua_singlestep`, `lua_getinfo`, `lua_getlocal`, `lua_setlocal`,
  `lua_getupvalue`, `lua_stackdepth`, `lua_break`, `lua_callhook`, `lua_resume` and `lua_Callbacks` (`debugbreak`,
  `debugstep`, `debugprotectederror`, ...) [L7].
- mlua itself writes `interrupt`, `userthread` and `userdata` in that struct [L4][L10].

**Luau 0.740 behaviour.**
- **Breakpoints.** `lua_breakpoint` moves to the next line with code and patches that line's first instruction to
  `LOP_BREAK` in the Proto and every nested Proto. Native execution does not implement interpreter breakpoints;
  installing one can invoke the registered callback that disables native execution for the affected code [L13].
- **Hitting one.** `LOP_BREAK` calls `debugbreak`, which may put the thread into error or yield state, with
  `savedpc` at the current instruction [L14]. Inference, untested: on resume the same BREAK fires again.
- **Breaking.** `lua_break` errors across a metamethod or C-call boundary; `lua_resume` returns `LUA_BREAK` (6)
  [L15].
- **Stepping.** `lua_singlestep` is per thread and inherited by new threads. `debugstep` runs before each
  instruction except `PREPVARARGS` and `BREAK` [L14][L16].
- **Errors.** `debugprotectederror` runs only when the thread is yieldable and "is only allowed to break" [L15].
- **Callbacks.** They are shared by all coroutines, and only `interrupt` may change while code runs [L12].
- **mlua and `LUA_BREAK`.** mlua handles only `OK` and `YIELD`. A thread that returns `LUA_BREAK` takes the error
  path and later reports `CoroutineUnresumable` [L6][L8]. A hook returning `LUA_YIELD` takes mlua's ordinary yield
  status path; this establishes neither payload preservation nor safe debugger pause/resume integration [L4][L6][L14].
- **Hook stack.** `luau_callhook` restores the pre-hook stack top after invoking the callback [L14]. Source-derived
  inference: a hook-pushed pause tag is not a supported way to return ordinary yielded values. Out-of-band pause
  state plus a zero-result yield is a candidate to prototype, not an established safe integration.

**What the merged VM gives the debugger** [R15]:
- `ScriptVm::lua()` exposes the `Lua` "for ... the debugger (task 15), which reaches the raw `lua_State` through
  `mlua::ffi`".
- Every event handler runs in its own guarded thread [R16].
- `task.wait` yields a `SLEEP` tag. Other yields leave resumption to their owner [R17]. The current scheduler's
  unknown-yield path and weak-key thread-state registry do not retain a strong `Thread` reference; a debugger
  must hold that reference explicitly while paused [R23].
- Each resume gets a fresh deadline, checked in the `set_interrupt` callback. The default is 0.1 s, and since #270
  `ScriptVm::set_resume_budget` changes it per VM [R15][R17].
- No `Compiler` is set anywhere, so scripts compile at debug level 1 [R15].
- The `luau-jit` feature is off (features are `luau` and `vendored`) [R2].

**Reference only.**
- Roblox Studio has conditional, log and temporary breakpoints, stepping, Call Stack, Watch and "Trigger At" [L20].
- `sssooonnnggg/luau-debugger` has no licence file (nothing may be copied); it blocks the VM's OS thread while
  paused [L21].
- luau-lsp has no debug adapter [L22].

| Plan feature | Mechanism | Safe mlua enough? |
|---|---|---|
| Line breakpoint | `lua_breakpoint` on the script's main function + `debugbreak` | No |
| Condition / logpoint | Evaluate in `debugbreak`, then continue or pause | Hook raw; evaluation in mlua |
| Pause a running game | Between frames stop stepping; in a loop the interrupt returns `VmState::Yield` | Yes |
| Step over/into/out | `lua_singlestep` + `debugstep` comparing line and `lua_stackdepth` | No |
| Call stack, locals, upvalues | `lua_getinfo`/`lua_getlocal`/`lua_getupvalue` on the paused thread (debug level 2) | No |
| Globals, tables, Instances | `Function::environment`, table iteration, `__todebugstring` | Yes |
| Watch / hover / evaluate | mlua chunk with an env built from a locals snapshot | Mostly |
| Set local | `lua_setlocal` | No |
| Break on error | `debugprotectederror` (handled); post-mortem frames of the dead thread (unhandled) | No |

These are API candidates. In particular, protected-error pausing needs an unresolved adapter: Luau permits
`lua_break` there, while ordinary mlua resume rejects `LUA_BREAK`. A yield-based breakpoint prototype would not
establish break-on-error support [L6][L15].

### Recommendations (judgment)

1. **Prototype nonblocking pause.** `Lua` is `!Send` here, `ScriptHost` holds `Rc<RefCell<World>>`, and egui runs
   on the main thread, so a blocked hook would freeze Studio.
   - Candidate: `debugbreak`/`debugstep` write out-of-band pause state, then yield zero results. Do not rely on
     hook-pushed return tags surviving the hook wrapper.
   - The debugger must retain a strong paused `Thread` handle before the resume caller releases its handle.
     Prototype Continue through `Budget::resume`, including garbage collection while paused and a fresh deadline.
   - While anything is paused, the runtime skips scripts, physics and character but still renders. That is the
     plan's "game freezes, Studio works".
2. **Prototype `lua_break` + raw `ffi::lua_resume` as the alternative.** Prove compatibility with mlua's thread
   status/stack bookkeeping and the budget scheduler before comparing cost. Test protected-error pausing separately.
3. **Contain the unsafe code.**
   - Use one module, `metallic_script::debug::ffi`, with `// SAFETY:` comments and `catch_unwind` in every callback.
   - Never overwrite `lua_Callbacks::userdata`; reach state through `get_or_init_from_ptr` + `app_data`.
   - Change callbacks only while no Luau code runs.
4. **Compiler settings.** Add a VM option that calls `Lua::set_compiler` with debug level 2 and optimisation level
   1 for Studio Play, and keep level 1 in `metallic-player`. Never use `luau-jit` while debugging.
5. **Server and client** VMs compile separately, so set each breakpoint in every VM that loaded the script, with a
   "Trigger At" filter.
6. **Non-yieldable sites.** Define an explicit unsupported-pause result and test metamethods, `table.sort`
   comparators and Luau called from Rust. Document which contexts the chosen prototype supports.
7. **Inherited stepping.** Clear the single-step flag on new threads in mlua's thread-create callback.
8. **Budget.** Pausing ends the resume, so Continue starts a fresh budget. Allow a larger budget while stepping;
   `ScriptVm::set_resume_budget` already exists [R15]. Evaluation gets its own short budget and suppresses
   breakpoints.
9. **Interrupt ownership.** The performance research proposes a watchdog thread that installs the interrupt through
   `mlua::ffi` [R10]. Luau allows setting `interrupt` from another thread [L12], but mlua's `set_interrupt` owns that
   slot [L4]. Whichever lands first, the debugger must not depend on the interrupt being present.
10. **Prototype order** (throwaway `#[test]`s on the merged VM):
    1. Breakpoint, zero-result yield, strong-thread retention through GC, locals inspection and the re-hit check.
    2. The same with `lua_break`.
    3. Step over, into and out.
    4. A breakpoint in a metamethod and in a Rust-called handler.
    5. `endless_loop_still_times_out_when_not_paused`.
    6. Breakpoints and locals under `sandbox(true)`, and a separate protected-error pause/resume case.

---

## 5. Tasks 6-7: VM facts against the merged code

| # | Fact (sourced) | Source | Merged code (`metallic_script`) |
|---|---|---|---|
| 1 | `set_interrupt` is one callback per VM with no "which resume" argument | [L4] | Covered: `Scheduler.deadline` holds the current resume's deadline [R15][R17] |
| 2 | The interrupt error can be caught by `pcall` | [L4] | Covered: the error repeats until the resume returns; tests `pcall_wrapped_loop_still_times_out`, `timeout_reported_once` [R18] |
| 3 | `sandbox(true)` removes nothing; `debug`, `getfenv`/`setfenv`, `loadstring` stay | [L19][L4] | Covered: they are removed before `lua.sandbox(true)`; test `forbidden_globals_are_nil` [R15][R18] |
| 4 | Yielding fails across a C-call boundary, so handlers need their own threads | [L4] | Covered: each handler runs in its own guarded thread [R16] |
| 5 | One long C or Rust call cannot be interrupted | [L19] | Partly: string patterns and `string.rep` are bounded [R15][R18]. `GetDescendants` builds the whole list in Rust with no limit; only the 256 MB cap applies [R19] |
| 6 | `set_memory_limit` covers the whole VM | [L4][L24] | Covered: 256 MB per VM; test `memory_cap_stops_script` [R15][R18]. Per-script categories: not used |
| 7 | `luau-jit` hides locals and ignores breakpoints | [L13] | Covered: not enabled [R2] |
| 8 | Debug level is set through `Lua::set_compiler` | [L9] | Open: never called, so level 1 (Task 15 needs level 2) |
| 9 | mlua 0.12.x tracks Luau FFI changes; `lua_Callbacks` is `#[non_exhaustive]` | [L7][L11] | Open: `mlua = "0.12.2"` is a caret requirement, so `cargo update` can move it. Pin `=0.12.2` once the debugger uses `ffi` |

The interrupt's per-call `Instant::now()` cost was measured in the performance research (2.4x-3.1x slower hot
loops) [R10]. It is not re-examined here.

---

## 6. Task 5: humanoid on Rapier

This track was dropped. Task 5 was implemented by the lead (#5, closed; `metallic_physics/src/character.rs`) [R2].
Researching `KinematicCharacterController` now would duplicate finished work.

---

## 7. Licence and version table

"Allowed" checks each crate's declared licence against the current `deny.toml` [R1]. That file now also allows
OFL-1.1 and Ubuntu-font-1.0, and has a CDLA-Permissive-2.0 exception for `webpki-roots`. "Locked" means the crate is
in `Cargo.lock` at that version [R2]. Transitive dependencies of candidates were not checked. **Bold** rows need an
owner decision.

| Crate | Version | Licence | Allowed | Locked / candidate |
|---|---|---|---|---|
| wgpu | 30.0.1 | MIT OR Apache-2.0 | yes | locked |
| winit | 0.30.13 | Apache-2.0 (unverified) | yes | locked |
| glam | 0.30.10, 0.33.12 | MIT OR Apache-2.0 (unverified) | yes | locked (two majors built) |
| rapier3d | 0.36.1 | Apache-2.0 | yes | locked |
| eframe, egui, egui-wgpu, egui-winit | 0.37.0 | MIT OR Apache-2.0 [E1][E21] | yes | locked |
| **epaint_default_fonts** | 0.37.0 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 [E17] | yes since OFL/Ubuntu were added | locked |
| mlua, mlua-sys | 0.12.2, 0.13.0 | MIT | yes | locked |
| luau0-src | 0.22.0+luau740 | MIT | yes | locked |
| **velopack** | 1.3.39-g8d2844f (prerelease); 1.2.161 stable | MIT [I1][I36] | yes | locked |
| ureq | 3.5.0 | MIT OR Apache-2.0 | yes | locked |
| **webpki-roots** | 1.0.11 | CDLA-Permissive-2.0 [I1] | exception only | locked |
| ring | 0.17.14 | Apache-2.0 AND ISC | yes | via rustls |
| windows | 0.62.2 | MIT OR Apache-2.0 | yes | locked |
| keyring | 3.6.3 (4.2.0 latest) | MIT OR Apache-2.0 [I36] | yes | locked |
| keyring-core, windows-native-keyring-store | 1.0.1, 1.1.0 | MIT OR Apache-2.0 | yes | candidate (keyring 4) |
| semver, serde_json | 1.0.28, 1.0.154 | MIT OR Apache-2.0 | yes | locked |
| egui_tiles | 0.18.0 | MIT OR Apache-2.0 | yes | candidate (UI research choice) |
| egui_ltreeview | 0.10.0 | MIT | yes | candidate (prototype only) |
| egui_code_editor, egui-probe | 0.4.2, 0.13.0 | MIT; MIT OR Apache-2.0 | yes | not recommended (egui ^0.36) |
| syntect, fancy-regex, onig | 5.3.0, 0.20.0, 6.5.3 | MIT | yes | candidate (onig builds C) |
| tree-sitter, tree-sitter-highlight, tree-sitter-luau | 0.27.1, 0.27.1, 1.2.0 | MIT | yes | candidate (grammar stale) |
| undo | 0.52.0 | MIT OR Apache-2.0 | yes | not recommended |
| profiling | 1.0.18 | MIT OR Apache-2.0 | yes | locked (via egui-wgpu); candidate for scopes |
| puffin, puffin_http | 0.20.0, 0.17.0 | MIT OR Apache-2.0 | yes | candidate |
| puffin_egui | 0.30.0 | MIT OR Apache-2.0 | yes | not usable (egui ^0.33.3) |
| wgpu-profiler | 0.28.0 | MIT OR Apache-2.0 | yes | candidate |
| tracy-client, tracing-tracy | 0.19.0, 0.12.0 | MIT OR Apache-2.0 | yes | candidate (optional) |
| tracing, tracing-subscriber | 0.1.44, 0.3.23 | MIT | yes | candidate |
| tracing-chrome | 0.7.2 | MIT | yes | not recommended (stale) |
| egui_plot, egui_logger | 0.38.0, 0.13.0 | MIT OR Apache-2.0; MIT | yes | candidate (egui ^0.37) |
| zip | 9.1.3 | MIT | yes | candidate |
| stats_alloc, cap | 0.1.10, 0.1.2 | MIT; MIT OR Apache-2.0 | yes | not recommended (old) |
| oauth2, octocrab, self_update, axoupdater | 5.0.0, 0.54.7, 1.3.3, 0.10.3 | MIT/Apache-2.0 family | yes | not needed (Task 13 chose otherwise) |

---

## 8. Other contradictions and risks

These are in addition to the Task 13 findings in section 2.

1. **Licence list changed in code, not in the rules.** CLAUDE.md and the spec list only MIT, Apache-2.0, BSD, Zlib,
   ISC, Unicode and MPL-2.0 [R8][R4]. `deny.toml` now allows OFL-1.1 and Ubuntu-font-1.0 for *every* crate, not as
   a per-crate exception [R1].
   - Studio, runtime and player enable `default_fonts`, citing "Ruling R11", with notices in
     `LICENSES/THIRD-PARTY-FONTS.md` [R11][R21].
   - Under CONDUCTOR.md, a ruling is a decision made on the owner's behalf. I did not find R11's text in the baton
     issue (#44), whose list shows R1-R9 [R21].
   - The UI research still says "Do not enable `default_fonts`" and asks the owner about OFL "for font files only"
     [R9].
   - So rules, research and code disagree until the owner confirms R11 and the licence list is updated.
2. **puffin_egui in the performance research** cannot be used with egui 0.37 [P4][R10] (section 3).
3. **`RenderContext` requests no optional features** [R3], so Task 16 GPU timing and the wireframe toggle need a
   device-creation change first.
4. **Two glam majors** in the build [R2]. Every physics↔render boundary converts types until Rapier and
   `metallic_render` share a major.
5. **mlua `LUA_BREAK` and missing line hooks** [L4][L6][L8]. Beyond the plan's list, Task 15 also needs the
   `debugbreak`/`debugstep` callbacks and `lua_stackdepth`. Breakpoints in non-yieldable code can only warn.
6. **Long Rust API calls ignore the budget** (section 5, row 5). As the API grows (Task 10+ and M2), each call that
   walks the tree needs its own limit.
7. **Unverified API names**, to check in spikes:
   - egui-wgpu native-texture update;
   - eframe device configuration;
   - Rapier `DebugRenderBackend` and query-filter defaults;
   - wgpu `resolve_query_set`/`get_timestamp_period`.

---

## 9. Owner decisions needed

1. **Confirm or reverse the CDLA-Permissive-2.0 exception for `webpki-roots`** (already in `deny.toml`, marked as
   needing a decision). Without it, Velopack must be replaced; every listed alternative is weaker (no installer or
   no deltas).
2. **Fonts and the OFL rule: confirm Ruling R11.** Pick one:
   - (a) keep OFL-1.1 and Ubuntu-font-1.0 allowed for all crates, as merged under R11;
   - (b) narrow them to per-crate exceptions for `epaint_default_fonts` (and bundled font files);
   - (c) turn off `default_fonts` and load system fonts.

   Then update CLAUDE.md and the spec licence list to match. This joins the UI research's decision 1 (IBM Plex,
   OFL) [R9].
3. **GitHub App registration** (already the code's design). Register the App with Contents: read, Metadata: read,
   device flow and expiring tokens, install it on Metallic-Club for `studio` only, and paste the client ID. Approve
   updating plan Task 13 and spec 1C to say so.
4. **Where releases live.** Choose between the private `studio` repo for Studio, Launcher and `stable` (spec and
   Launcher plan) and separate releases repos per product. This decides how finding 1 in section 2 is fixed.
5. **Code signing.** When (before any public `stable`?), which kind (Azure Trusted Signing, OV or EV), and who pays.

## 10. Suggested follow-up issues (not created)

1. Spike: Luau breakpoint pause by yield vs `lua_break` on the merged VM (#22 prototype tests 1-6)
2. Script: switchable compiler debug level for Studio Play (`Lua::set_compiler`)
3. Script: pin `mlua = "=0.12.2"` before the debugger uses `mlua::ffi`
4. Script: limits for tree-walking API calls (`GetDescendants`) that the interrupt cannot stop
5. Render: request `TIMESTAMP_QUERY`/`POLYGON_MODE_LINE` when available; decide eframe device config vs own loop
6. Workspace: move wgpu, winit and glam to `[workspace.dependencies]`; CI fails on two wgpu majors
7. Profiler: own egui timeline over puffin frame data (correct the performance research's `puffin_egui` note)
8. ~~Updater: `download` reads only the 10 newest releases (`GithubSource`)~~: filed as #202, fixed by #237
9. Updater: pin velopack and vpk to the same exact version (stable 1.2.161 or exact 1.3.39)
10. Updater: keyring target `github.MetallicStudio` and Enterprise roaming; fix docs or move to keyring 4 with Local (docs part filed in #206)
11. Docs: update plan Task 13 and spec 1C from OAuth app / `repo` scope to the GitHub App
12. Licences: record the owner decision on OFL/Ubuntu and CDLA in CLAUDE.md and `deny.toml`
13. Updater: use `wait_exit_then_apply_updates` and wire the 6 h background check off the UI thread

---

## Sources

All accessed 2026-10-30. Crate versions, dates and licences come from the crates.io API that day unless marked
"crate source".

**Repo** (`origin/main` at `96aceb0`)
- [R1] `deny.toml`
- [R2] `Cargo.toml`, `crates/*/Cargo.toml`, `Cargo.lock`, `cargo tree -d`, `gh issue view 5 6 7 9 13 22 23`
- [R3] `crates/metallic_render/src/context.rs`
- [R4] `docs/superpowers/plans/2026-10-30-engine-m1.md` (Task 13); `docs/superpowers/specs/2026-10-30-metallic-studio-design.md` (§5, §7 1C)
- [R5] `docs/research/2026-10-30-platform-architecture.md`
- [R8] `CLAUDE.md` §6
- [R9] `docs/research/2026-10-30-studio-ui-design.md` (§1, §3.8, §5.2, §5.14, §7, §8)
- [R10] `docs/research/2026-10-30-performance-and-scale.md` (§1, §2.4, §4, §5.3)
- [R11] `crates/metallic_studio/src/main.rs`, `crates/metallic_studio/Cargo.toml`
- [R12] `crates/metallic_studio/src/github_auth.rs`, `crates/metallic_studio/src/updater.rs`
- [R13] `tools/release.py`, `docs/releasing.md`, `ARCHITECTURE.md`
- [R14] `docs/agents/13-installer-update.md`
- [R15] `crates/metallic_script/src/vm.rs`
- [R16] `crates/metallic_script/src/api/signals.rs`
- [R17] `crates/metallic_script/src/task_lib.rs`
- [R18] `crates/metallic_script/tests/vm.rs`, `tests/vm_round2.rs`
- [R19] `crates/metallic_script/src/api/instance.rs`
- [R20] `docs/superpowers/plans/2026-10-30-launcher-l1.md` (architecture line, decisions table, Task 1)
- [R21] `crates/metallic_runtime/Cargo.toml`, `crates/metallic_player/Cargo.toml`, `LICENSES/THIRD-PARTY-FONTS.md`, `docs/agents/8-runtime-parkour.md`, commit `cc68340`, `docs/CONDUCTOR.md`, issue #44 (rulings list)
- [R22] `crates/metallic_player/src/window.rs` (`impl ApplicationHandler for App`), `crates/metallic_player/src/overlay.rs` (`egui_wgpu::Renderer`)
- [R23] Publication source at `5c84cf7566901c73bd9de3621ab6ad384add8030`: `crates/metallic_script/src/task_lib.rs`,
  `crates/metallic_studio/src/commands.rs`, `crates/metallic_update/src/feed.rs`, `crates/metallic_update/src/feed/tagged.rs`.

**Studio stack**
- [E1] https://crates.io/api/v1/crates/egui-wgpu
- [E2] https://crates.io/api/v1/crates/egui-wgpu/0.37.0/dependencies
- [E3] https://raw.githubusercontent.com/emilk/egui/main/CHANGELOG.md
- [E4] https://docs.rs/egui-wgpu/latest/egui_wgpu/trait.CallbackTrait.html
- [E5] https://raw.githubusercontent.com/emilk/egui/main/crates/egui_demo_app/src/apps/custom3d_wgpu.rs
- [E12] https://crates.io/api/v1/crates/egui_ltreeview
- [E13] https://github.com/LennysLounge/egui_ltreeview
- [E14] https://crates.io/api/v1/crates/egui-probe
- [E15] https://crates.io/api/v1/crates/egui_code_editor
- [E16] https://docs.rs/crate/egui_extras/latest/features
- [E17] https://crates.io/api/v1/crates/epaint_default_fonts
- [E18] https://crates.io/api/v1/crates/eframe/0.37.0
- [E19] https://crates.io/api/v1/crates/egui/0.37.0 ; https://crates.io/api/v1/crates/epaint/0.37.0
- [E20] crates.io dependency lists (`https://crates.io/api/v1/crates/<name>/<version>/dependencies`): egui-wgpu 0.36.2, egui_plot 0.38.0, egui-probe 0.13.0, egui_code_editor 0.4.2, egui_logger 0.13.0, egui_ltreeview 0.10.0, transform-gizmo 0.11.0
- [E21] crates.io crate records (`https://crates.io/api/v1/crates/<name>`): eframe, egui-winit, syntect, onig, fancy-regex, tree-sitter, tree-sitter-highlight, tree-sitter-luau, undo, tracing, tracing-subscriber, egui_logger, semver, serde_json, puffin_http, stats_alloc, cap

**Profiler and debug views**
- [P1] https://github.com/EmbarkStudios/puffin
- [P2] https://crates.io/api/v1/crates/puffin
- [P3] https://docs.rs/crate/puffin/0.20.0
- [P4] https://docs.rs/crate/puffin_egui/latest
- [P6] https://crates.io/api/v1/crates/tracy-client
- [P7] https://crates.io/api/v1/crates/tracing-tracy
- [P8] https://crates.io/api/v1/crates/profiling
- [P10] https://docs.rs/wgpu/latest/wgpu/struct.Features.html
- [P11] https://crates.io/api/v1/crates/wgpu-profiler ; https://raw.githubusercontent.com/Wumpf/wgpu-profiler/main/CHANGELOG.md
- [P12] https://github.com/Wumpf/wgpu-profiler
- [P13] https://docs.rs/wgpu/latest/wgpu/struct.Device.html
- [P14] https://docs.rs/mlua/latest/mlua/struct.Lua.html
- [P15] https://docs.rs/rapier3d/latest/rapier3d/pipeline/struct.DebugRenderPipeline.html
- [P16] https://docs.rs/rapier3d/latest/rapier3d/pipeline/struct.DebugRenderMode.html
- [P17] https://crates.io/api/v1/crates/egui_plot
- [P18] https://crates.io/api/v1/crates/zip
- [P19] https://crates.io/api/v1/crates/tracing-chrome
- [P20] https://create.roblox.com/docs/studio/microprofiler

**Luau debugger and VM** (source read at mlua `v0.12.2` and Luau `0.740`)
- [L1] https://crates.io/api/v1/crates/mlua ; https://crates.io/api/v1/crates/mlua/0.12.2/dependencies
- [L2] https://crates.io/api/v1/crates/mlua-sys ; https://crates.io/api/v1/crates/mlua-sys/0.13.0/dependencies
- [L3] https://crates.io/api/v1/crates/luau0-src
- [L4] https://github.com/mlua-rs/mlua/blob/v0.12.2/src/state.rs
- [L5] https://github.com/mlua-rs/mlua/blob/v0.12.2/src/debug.rs
- [L6] https://github.com/mlua-rs/mlua/blob/v0.12.2/src/thread.rs
- [L7] https://github.com/mlua-rs/mlua/blob/v0.12.2/mlua-sys/src/luau/lua.rs
- [L8] https://github.com/mlua-rs/mlua/blob/v0.12.2/mlua-sys/src/luau/compat.rs
- [L9] https://github.com/mlua-rs/mlua/blob/v0.12.2/src/chunk.rs ; https://github.com/mlua-rs/mlua/blob/v0.12.2/src/luau/mod.rs
- [L10] https://github.com/mlua-rs/mlua/blob/v0.12.2/src/state/extra.rs
- [L11] https://github.com/mlua-rs/mlua/blob/v0.12.2/CHANGELOG.md
- [L12] https://github.com/luau-lang/luau/blob/0.740/VM/include/lua.h
- [L13] https://github.com/luau-lang/luau/blob/0.740/VM/src/ldebug.cpp
- [L14] https://github.com/luau-lang/luau/blob/0.740/VM/src/lvmexecute.cpp
- [L15] https://github.com/luau-lang/luau/blob/0.740/VM/src/ldo.cpp
- [L16] https://github.com/luau-lang/luau/blob/0.740/VM/src/lstate.cpp
- [L17] https://github.com/luau-lang/luau/blob/0.740/Compiler/include/luacode.h
- [L19] https://luau.org/sandbox
- [L20] https://create.roblox.com/docs/studio/debugging
- [L21] https://github.com/sssooonnnggg/luau-debugger (no licence file; reference only)
- [L22] https://github.com/JohnnyMorganz/luau-lsp
- [L23] https://github.com/mlua-rs/mlua/blob/v0.12.2/LICENSE
- [L24] https://github.com/mlua-rs/mlua/blob/v0.12.2/src/memory.rs

**Installer, updates, sign-in**
- [I1] crates.io crate records: velopack, ureq, webpki-roots, ring, keyring, keyring-core, windows-native-keyring-store, oauth2, octocrab, self_update, axoupdater, windows
- [I2] https://api.nuget.org/v3-flatcontainer/vpk/index.json
- [I3] https://crates.io/api/v1/crates/velopack/1.2.161/dependencies
- [I5] https://docs.rs/velopack/1.2.161/velopack/struct.UpdateManager.html
- [I7] https://docs.rs/velopack/1.2.161/velopack/struct.VelopackAsset.html ; https://docs.rs/velopack/1.2.161/velopack/struct.UpdateInfo.html
- [I10] https://docs.velopack.io/packaging/installer
- [I13] https://docs.velopack.io/packaging/signing
- [I17] https://docs.rs/crate/velopack/1.2.161/source/src/sources/github.rs ; https://github.com/velopack/velopack/tree/develop/src/lib-rust/src/sources
- [I18] https://docs.github.com/en/rest/authentication/permissions-required-for-github-apps
- [I20] https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api
- [I24] https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/scopes-for-oauth-apps
- [I25] https://docs.github.com/en/organizations/managing-oauth-access-to-your-organizations-data/about-oauth-app-access-restrictions
- [I26] https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app
- [I27] https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/refreshing-user-access-tokens
- [I31] https://docs.rs/keyring/4.2.0/keyring/
- [I32] https://docs.rs/keyring-core/1.0.1/keyring_core/struct.Entry.html
- [I33] https://docs.rs/windows-native-keyring-store/latest/windows_native_keyring_store/
- [I34] https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw
- [I35] Crate source velopack 1.3.39-g8d2844f from crates.io (`src/sources/github.rs`, line with `releases?per_page=10&page=1`), read locally from the Cargo registry
- [I36] Crate source keyring 3.6.3 from crates.io (`src/windows.rs`: `format!("{user}.{service}")`, `CRED_PERSIST_ENTERPRISE`; `Cargo.toml` licence), and velopack 1.3.39-g8d2844f `Cargo.toml` licence, read locally from the Cargo registry
