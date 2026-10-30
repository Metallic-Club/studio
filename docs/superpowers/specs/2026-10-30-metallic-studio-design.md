# Metallic Studio: design

Date: 2026-10-30 (revision 2: own engine in Rust, replacing the Godot-fork revision the same day).
Status: draft for owner approval.
Repos: `Metallic-Club/studio` (this one: the Metallic Engine and Metallic Studio), `Metallic-Club/plateform` (website
and backend; designed separately, see "Platform link").

## 1. Intent

Metallic Studio is the creator tool of Metallic, the owner's game platform. People use it to make 2D and 3D games
that are published to and played on Metallic. The owner's aim is a studio and engine that are better than Roblox
Studio, Godot, Unity and Unreal: easier to learn, better looking, faster, with multiplayer built in, and **our own
engine**, written by us, not a fork of someone else's.

Why it matters: Roblox Studio is easy but limited and looks dated; Unity and Unreal are powerful but hard for beginners
and do not ship multiplayer, voice or a marketplace as one piece. Metallic Studio should give beginners a working,
good-looking multiplayer game from a template in minutes, and give experienced creators the depth to go further.

## 2. Goals (long-term target)

The long-term feature target is the owner's own list plus a Roblox Studio parity list (about 600 features in 20 areas:
core systems, world, rendering, lighting, physics, scripting, animation and characters, audio, UI, input, networking,
data, effects, assets, services, navigation, editor tools, monetisation, security, platforms). Owner's own list:

