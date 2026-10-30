# Metallic Studio: design

Date: 2026-10-30. Status: draft for owner approval.
Repos: `Metallic-Club/studio` (this one, the editor and engine), `Metallic-Club/plateform` (website and backend; designed
separately, see "Platform link" below). Both are private and were empty on 2026-10-30.

## 1. Intent

Metallic Studio is the creator tool of Metallic, the owner's game platform. People use it to make 2D and 3D games
that are published to and played on Metallic. The owner's aim is a studio that is better than Roblox Studio, Godot,
Unity and Unreal: easier to learn, better looking, faster, with multiplayer built in.

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
- Monetisation, payouts and the marketplace backend: they belong to the platform repo and come after the platform
  design.

## 4. Anti-goals (a failure even if it technically works)

- Looks like stock Godot with a new logo. Every screen a creator sees must look and read as Metallic.
- Ships features that do not work end to end. Each milestone is judged by doing the thing, not by code existing.
- Slow on ordinary PCs. A template game must run smoothly on a mid-range laptop, not only on a gaming PC.
- Beginners hit an empty screen. A new user must reach "my game is running" without reading docs.
- Fake progress: status is reported as planned / implemented / tested / accepted / released, never inflated.

## 5. Constraints

- Brand (owner's rules): name "Metallic" (never "Metallic Studios"); deep graphite/charcoal, silver/titanium,
  restrained violet; the emblem is symbol-only (no letter M, no monogram); no neon, glow, shields or starbursts.
- Brand assets: for now only a placeholder app icon, `branding/placeholder-icon.png` (a violet-black crow head on a
  dark graphite rounded square). The owner says it may change, so keep icon and splash files easy to swap (one place
  in the repo, no copies baked into code). Editor theme colours are taken from this icon within the brand rules until
  final brand files arrive.
- Licensing: Godot, Jolt and Box2D are MIT. Keep their copyright and licence notices in the source and in an
  in-app "third-party licences" screen. Do not copy code from Roblox, Unity or Unreal.
- No secrets in git: no server addresses, user names, keys or tokens. Read keys from local files or environment.
- No pushes to `main` without the owner's say-so; work on branches `studio/<item>-<yyyymmdd>`.
- PC load (owner's rule): engine builds are heavy. Full engine compiles run 00:00-08:00 America/Chicago; in the
  daytime only incremental builds and light work.
- Windows 11 is the first and only platform until milestone 3.

## 6. Decided (owner and assistant)

- **Base: a fork of Godot 4** (assistant's recommendation, owner agreed). Reason: it already has a working 2D/3D
  editor, C#, a plugin system, an asset importer and Jolt 3D physics, so effort goes into what makes Metallic better.
  Bevy (Rust) was considered and set aside: no editor, so the first usable studio would be much further away.
- **We own the code.** The fork is fully rebranded and any part, the renderer first, may be rewritten.
- **Renderer track** starts at milestone 2 and runs alongside everything else, aiming at Unreal-level lighting and
  performance over time, with low-end performance never sacrificed.
- **Physics**: Jolt for 3D (Godot's built-in option). "Box3D" is read as "a Box2D-quality 3D engine"; if the owner
  means a specific library, revisit. Box2D (v3) is added as a 2D physics module.
- **Games run in one shared Metallic player** (like Roblox): players install Metallic once and games download into
  it. Follows from Metallic being a platform. Consequence: creator code runs on players' PCs, so it must be
  sandboxed (no file, network or OS access beyond what the engine grants).
- **Languages, in order** (assistant's call, from the sandbox need): **Luau** first, the sandboxed Lua dialect Roblox
  uses (MIT licence, built for untrusted code, familiar to Roblox creators). Then C#, Rust and Java, each running
  inside a WebAssembly sandbox so they are as safe as Luau. Godot's own GDScript stays inside the engine for internal
  tools but is hidden from creators; stock C# (.NET) is not exposed because it cannot be sandboxed.
- **Rebrand depth** (assistant's call): everything a creator sees (names, icons, executable, start screen, theme,
  menus, the Luau API names) is Metallic. Internal Godot file formats, C++ names and namespaces stay as they are so
  upstream Godot fixes can still be merged.
- **Templates are written multiplayer-ready from the start** (assistant's call): Roblox-style split into server and
  client scripts, server-authoritative character and checkpoints, even though milestone 1 runs both in one process.
- **Milestone order** (each ends in something the owner can try):
  1. First slice: Metallic-branded editor on Windows; new project from the Parkour template; edit the level; press
     Play.
  2. Multiplayer: test with a friend, dedicated server, standard and proximity voice chat. Renderer track starts.
  3. Templates: FPS with view models, Horror, Platformer, Battle Royale, Build example.
  4. Creation workflow: asset browser, local backups, version control, then Team Create.
  5. (ongoing from 2) Renderer upgrades.
  6. Built-in modeller, animation editor and VFX editor, linked live to the game.
  7. Toolbox/Marketplace, community, the learning course, Litha as the in-studio teacher.

## 7. Milestone 1 in detail (what gets planned first)

A creator on Windows 11:
1. Starts "Metallic Studio" (own name, icon, splash and window title; Metallic theme in the editor).
2. Sees a Metallic start screen with "New game from template"; Parkour is there with a preview image.
3. Creates the project; the Parkour level opens in the 3D viewport with a playable character, checkpoints, a few
   obstacle types and a finish.
4. Moves, adds and deletes obstacles with the normal editor tools.
5. Presses Play; the character runs, jumps and reaches the finish; checkpoints respawn on fall.

The Parkour template's game code is Luau (server and client scripts), so milestone 1 includes a first Luau
integration: enough API to drive the character, checkpoints and finish.

Acceptance: the owner does steps 1-5 on this PC from a built installer or zip, with no stock Godot name, logo or
default theme visible on those screens. Performance (assistant's call): at least 60 fps at 1080p on high settings on
this PC (RTX 3060 Ti, Ryzen 5 5500, 32 GB); at least 30 fps at 1080p on low settings on integrated laptop graphics
(Intel Iris Xe class), checked later on such a machine or marked "not verified".

Art (assistant's call): template art comes from CC0 asset packs plus art the owner makes with ChatGPT; nothing with
an unclear licence.

## 8. Platform link

`Metallic-Club/plateform` is the website and backend (accounts, discovery, economy, moderation, servers). It gets its
own design after this one; the owner's raw list is in that repo at `docs/platform-feature-list.md`. Studio depends on
it from milestone 2 on (sign-in, publishing, server lists, voice relay). Until then Studio works fully offline.
Sign-in from Studio, when it comes, must not handle passwords inside the editor (browser or device sign-in flow).

## 9. Left to the builder

- Which Godot 4 stable release to fork (latest stable at fork time) and how to track upstream fixes.
- Repo layout, as long as upstream Godot changes can still be merged.
- How branding is applied (renaming binaries, theme, start screen) and the exact theme values within the brand rules.
- Template internals for Parkour (character controller, level pieces), within the acceptance in section 7.
- Build tooling (SCons, MSVC or MinGW), CI, packaging (zip or installer) for milestone 1.
- Which Luau binding to build on (an existing MIT one, or our own module), and how the editor's Play button and the
  shared player share code.
- Networking, voice codec and server design for milestone 2: to be designed when milestone 2 starts.