- 2D and 3D games in one engine, easier to use than any of the four above
- Game code in C#, Rust, Lua and Java
- Strong optimisation; a render system that keeps improving toward Unreal level
- Physics: Box2D for 2D; a 3D physics engine ("Box3D" in the owner's words, see section 6)
- Built-in modeller (Blender-like), animation editor and VFX editor, linked live to the game
- Plugins and add-ons
- Team Create, version control, asset browser, local backups
- Toolbox/Marketplace: templates, code, shaders, audio, models, textures
- Templates: Parkour, First Person Shooter with view models, Horror, Platformer, Battle Royale, a Build example, more
- Community; voice chat (standard and proximity)
- Learning section: an interactive course from basics to advanced
- An AI that teaches you how to work in the studio (Litha, the platform's game-dev assistant)

The owner will add more; this document covers the direction and the first milestones, not the full list.

## 3. Non-goals for now

- Consoles (Xbox, PlayStation) and VR: later, after Windows, then macOS/Linux/mobile.
- Java scripting: last of the four languages (hardest to sandbox well); not before Luau, C# and Rust work.
- Monetisation, payouts and the marketplace backend: they belong to the platform repo.
- Forking an existing engine (Godot, Bevy, Unity, Unreal). Libraries are fine (section 6); the engine is ours.

## 4. Anti-goals (a failure even if it technically works)

- Feels like a tech demo: every screen a creator sees must look and read as a finished Metallic product.
- Ships features that do not work end to end. Each milestone is judged by doing the thing, not by code existing.
- Slow on ordinary PCs. A template game must run smoothly on a mid-range laptop, not only on a gaming PC.
- Beginners hit an empty screen. A new user must reach "my game is running" without reading docs.
- Fake progress: status is reported as planned / implemented / tested / accepted / released, never inflated.

## 5. Constraints

- Brand (owner's rules): name "Metallic" (never "Metallic Studios"); deep graphite/charcoal, silver/titanium,
  restrained violet; the emblem is symbol-only (no letter M, no monogram); no neon, glow, shields or starbursts.
- Brand assets: for now only a placeholder app icon, `branding/placeholder-icon.png` (a violet-black crow head on a
  dark graphite rounded square). The owner says it may change, so icon and splash files live in `branding/` only and
  are easy to swap. Editor theme colours are taken from this icon within the brand rules until final brand files
  arrive.
- Licences: only permissive dependencies (MIT, Apache-2.0, BSD, Zlib, ISC, Unicode, MPL-2.0 as files); no GPL/LGPL/
  AGPL; checked automatically (`cargo deny`). Every dependency's licence is listed in an in-app "third-party
  licences" screen and in the packaged `LICENSES/`. Do not copy code from Roblox, Unity, Unreal or Godot.
- No secrets in git: no server addresses, user names, keys or tokens. Read keys from local files or environment.
- No pushes to `main` without the owner's say-so; work on branches `studio/<item>-<yyyymmdd>`.
- PC load (owner's rule): the owner's PC stays responsive while they use it; heavy builds and long test runs at night
  (00:00-08:00 America/Chicago) or while the owner is idle; daytime builds with at most 4 parallel jobs.
- Windows 11 is the first and only platform until milestone 3; nothing may block macOS/Linux later.

## 6. Decided

Owner decisions:
- **Our own engine, from scratch, written in Rust** (owner, 2026-10-30, after the assistant laid out Rust vs C++).
  The engine is called the **Metallic Engine**; the editor is **Metallic Studio**; the game app players use is the
  **Metallic player**.
- **Games run in one shared Metallic player** (like Roblox): players install Metallic once and games download into
  it. Creator code runs on players' PCs, so it must be sandboxed (no file, network or OS access beyond what the engine
  grants).

Assistant's calls (owner may override):
- **Libraries, not engines.** We use proven libraries for the hard low-level pieces and write everything else
  ourselves:
  - Graphics: `wgpu` (DirectX 12 on Windows, Vulkan, later Metal). The renderer on top is ours: physically based
    materials, shadows, sky and fog first, then global illumination and more as a dedicated renderer track.
  - Window and input: `winit`, gamepads via `gilrs`.
  - 3D physics: Rapier (pure Rust, Apache-2.0) behind our own physics interface, so Jolt or our own solver can
    replace it later. "Box3D" is read as "a Box2D-quality 3D engine"; if the owner means a specific library, revisit.
  - 2D physics: Box2D (v3) through Rust bindings, when 2D arrives (milestone 3).
  - Scripting: Luau through `mlua` with its sandbox mode.
  - Editor UI: `egui`, fully restyled with the Metallic theme; a custom UI toolkit can replace it later.
  - Model import: glTF 2.0.
- **World model like Roblox:** a tree of Instances (Workspace, Parts, Models, Folders, Scripts, ...), each with a class
  name, properties and children. Creators see this tree in the Explorer and in Luau. Internals (data layout, ECS or
  not) are ours to choose for speed.
- **Parts first:** levels are built from Roblox-style Parts (block, sphere, cylinder, wedge) with physical materials
  (plastic, metal, wood, concrete, glass, grass, ...). This needs no imported art, so templates can be built inside
  Studio itself. Imported glTF models come next.
- **Files are text:** projects and scenes are saved as human-readable text files (one scene per file), so version
  control and Team Create can diff and merge them later.
- **Languages, in order:** **Luau** first (the sandboxed Lua dialect Roblox uses, MIT, familiar to Roblox creators).
  Then C#, Rust and Java, each compiled to WebAssembly and run in a sandbox, so they are as safe as Luau.
- **Server and client from the start:** scripts are split Roblox-style into server scripts and client scripts with
  RemoteEvents between them, even while milestone 1 runs both in one process.

Milestone order (each ends in something the owner can try):
  1. **Engine and Studio first slice** (two halves, below): the Parkour game plays in the Metallic Engine, and
     Metallic Studio can open, edit and play it.
  2. Multiplayer: test with a friend, dedicated server, standard and proximity voice chat. Renderer track starts.
  3. 2D (Box2D) and more templates: FPS with view models, Horror, Platformer, Battle Royale, Build example.
  4. Creation workflow: asset browser, glTF import, local backups, version control, then Team Create.
  5. (ongoing from 2) Renderer upgrades toward Unreal-level lighting and performance.
  6. Built-in modeller, animation editor and VFX editor, linked live to the game.
  7. Toolbox/Marketplace, community, the learning course, Litha as the in-studio teacher.

## 7. Milestone 1 in detail

**1A: the engine runs a game.** On Windows 11, `metallic-player` opens a window and plays the Parkour game from its
project folder: a lit 3D world of Parts with shadows and a sky, a third-person character (walk, run, jump, camera),
checkpoints, a moving platform, lava, a spinning bar and a finish that shows your time. All game logic is Luau server
and client scripts using Roblox names (`game`, `workspace`, `script.Parent`, `.Touched`, `task.wait`, RemoteEvent).
A bad script (endless loop, error) never freezes or crashes the game.

**1B: Metallic Studio edits it.** A creator:
1. Starts Metallic Studio (own name, crow icon, splash and Metallic theme).
2. Sees a start screen with "New game from template"; Parkour is there with a preview image.
3. Creates the project; the level opens in a 3D viewport with an Explorer (the Instance tree), a Properties panel and
   an Output panel.
4. Selects, moves, rotates, scales, adds (Part, SpawnLocation, Checkpoint) and deletes things; undo/redo works; saves.
5. Presses Play; the game runs inside Studio; Stop returns to the edited level unchanged.

Milestone 1 details (assistant's calls):
- **Roblox API scope:** Roblox names and behaviour, not source compatibility with existing Roblox games. Milestone 1
  exposes the subset Parkour needs, listed in the plan and documented in `docs/luau-api.md`; it grows each milestone.
- **Character like Roblox:** a character Model with `HumanoidRootPart` and a `Humanoid` (Health 100, WalkSpeed 16,
  JumpPower 50, MaxSlopeAngle 89, states running/jumping/falling/dead, `Died`), driven by our own kinematic capsule
  controller on Rapier; it is carried by moving platforms and pushed by moving obstacles. The avatar is our own
  blocky six-part figure (head, torso, arms, legs) in titanium grey with violet accents. Units are studs.
- **Checkpoint and SpawnLocation are built-in classes** (special Parts): touching a Checkpoint sets the player's
  respawn point and lights it up, with no script needed; scripts can still listen to its `Touched`.
- **Minimal UI for scripts:** `ScreenGui`, `Frame`, `TextLabel` (enough for the finish time and a checkpoint counter).
- **Play runs in a separate world** cloned from the edit world; Stop throws it away, so the edited level is untouched.
- **The player app:** `metallic-player <project folder>` runs a game on its own, without Studio. Downloading games
  comes with the platform (milestone 2+).
- **Sandbox for milestone 1:** Luau runs in-process with a per-resume time budget (0.1 s), a memory cap per script VM
  (256 MB) and no file, network or OS access. Separate script processes come with multiplayer servers.
- **Parkour size:** about 150-300 Parts, 2-3 minutes to finish. Stress target: 5,000 Parts on screen still at 60 fps on
  this PC.
- **Editor tools:** selection and multi-select, move/rotate/scale gizmos with snapping (1 stud, 15 degrees), insert
  Part/SpawnLocation/Checkpoint/Script, delete, duplicate, undo/redo, Properties panel editing every property of the
  selected Instances, save. The template preview image is a Studio screenshot.

Acceptance: the owner does steps 1-5 on this PC from a built zip. Performance: at least 60 fps at 1080p on high
settings on this PC (RTX 3060 Ti, Ryzen 5 5500, 32 GB) while playing Parkour; at least 30 fps at 1080p on low settings
on integrated laptop graphics (Intel Iris Xe class), checked on such a machine or marked "not verified".

## 8. Platform link

`Metallic-Club/plateform` is the website and backend (accounts, discovery, economy, moderation, servers). It gets its
own design after this one; the owner's raw list is in that repo at `docs/platform-feature-list.md`. Studio depends on
it from milestone 2 on (sign-in, publishing, server lists, voice relay). Until then Studio works fully offline.
Sign-in from Studio, when it comes, must not handle passwords inside the editor (browser or device sign-in flow).

## 9. Left to the builder

- Crate layout of the Cargo workspace, internal data structures (ECS or plain arenas), threading.
- Exact text file format for scenes and projects (must stay human-readable and diff-friendly).
- Renderer internals for milestone 1 (forward or clustered, shadow technique) within the fps targets.
- Which Rust bindings for Box2D (milestone 3), how WebAssembly languages are hosted (later).
- Exact theme values within the brand rules; editor panel layout details.
- Networking, voice codec and server design for milestone 2: designed when milestone 2 starts.
