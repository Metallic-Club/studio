# Templates and new-creator onboarding: research and decisions

Date: 2026-10-30. Status: proposal for the owner. Author: Claude (research session).
Scope: what makes a creator template great, how a new user gets from first launch to a published game, and which
templates Metallic should ship, in what order, with what engine work.

Companion files: `docs/templates/README.md` (shared template conventions and the API gap table),
`docs/templates/quality-bar.md` (the checklist every template must pass), and ten detailed template designs in
`docs/templates/`.

How to read the evidence: **Sourced** means a page was fetched in this session and says it (URL in section 8).
**Search only** means a search-result summary said it but the page was not opened. **Judgment** is my reasoning.
Decisions marked "assistant's call" are proposals the owner can override.

---

## 1. Decisions in short

1. **Every template ships as a finished, multiplayer-ready game**, not a demo: it can be won or finished, works solo,
   and is server-authoritative from day one. Unreal and Unity templates that added multiplayer later are the main
   pain point in their forums (2.2).
2. **"Make it mine" in under five minutes without code**: one-click **look presets** restyle the whole game, a
   **Game settings** panel exposes 6 to 15 rule knobs (attributes on one `Configuration`), and renaming takes one
   action. Code comes after, through marked `-- TRY:` lines (3).
3. **The first screen asks one question, "What do you want to make?"**, shows genre tiles with real play loops, and
   creates the project in one click with name and folder filled in. First Play within 60 s of first launch (4).
4. **The tutorial lives inside the template**: 5 to 7 single-action steps in the same arc for every template (play,
   restyle, tune, build, change one line, play, share). Short tours finish; long ones do not (2.3).
5. **Litha explains and points first, edits only with a visible plan and the creator's approval**, and asks the
   creator to make the next small change by hand. Tutorials work without Litha (4.3).
6. **Achievements count making, not clicking** (published, changed a script, finished a track). No punishing daily
   streaks (4.4).
7. **Catalog: 27 templates plus 4 empty starters**, ranked. Top 10 after Parkour: Platformer (2D), Tycoon, FPS Arena,
   Horror, Build Sandbox, Battle Royale, Party Rounds, Simulator, Tower Defense (isometric), Survival Nights (5).
8. **Templates never rot**: each one has a headless smoke test that runs in CI on every engine change, and a
   `template_version` so shared modules can be updated in creators' projects later (Unity's microgames died of
   version rot, 2.2).
9. **The biggest engine gaps for templates are not in any plan yet**: ModuleScripts and `require`, attributes, tags,
   clickable UI, CFrame, input and camera APIs, raycasts in Luau, an audio subsystem, local lights with shadows, a
   data store, and a view-model render layer (6).

---

## 2. Findings

### 2.1 What people play now

**Roblox.** A live snapshot on 2026-10-30 05:55 UTC (off-peak; the prior 24 h ranged 8.1M to 13.3M concurrent)
showed 9.2M players across 17K experiences. Leaders: Steal An Egg 928K, Murder Mystery 2 434K, Brookhaven RP 269K,
Steal a Brainrot 263K, Adopt Me! 210K, 99 Nights in the Forest 121K, Blox Fruits 115K, RIVALS 93K, Throw Food For
Animals 86K, Jujutsu Shenanigans 72K, Fish It! 65K, Dandy's World 60K, Pet Simulator 99 57K (Sourced, rowatcher; a
third-party tracker, not Roblox's own chart). Roblox's own December 2025 recap says "roleplaying and simulation games
led", with horror and roleplay the top broad searches; by device, mobile leans to horror, escape and obby, PC to RPG
and fighting (Sourced, Roblox newsroom). Steal a Brainrot peaked at 25.4M concurrent in October 2025 and Grow a Garden
at 22.3M or more in August 2025 (Sourced, Wikipedia). Naavik reports that 2025's three breakout hits (Grow a Garden,
Steal a Brainrot, 99 Nights) each drew more monthly engagement than Fortnite Battle Royale, and that before 2025 growth
was spread across many small obby and tycoon games (Sourced, secondary).

**Fortnite islands.** On 2026-10-29 (Epic's data API, via a tracker), the top 20 islands by peak players were 6
Simulation and Tycoon (led by a "steal" game at 48K and a droid tycoon), 6 Shooter (2v2, 1v1, red vs blue, free for
all), 5 Battle Royale-style (box fights, zone wars), 2 Party and Mini Games (the second most played island, 43K peak,
850K unique players) and 1 Strategy (a bed-defence game). Tycoon islands had the best next-day return, 45% to 83%;
shooters 30% to 43% (Sourced, endgametools). In 2024, creator islands took 36.5% of Fortnite play time, Epic paid
$352M to about 70,000 creators, and over 30% of creator-island time went to non-combat genres such as RPG, party and
horror (Sourced, Tubefilter).

**Judgment.** Across both platforms the genres that hold players are: collect-and-earn (tycoon, simulator, "steal",
garden, fishing, pets), shooters and arena fights, round-based party and social-deduction games, co-op horror and
survival, and roleplay. Obby and platformers are the classic first game. Tower defense and racing are steady but
smaller. Templates should cover the first group well because beginners copy what they play.

### 2.2 How other platforms do templates

| Platform | What ships | What works | What fails | Source |
|---|---|---|---|---|
| Roblox Studio | 23 templates: gameplay (Platformer, Laser Tag, FPS System, Racing, Classic Racing, Line Runner, Capture the Flag, Team/FFA Arena, Combat, Concert, Move It Simulator, Mansion of Wonder, Classic Obby, UGC Homestore), environments (Modern City, Village, Castle, Suburban, Pirate Island), starters (Starting Place, Baseplate, Classic Baseplate, Flat Terrain) | 2024 templates are configured through attributes and a constants module; the Platformer has a README and extra comments; Racing makes a new race by duplicating a folder with numbered checkpoints | Laser Tag's view model "clips through walls"; mobile HUD off-centre; old templates (Line Runner, Capture the Flag) look dated and two were removed from the picker in 2024; Platformer's moving platforms jitter under latency; Racing's reverse camera is "REALLY jarring"; no beginner tycoon, roleplay or survival template; the one architecture sample (Plant) is "not suitable for beginners" | Sourced: Roblox docs and DevForum threads |
| Roblox onboarding | Onboarding tour, a first-experience tutorial (3D platformer), playable templates, curriculum tracks | In September 2026 Roblox reported about 9,000 games published from its prompt-based **Build** tab, and 71% of Build creators had never used Studio | Roblox itself says the old Studio UI overwhelmed new creators; the new UI drew "far more wasteful of screen space" and "feels way too empty"; Assistant replies report "hallucinated methods" and edits "when I never asked it to"; free models with hidden `require` backdoors | Sourced: RDC 2026 post, DevForum |
| Fortnite UEFN | 15 Feature Examples (learn one system) and 14 Game Examples (fork a working mode: Gunfight 2v2, Prop Hunt, Parkour, My First Island, Speedway Race with persistence...); brand templates (LEGO, Fall Guys) bake rules and age ratings into the island | Clear split between "learn one system" galleries and "fork a game" templates; Fall Guys islands ship a sample obstacle course with rules preset | Verse onboarding is a six-step path with no time estimates; a 2026 review says "budget at least a few weeks" and lost two hours to a failable call; Creative projects cannot move into UEFN without a rebuild | Sourced: Epic docs, uefncentral |
| Unity | Microgames (FPS, LEGO, Karting...) taught by "mods": small guided changes in four groups (power-ups, level design, visuals, tuning); Starter Assets; Unity 6.3 multiplayer templates | The mod idea: about 10-minute guided changes to a working game | Microgames stopped being updated past 2021/2022 and quietly left the funnel; LEGO "only supports the 2021 editor"; 6.3 multiplayer templates shipped with broken asset files and a 2 h 15 min shader compile | Sourced: Unity forums, Learn |
| Unreal | Base templates (First Person, Third Person, Top Down, Vehicle) with **variants** (Arena Shooter, Survival Horror, Combat, Side Scroller, Platforming, Strategy, Twin Stick, Offroad, Timetrial) | Variants: one base, several genres, picked from a dropdown | The 5.6 FPS template breaks with two players; one creator spent about 8 hours making it multiplayer; Lyra "isn't the ideal project for beginners" | Sourced: Epic docs, forums, itch devlog |
| Godot | "Dodge the Creeps" 2D tutorial in 7 parts with an asset zip; 13 categories of demo projects | Learners remix the finished tutorial game | Screenshots and versions drift; no beginner path through the demos | Sourced: Godot docs, forum, GitHub |
| GDevelop | Start from "a game template that can be customized in a few minutes", then Preview; in-app tutorials as open data files; AI "Build for me" and "AI Chat" | Template first, preview second; tutorials as data | GDevelop says new developers "often begin several projects without completing any"; the AI agent "is not an instant make-my-whole-game button" and has no undo | Sourced: GDevelop wiki, blog |
| Construct 3 | Example browser with hundreds of tagged examples, 12 "heavily commented" beginner templates | Comments as teaching | (no fetched evidence) | Search only |
| Core | Lua, kitbashing, frameworks for deathmatch, capture point and battle royale | Republish a framework "in seconds" (search only) | A top game averaged 10 to 15 concurrent players; good templates did not create player demand | Sourced: Naavik 2022, Wikipedia |
| Rec Room | Maker Pen, Circuits, Rec Room Studio (Unity-based) | | Shut down 1 June 2026. Its CEO: mobile and console users made "millions of pieces of content" but the devices were not "good for building stuff that other players engaged with"; most creators "couldn't" because "we didn't provide them with the necessary tooling"; Maker AI "frustrated our more impactful creators" | Sourced: Game Developer, Road to VR |
| Dreams | "Stealth create": arrange existing items, then alter prefabs, then build your own; a remix ladder; prizes from play unlock elements for making | Making grows out of playing; remixing is opt-in | Tutorials took hours (search only); motion controls drew accessibility complaints; live service ended 2023 | Sourced: Game Developer, Wikipedia |
| Super Mario Maker | SMM1 unlocked tools over nine days, patched at launch to unlock by activity; SMM2's Yamamura's Dojo: 45 short lessons on design craft (fairness, direction, testing, feedback) | Teach design craft, not only buttons; gate on actions, not on days | Time-gated unlocks | Sourced: Wikipedia, Mario Wiki |
| LittleBigPlanet | Story items feed creation; playable tutorials with a subset of tools (search only) | Play feeds making | | Sourced: Wikipedia |
| itch.io and Unity Asset Store | Top-selling templates: retro shooter, survivors-like, deckbuilder, souls-like, visual novel UI; Asset Store top kits: top-down, 2D/2.5D platformer, FPS, horror (3 kits), survival, RPG | Buyers praise learning value and modular parts | A souls-like author: genre-locked templates are "not as useful as I hoped"; buyers want systems that work "with any camera angle" | Sourced: itch, Asset Store |

### 2.3 Onboarding research

- **Short tours finish, long ones do not.** Across 550M tour interactions, completion was 72% for 3 steps, 74% for 4,
  and 16% for 7 or more; tours launched from a checklist completed at 67% versus 31% for others; tours the user starts
  get about twice the engagement of automatic ones (Sourced, Chameleon). Average onboarding checklist completion in
  547 SaaS products: 19.2% (Sourced, Userpilot).
- **Front-loaded tutorials can make a simple app feel harder.** In a 70-person study, people who read an up-front
  tutorial rated tasks harder (4.92 vs 5.49 of 7, p = 0.047) with no gain in success (Sourced, Nielsen Norman Group).
- **Tutorials pay off in complex games, in context.** Andersen et al. (CHI 2012, about 45,000 players): tutorials
  helped engagement in the complex game and little in simple ones; teaching a mechanic where it is used made them
  more efficient (Sourced via Cao and Liu 2022 and Cheung et al. 2014; the original paper was blocked). A game engine
  is complex, so guidance helps, if it is in context and skippable (Judgment).
- **Ask what they want to make.** Canva asked new users what they wanted to design and personalised the start; it
  raised activation (share who complete a design) by 10%, up to about 12% after tuning (Sourced, Appcues).
- **A first project that is clearly yours predicts learning.** In 3 years of Scratch data, making an original project
  in the first 14 days raised the odds of later using new programming concepts 1.55 times; remixing alone lowered them
  to 0.74 (Sourced, Cheng and Hill, CSCW 2022). Over a longer span, users who remix more have larger programming
  repertoires (Sourced, Dasgupta et al., CSCW 2016). So: start from a template, but make a real change early
  (Judgment).
- **Low floor, wide walls.** Resnick: "It's not enough to provide a single path from low floor to high ceiling; we
  need to provide wide walls" (Sourced). Many genres at the same easy start is the wide-walls idea (Judgment).
- **AI helps novices finish but can hollow out learning.** Novices aged 10 to 17 with Codex completed 1.15 times more
  tasks with 1.8 times higher scores and did no worse on later manual edits (Sourced, Kazemitabaar et al., CHI 2023).
  But learners who generated a whole solution from one prompt scored 96% on authoring and 62% on modifying code later;
  a hybrid of manual and AI work scored 77% and 73% (Sourced, Kazemitabaar et al., 2023). Roblox reported that
  creators who used Assistant or other AI tools at least once increased publishes by an estimated 31% (Sourced,
  PocketGamer; correlational). Roblox's Assistant now has a Planning Mode that shows an editable plan before it builds
  (Sourced, Roblox docs).
- **Badges steer behaviour toward what they count.** On Stack Overflow, users shift effort toward a badge as they near
  it (Sourced, Anderson et al., WWW 2013). Duolingo's streak animation raised new learners' day-7 retention by 1.7%
  (Sourced, Duolingo blog). Small effects; aim them at making things (Judgment).

### 2.4 Lessons for Metallic (Judgment, each tied to 2.1 to 2.3)

1. **Beginners want to change a working game, not assemble one.** Unity's mods, Dreams' stealth create, GDevelop's
   template-then-preview and Roblox's Build tab all start from something that already runs.
2. **Ship the multiplayer version first.** Unreal's and Unity's forums show what retrofitting costs; on Metallic every
   game is online.
3. **The template must keep working for years.** Unity's microgames rotted; Roblox's oldest templates look 2015-era.
   CI smoke tests and versioned shared modules prevent both.
4. **Quality by default protects the platform.** Rec Room's flood of low-engagement content is the warning. Templates
   that look and play well by default raise the floor of what gets published.
5. **Rules knobs beat code for a first change.** Roblox's best templates already moved tuning into attributes; we make
   that a first-class panel with labels and safe ranges.
6. **Variants are cheap breadth.** Unreal's base-plus-variant model gives many genres from few bases. Our variants are
   settings presets and look presets on one template.
7. **Teach design craft as well as buttons.** Yamamura's Dojo teaches fairness and testing; our templates carry short "design
   notes" next to the parts they explain (why the first gap is small, why spawn protection exists).
8. **AI must show its work.** Roblox Assistant's top complaints are invented API members and unrequested edits;
   learning research says one-shot generation hurts later skill.
9. **Keep the first screen to one question.** Roblox's new UI drew complaints from both sides; Canva's one question
   raised activation.
10. **Plan for the genres people play now.** Collect-and-earn, shooters, party and social deduction, co-op horror and
    survival, roleplay. Roblox ships no beginner template for most of them.

---

## 3. What makes a template great

Every point below is a rule in `docs/templates/README.md` and a line in `docs/templates/quality-bar.md`.

| Quality | What it means in a Metallic template |
|---|---|
| Playable instantly | Press Play and within 10 s you know the goal and within 30 s you have done the core action. Solo always works; genres that need more players fill the gap (practice range, solo run, warm-up round) |
| A finished game | It can be won or finished, then played again without restarting Studio. No placeholder text, no test parts |
| Multiplayer-ready | Server decides, client shows. Every RemoteEvent is listed with its server checks. Late joiners get current state. Leaving never leaves a round stuck |
| Easy to restyle | One click applies a **look preset**: lighting, sky, fog, a palette of material roles applied through tags, and the UI theme. Nine presets ship (Titanium, Toybox, Meadow, Dusk, Noir, Frost, Foundry, Paper, Pixel) |
| Remix knobs as settings | A **Game settings** panel shows 6 to 15 attributes with labels, ranges and help text. Each extreme leaves a playable game. Variants are named settings presets ("Tag", "One shot", "Easy") |
| Clearly commented scripts | Line 1 says what the script does; a header lists remotes, settings and tags; 2 or 3 `-- TRY:` lines per script work as written; no script over about 200 lines |
| Tutorial hooks | `tutorial.toml` steps with completion checks Studio observes; Litha reads the same steps |
| Swap-in assets | Visuals made of Parts sit in named Models (`ViewModels.Rifle`, `GizmoModels.BoltBug`); replace the Model, keep the name, nothing else changes |
| Built from Parts | M3 templates need no imported art, so they work before glTF import (M4) and creators can edit every shape in Studio |
| Teaches one big idea | Each template's scripts explain one transferable idea: rounds (Party Rounds), cheap NPC crowds (Tower Defense), server-checked placement (Build), persistence (Tycoon) |
| Proven | A headless smoke test per template runs in CI on every engine change; acceptance checks are ticked with evidence |

---

## 4. Onboarding: first launch to first publish

### 4.1 The flow

Times are targets for a first-time user on the reference PC (assistant's call). Each screen states the one thing it
asks of the user.

| Step | Screen | User does | Target |
|---|---|---|---|
| 0 | Install, first launch, splash | Nothing. No sign-in wall: Studio works offline until publishing (spec section 8) | splash under 2 s |
| 1 | **"What do you want to make?"** Eight large genre tiles, each a silent 4 to 6 s loop of real play (Obby, Shooter, Horror, Platformer 2D, Tycoon, Build, Party, Battle royale), plus "All templates" and "Start empty". Returning users see "Open recent" first | Click a tile | 15 s |
| 2 | **Template card**: name, one-line pitch, players, the loop, a look picker (default selected), "Make it mine". Name and folder are pre-filled (`Documents\Metallic\Games\My Horror Game`); "More options" holds the folder picker | One click | 10 s |
| 3 | **Studio opens on the level.** The Play button is highlighted; the tutorial panel shows step 1, "Press Play to try your game" | Press Play | first Play by 60 s from launch |
| 4 | **Playing.** The template's own HUD states the goal. The creator plays until they choose to stop | Play, then Stop | 1 to 3 min |
| 5 | **Make it yours** (tutorial steps 2 and 3): look picker, then the Game settings panel | Pick a look, change one knob, play again | "mine" within 5 min |
| 6 | **Build on it** (step 4): duplicate and move one thing the template points at | Ctrl+D, drag | 1 min |
| 7 | **Change one line** (step 5): the script opens at a `-- TRY:` line, highlighted | Edit a number, play | 2 min |
| 8 | **Name and share** (step 7): rename the game; until the platform exists, "Save and share" exports a zip. From M2, **Publish**: sign in through the browser (device flow, no password in Studio), title, description, thumbnail picked from three captures made during Play, visibility Private (default), Friends or Public | Publish | under 30 s |
| 9 | **After publishing**: game page link, "Invite a friend to test" (M2), and the learning path's next three suggestions | | |

Rules for the flow:
- The welcome screen replaces nothing in plan Task 9; it is the "New" tab of the same start screen, with the
  template cards Task 9 already specifies. Experienced users can make it open on "Recent" by default.
- Skipping any step never blocks the next. The tutorial panel is a checklist the creator can do in any order after
  step 1 (checklist-launched tours finish twice as often, 2.3).
- No modal pop-ups after step 3. Hints sit in the tutorial panel or as a coach mark on the thing they describe.
- A public publish of an unchanged template is refused with "Change something first: a look, a setting or the
  level" (assistant's call). Private publishing is always allowed. This keeps clone floods off discovery (Rec Room
  lesson) and nudges the "original first project" that predicts learning (Cheng and Hill).

### 4.2 Interactive tutorial inside the template

Format and completion checks: `docs/templates/README.md` section 5. Every template follows the same seven-beat arc,
so the second template a creator opens feels familiar: **play it, restyle it, tune it, build on it, change one line,
play again, share**. Each step's text is 25 words or fewer and has a check Studio can observe (a touch during Play, a
setting change, a script edit). Steps run as a checklist, not a forced sequence.

Templates also carry **design notes**: short cards attached to Instances (an attribute `DesignNote` with one or two
sentences) that Studio shows when the creator selects that Instance. Example on the Platformer's first gap: "This gap
is 3 studs so everyone clears it. The first jump teaches the button, not the skill." This is the Yamamura's Dojo idea
placed where the creator is already looking (assistant's call).

### 4.3 Litha's role

Litha is milestone 7 in the spec. Onboarding must work fully without it; Litha makes it better.

| Litha does | How |
|---|---|
| Explains | "What does this script do?" answered from the script's header comment and code; "Why did my game break?" answered from the Output error and the line it names |
| Points | Answers "how do I make the jump higher?" by highlighting the Game settings knob or the Properties field, not by changing it |
| Narrates the tutorial | Reads the current step aloud or in the panel and expands it on request ("show me where") |
| Edits only with consent | Shows a plan and the exact diff (Instances and code), applies it as one undo step on approval, and writes "Litha changed: ..." to Output |
| Hands the next change back | After it writes code, it suggests the next small change for the creator to make by hand, with the line highlighted |
| Stays inside the real API | Generates only against `docs/luau-api.md` for the current engine version and runs the Luau type checker on its output before showing it, so it cannot suggest members that do not exist |

Litha never inserts assets from unknown sources, never publishes, never changes Game settings or scripts silently, and
can be turned off. These rules answer the top Roblox Assistant complaints (invented methods, unrequested edits) and
the learning research on one-shot generation (2.3).

### 4.4 Learning path and achievements

- **Next steps panel** after the first session: three suggestions based on the open template, each a 10-minute guided
  change in the Unity "mod" style (Platformer: "add a moving platform", "make a second level", "add a boss crawler").
- **Tracks**: Build, Script, Design, Multiplayer, Art. Each is a sequence of mods across templates, ending in a small
  original game. The spec's M7 interactive course grows from these.
- **Achievements count making**: First Play, Made it mine (look, setting and name changed), First script change, First
  new part, First publish, First 10 players, First friend test, Track finished. Private by default; shown on the
  creator profile if they choose. No daily streaks with loss; an optional weekly "made something" goal at most
  (assistant's call, from the badge and streak evidence in 2.3).

### 4.5 What to measure (opt-in telemetry, assistant's call)

| Metric | Target to start with |
|---|---|
| Time from first launch to first Play (median) | under 90 s |
| Share of first sessions that reach "mine" (look, setting or level change, plus Play after it) | 60% |
| Tutorial checklist completion | 50% (sources put short flows at 50% to 75%) |
| First-session publish rate (from M2) | track, no target until there is data |
| Creators who return within 7 days | track |

Run Canva-style experiments on a small share of new users once there is traffic. A useful first one: welcome screen
with and without the intent question.

---

## 5. Template catalog

Ranked by priority, which is also the proposed build order within each milestone. Milestones follow spec section 6;
"M4 wave" means templates built during M4 (creation workflow) once M2 multiplayer and the M3 2D and API work exist.
API group ids (A1 to A28) are defined in `docs/templates/README.md` section 6. "Detailed" links the full design.

| # | Template | Dim. | Genre | Milestone | Why it matters | Engine and API work it needs (beyond M1) | Default look |
|---|---|---|---|---|---|---|---|
| 0 | Parkour | 3D | Obby | M1 (planned in Task 8) | The first game; proves the engine | none beyond M1; retrofit to A1-A3 conventions in M3 | `titanium` |
| 1 | **Platformer** ([detailed](../templates/platformer-2d.md)) | 2D | Platformer | M3 | 2D flagship, lowest floor, proves the 2D stack | A23 2D (Box2D, Part2D, Camera2D, 2D controller), A2, A3, A8, A12, A21 | `paper` |
| 2 | **Tycoon** ([detailed](../templates/tycoon.md)) | 3D | Tycoon, collect-and-earn | M3 | The top genre on both platforms; teaches persistence and server-checked purchases | A17 data store, A15 prompts, A8, A21 world UI, A6 | `toybox` |
| 3 | **FPS Arena** ([detailed](../templates/fps-arena.md)) | 3D | Shooter | M3 (owner) | Owner's list; 6 of Fortnite's top 20 islands are shooters; builds the weapon kit | A22 view models, A7 raycast, A19 camera, A20 mouse lock, A9 teams, A11 tools, A16 | `foundry` |
| 4 | **Horror** ([detailed](../templates/horror.md)) | 3D | Co-op horror | M3 (owner) | Owner's list; top Roblox search; NPC states, objectives, scares | A13 spot lights with shadows, A14 audio, A15, A10 `MoveTo`, A7 | `noir` |
| 5 | **Build Sandbox** ([detailed](../templates/build-sandbox.md)) | 3D | Creative building | M3 (owner) | Owner's "Build example"; turns players into builders; placement code reused by 4 templates | A7 box queries, A17, A19 mouse rays, A21, 10,000-part rendering | `meadow` |
| 6 | **Battle Royale** ([detailed](../templates/battle-royale.md)) | 3D | Battle royale | M3 (owner) | Owner's list; a quarter of Fortnite's top 20 is BR-style | FPS kit, A16 server time, A18 glide, 640-stud draw distance, 16-24 players (M2) | `dusk` |
| 7 | **Party Rounds** ([detailed](../templates/party-rounds.md)) | 3D | Party, minigames, hidden role | M4 wave | Second most played Fortnite island is a party game; hidden-role game is #2 on Roblox; the reusable round framework | A1 is the whole design, A18 knockback on characters, A16 | `toybox` |
| 8 | **Simulator** ([detailed](../templates/simulator.md)) | 3D | Collect, sell, upgrade, pets | M4 wave | Long-running Roblox staple; base for Fishing and Garden | A11 tools, A17, A21 shop UI | `meadow` |
| 9 | **Tower Defense** ([detailed](../templates/tower-defense.md)) | 2.5D isometric | Tower defense | M4 wave | The in-between dimension; data-driven waves; cheap crowds | A19 scriptable camera, A16 unreliable remotes, A6 | `meadow` |
| 10 | **Survival Nights** ([detailed](../templates/survival-nights.md)) | 3D | Co-op survival | M4 wave | One of 2025's three breakout genres; day and night | A13 `ClockTime` sun and many point lights, A11, 30 NPC Humanoids | `meadow` |
| 11 | Horde Survivor | 2D top-down | Survivors-like | M4 wave (2D) | Top-selling itch template genre; co-op; tests 2D crowds | A23, 500 sprites on screen, A12 | `pixel` |
| 12 | Kart Racing | 3D | Racing | M4 wave | Classic beginner want; Roblox's racing template drew camera complaints we can avoid | A18 `LinearVelocity`, `AlignOrientation`, `SetNetworkOwner` (M2), A20 gamepad axes, A14 engine pitch | `dusk` |
| 13 | Roleplay Town | 3D | Roleplay, life sim | M4 wave (needs glTF art) | Brookhaven is a top-3 Roblox game; jobs, houses, vehicles | A27 imported models, A28 chat, A15, A17; later A26 animation | `meadow` |
| 14 | Obby Tower | 3D | Vertical obby | M3 | Cheapest new 3D template: random stacked sections from Parkour pieces | A1, A2, A24 `Random.new` | `toybox` |
| 15 | Side-on Runner | 2.5D side-on | 3D platformer on a plane | M3 | Shows 2.5D with no new API besides camera and plane lock | A18 `PlaneConstraint`, A19 | `frost` |
| 16 | Fishing | 3D | Collection, minigame | M4 wave | Two fishing games among Roblox's most played in the October 2026 snapshot; Simulator core | Simulator core, A12, A21 timing bar | `meadow` |
| 17 | Garden | 3D | Grow and sell | M4 wave | Grow a Garden-scale genre; teaches offline time | Simulator core, A17, A24 `DateTime` | `meadow` |
| 18 | Hangout | 3D | Social space | M2 | M2's own test world for multiplayer and voice; little code | M2 networking, voice, A28 chat, A15 seats | `titanium` |
| 19 | Escape Room | 3D | Puzzle adventure | M3 | Prompts, keys, codes; strong on mobile per Roblox's device data | A15, A21 `TextBox` keypad, A12 | `noir` |
| 20 | Idle Clicker | 2D (UI only) | Idle, incremental | M3 | The smallest template: UI only, teaches numbers and saving | A21, A17 | `paper` |
| 21 | Puzzle Grid | 2D | Push-block or match puzzle | M3 | Tiny 2D template with a level editor feel; tile maps | A23 `TileMap2D`, A20 | `paper` |
| 22 | Top-down Adventure | 2D | Action adventure | M4 wave (2D) | Tile maps, NPC dialogue, inventory; Asset Store's top kit genre | A23, A21 dialogue UI | `pixel` |
| 23 | Arena Fighter | 3D | Battlegrounds, fighting | M6 | Two fighting games among Roblox's most played in the October 2026 snapshot | A26 animation, A25 VFX, A18 knockback | `foundry` |
| 24 | Card Battler | 2D (UI) | Deckbuilder, turn-based | M4 wave | Top-selling itch template genre; turn logic | A21 drag and drop, A1 | `paper` |
| 25 | Story | 2D | Visual novel | M4 wave | Ren'Py kits sell well; dialogue trees anyone can write | A21 rich text, A1 | `paper` |
| 26 | Base Wars | 3D | Team strategy | M4 wave | A bed-defence game is in Fortnite's top 20; teams, gather, build, raid | Build Sandbox placement, A9, A11 | `toybox` |
| 27 | Ball Arena | 3D | Physics sport | M4 wave | Physics showcase; car or on-foot ball game | A18 impulses, ball network ownership (M2) | `titanium` |

**Empty starters** (not ranked; each is a project with a camera, a character, one look preset and nothing else):

| Starter | Ships | Contents |
|---|---|---|
| Empty 3D (baseplate) | M1 (assistant's call: add to Task 9's start screen, it is one scene file) | 512 x 512 baseplate, SpawnLocation, `titanium` |
| Empty 2D | M3 | Ground strip, SpawnLocation2D, Camera2D, `paper` |
| Empty side-on 2.5D | M3 | Baseplate, side camera script, `PlaneConstraint`, `frost` |
| Empty isometric | M3 | Baseplate, isometric camera script, `meadow` |

**Milestone summary.** M1: Parkour, Empty 3D. M2: Hangout. M3: Platformer, Tycoon, FPS Arena, Horror, Build Sandbox,
Battle Royale (the owner's five plus Tycoon), then Obby Tower, Side-on Runner, Escape Room, Idle Clicker, Puzzle Grid
and three empty starters (small, mostly reuse). M4 wave: Party Rounds, Simulator, Tower Defense, Survival Nights,
Horde Survivor, Kart Racing, Roleplay Town, Fishing, Garden, Top-down Adventure, Card Battler, Story, Base Wars, Ball
Arena. M6: Arena Fighter. M7: community templates through the Marketplace.

Why Tycoon joins the owner's five in M3 (assistant's call): it is the most played genre on both platforms (2.1),
needs no engine work beyond what the five already need except a data store, and it is a starter-difficulty template
where the owner's five are mostly intermediate or advanced.

---

## 6. Engine and Studio work the templates need

### 6.1 API groups not in any plan yet

Plan Task 7 covers the M1 Parkour API only (listed in `docs/templates/README.md` section 6). Count of the 10 detailed
templates that need each group (Battle Royale counts everything it inherits from FPS Arena):

| Group | Needed by | Note |
|---|---|---|
| A1 `ModuleScript` and `require` | 10 of 10 | M1's sandbox removes `require` entirely; it must come back as `require(moduleScript)` with an Instance argument only |
| A2 attributes and `Configuration` | 10 of 10 | The Game settings panel depends on it |
| A3 `CollectionService` tags | 10 of 10 | Look roles and most game logic |
| A21 clickable and world UI | 10 of 10 | M1 has no button at all |
| A4 more Instance members | 10 of 10 | `Changed`, `ChildAdded`, `Destroying`... |
| A12 `TweenService` | 9 of 10 | Feedback and moving things |
| A14 audio | 10 of 10 | **No audio subsystem is planned anywhere.** Horror fails without it |
| A6 `CFrame` | 9 of 10 (all 3D) | M1 uses Position and Orientation only |
| A20 input | 9 of 10 | M1 drives input in the engine; scripts cannot read keys |
| A10 character control | 9 of 10 | `LoadCharacter`, `MoveTo`, `HealthChanged`, `CameraMode` |
| A19 camera | 8 of 10 | `CurrentCamera`, `RenderStepped`, `ViewportPointToRay` |
| A7 raycasts and box queries in Luau | 7 of 10 | The Rust side exists in Task 4 |
| A8 value objects and `leaderstats` | 8 of 10 | Plus a built-in player list |
| A16 `RemoteFunction`, unreliable remotes, server time | 7 of 10 | |
| A13 local lights, lighting properties | 6 of 10 | Renderer: shadowed spot lights, many point lights, a moving sun |
| A15 `ProximityPrompt` | 6 of 10 | |
| A11 `Tool` and backpack | 4 of 10 | |
| A17 data store | 3 of 10 | Studio emulation first, platform store from M2 |
| A18 physics control | 3 of 10 | Knockback on the kinematic character needs controller support |
| A22 view-model layer | 2 of 10 | Metallic addition; Roblox's own template clips through walls |
| A23 2D | 1 of 10 (plus 7 catalog entries) | Proposal in `docs/templates/README.md` section 7 |

Recommendation (assistant's call): add A1, A2, A3, A4, A12, A20 and A21 to the M2 API work, because every template
needs them and they are small next to networking. Audio needs its own task (an audio crate on a permissive library,
positional sound, `Sound` and `SoundService`), and should be planned before M3 starts.

### 6.2 Engine work beyond the API

- An **audio subsystem** (nothing planned).
- **Local lights with shadows**: at least 5 shadowed spot lights and 20 point lights at 60 fps (Horror), a moving sun
  (Survival).
- **10,000 Parts at 60 fps** (Build Sandbox at default limits; M1's stress target is 5,000).
- **30 NPC Humanoids** at 60 Hz on the server (Survival).
- **A view-model render pass** (FPS, Battle Royale).
- **Knockback and impulses on the kinematic character controller** (Party Rounds, Platformer stomp bounce).
- **2D stack** (spec M3): Box2D, 2D renderer path, 2D controller with coyote time and jump buffering.

### 6.3 Studio work (not in M1's plan)

- **Game settings panel** reading `[[setting]]` from `template.toml` (README section 2).
- **Look presets**: preset files, role tags, one-command apply, a look picker with thumbnails (README section 3).
- **Tutorial runner** reading `tutorial.toml`, coach marks and completion checks (README section 5).
- **Design notes** shown on selection (4.2).
- **Welcome screen** with the intent question and video tiles (4.1); preview video capture for templates.
- **Template versioning**: `template_version` in `metallic.toml` and a "Update template modules" command that
  replaces shared ModuleScripts the creator has not edited.
- **Template CI**: each template's smoke test runs headless on every pull request that touches the engine.
- **Publish dialog** (M2, with the platform).

---

## 7. What I could not verify, and open questions

Could not verify:
- No official Roblox figure for the share of Studio users who publish, and no official Studio onboarding-tour data.
- No Roblox template announcements in 2025 or 2026; whether the Plant reference project is still open source.
- Fortnite.gg genre statistics (HTTP 403); Fortnite data used here comes through a third-party tracker of Epic's API.
- Core's status: no shutdown found (Wikipedia and a delisting tracker do not report one); search snippets mention
  layoffs in 2026.
- Construct 3 pages (HTTP 403); its claims here are search-only.
- The Andersen et al. CHI 2012 paper itself (blocked); its findings come through two papers that cite it.
- No independent study comparing interactive walkthroughs to video; vendor blogs only.
- Unity AI, Rosebud, Canva Magic, Khan Academy and Codecademy evidence was not gathered (search budget ran out in one
  research thread).
- Concurrent-player numbers are snapshots from third-party trackers on one day; treat order, not exact numbers, as
  the finding.

Open questions for the owner:
1. Should Tycoon join the owner's five in M3 (section 5)? It is the most played genre and a starter template.
2. Should a public publish of an unchanged template be refused (4.1)?
3. Should the shipped look presets follow the Metallic brand rule (no neon, no glow) even though they style creators'
   games? I assumed yes.
4. Is an audio task acceptable before M3 (6.1)? Without it, Horror cannot pass the quality bar.
5. The 2D API proposal (`docs/templates/README.md` section 7) needs the M3 design session's decision.

---

## 8. Sources

Fetched in this session:

Platforms and genres
- Roblox templates list: https://create.roblox.com/docs/resources/templates
- Roblox Laser Tag template thread: https://devforum.roblox.com/t/new-laser-tag-studio-template/3033810
- Roblox Platformer template thread: https://devforum.roblox.com/t/new-platformer-template/3088494
- Roblox Racing template thread: https://devforum.roblox.com/t/new-studio-racing-template/2805745
- Roblox Plant reference project: https://create.roblox.com/docs/resources/plant-reference-project
- Roblox full-project resources thread: https://devforum.roblox.com/t/roblox-official-full-project-resources/2522805
- Next Gen Studio UI beta: https://devforum.roblox.com/t/next-gen-studio-ui-preview-is-here-beta/3075390
- Flexible Studio UI rollout: https://devforum.roblox.com/t/an-update-on-the-new-flexible-studio-ui/4085392
- "Make Roblox development easy to learn": https://devforum.roblox.com/t/make-roblox-development-easy-to-learn/715270
- Assistant complaints: https://devforum.roblox.com/t/the-ai-assistant-feature-sucks/2740750
- Assistant full release: https://devforum.roblox.com/t/full-release-use-assistant-to-boost-your-productivity/3294217
- Assistant feedback: https://devforum.roblox.com/t/feedback-on-roblox-assistant-responses/3799181
- Free-model backdoors: https://devforum.roblox.com/t/can-someone-delete-these-popular-models-for-viruses/2977832
- Creator Roadmap 2025 recap: https://devforum.roblox.com/t/creator-roadmap-2025-end-of-year-recap/4156739
- RDC 2026: https://about.roblox.com/newsroom/2026/09/rdc-2026-the-world-needs-more-play
- Roblox Replay 2025: https://about.roblox.com/newsroom/2025/12/roblox-replay-decoded-search-style
- Roblox Assistant guide: https://create.roblox.com/docs/assistant/guide
- Roblox learning path: https://create.roblox.com/docs/experiences
- Roblox live counts: https://rowatcher.com/roblox-player-count
- Steal a Brainrot: https://en.wikipedia.org/wiki/Steal_a_Brainrot
- Naavik, State of UGC Games 2026: https://naavik.co/deep-dives/the-state-of-ugc-games-2026/
- Naavik on Manticore: https://naavik.co/digest/a-spotlight-on-manticore-games/
- Roblox Studio AI chief interview: https://www.pocketgamer.biz/roblox-studio-chief-on-how-ai-will-revolutionise-ugc/
- Fortnite island top 20 (Epic data API): https://endgametools.com/en/fortnite/map-codes
- UEFN starter templates: https://dev.epicgames.com/documentation/fortnite/unreal-editor-for-fortnite-starter-templates
- Verse onboarding: https://dev.epicgames.com/documentation/fortnite/onboarding-guide-to-programming-with-verse-in-unreal-editor-for-fortnite
- Fall Guys islands: https://dev.epicgames.com/documentation/en-us/fortnite/building-fall-guys-islands-in-fortnite-creative
- LEGO Islands: https://www.lego.com/en-be/aboutus/news/2024/march/LEGO-Islands-Unreal-Editor-Fortnite
- Fortnite 2024 creator payouts: https://www.tubefilter.com/2025/01/23/fortnite-epic-games-creator-payouts-2024/
- Creative vs UEFN 2026: https://uefncentral.com/blog/fortnite-creative-vs-uefn-2026
- Core: https://en.wikipedia.org/wiki/Core_(video_game) and https://delistedgames.com/companies/manticore-games/
- Rec Room shutdown: https://www.gamedeveloper.com/business/rec-room-shutting-down-after-failing-to-become-sustainability-profitable-
- Rec Room layoffs: https://roadtovr.com/rec-room-layoff-half-staff-august-2025/
- Rec Room Studio: https://www.uploadvr.com/rec-room-studio-unity-graphics/
- Dreams toolset: https://www.gamedeveloper.com/design/how-media-molecule-designed-a-fun-and-robust-toolset-for-i-dreams-i-
- Dreams: https://en.wikipedia.org/wiki/Dreams_(video_game)
- Super Mario Maker: https://en.wikipedia.org/wiki/Super_Mario_Maker
- Super Mario Maker 2: https://en.wikipedia.org/wiki/Super_Mario_Maker_2
- Yamamura's Dojo: https://www.mariowiki.com/Yamamura%27s_Dojo
- LittleBigPlanet: https://en.wikipedia.org/wiki/LittleBigPlanet
- Fall Guys: https://en.wikipedia.org/wiki/Fall_Guys

Engines and template stores
- LEGO Microgame thread: https://discussions.unity.com/t/what-happened-to-the-lego-microgame/342692
- Microgames not appearing: https://discussions.unity.com/t/unity-microgames-are-not-appearing/940671
- FPS Microgame course: https://learn.unity.com/project/fps-template
- FPS Microgame launch: https://gamefromscratch.com/unity-release-new-fps-template-and-tutorial-series/
- Unity pathways: https://learn.unity.com/pathways
- Unity Starter Assets: https://assetstore.unity.com/packages/essentials/starter-assets-thirdperson-updates-in-new-charactercontroller-package-196526
- Unity 6.3 multiplayer templates: https://discussions.unity.com/t/unity-6-3-new-multiplayer-templates-available-in-the-hub/1700722
- Unreal template variants: https://dev.epicgames.com/documentation/unreal-engine/variants-in-game-templates
- Unreal 5.6 templates guide: https://www.unreal-university.blog/unreal-engine-5-6-complete-guide-to-the-new-templates/
- Unreal templates reference: https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-engine-templates-reference
- Unreal 5.6 FPS multiplayer thread: https://forums.unrealengine.com/t/multiplayer-error-with-new-ue-5-6-fps-templates/2558668
- Lyra devlog: https://itch.io/devlog/539954/unreal-engine-test-version-current-state.amp
- Godot first 2D game: https://docs.godotengine.org/en/stable/getting_started/first_2d_game/index.html
- Godot forum (Dodge the Creeps): https://forum.godotengine.org/t/dodge-the-creeps-help/125760
- Godot demo projects: https://github.com/godotengine/godot-demo-projects
- GDevelop getting started: https://wiki.gdevelop.io/gdevelop5/getting_started/
- GDevelop home: https://gdevelop.io/
- GDevelop in-app tutorials: https://github.com/GDevelopApp/GDevelop-tutorials
- GDevelop courses post: https://gdevelop.io/blog/make-successful-games-with-gdevelop-courses
- GDevelop AI agent post: https://gdevelop.io/blog/make-games-with-ai-agent-gdevelop-automated-prompt
- itch.io top template sellers: https://itch.io/game-assets/top-sellers/tag-template
- itch.io top Godot assets: https://itch.io/game-assets/top-sellers/tag-godot
- Souls-like template page: https://felesmachina.itch.io/cats-godot-souls-like-template
- Unity Asset Store systems templates: https://assetstore.unity.com/templates/systems
- GameMaker Fire Jump: https://gamemaker.io/en/tutorials/fire-jump-dnd-1

Onboarding and learning research
- Low floor, wide walls: https://gordonbrander.com/notes/low-floor-wide-walls-high-ceiling/
- Remixing and computational thinking (Dasgupta et al. 2016): https://www.media.mit.edu/publications/remixing-as-a-pathway-to-computational-thinking/
- Many Destinations, Many Pathways (Cheng and Hill 2022): https://arxiv.org/pdf/2211.04046
- Scratch in-editor tutorials: https://www.scratchfoundation.org/ (in-editor tutorials page) and https://en.scratch-wiki.info/wiki/Getting_Started_with_Scratch
- Minecraft Education coding: https://www.microsoft.com/en-us/education/blog/2022/12/anyone-can-teach-coding-with-minecraft/
- MakeCode for Minecraft Education: https://minecraft.makecode.com/setup/minecraft-education-edition
- Userpilot benchmarks: https://userpilot.com/blog/product-metrics-benchmark-report/
- Userpilot walkthroughs: https://userpilot.com/blog/interactive-software-walkthroughs/
- Chameleon product tours: https://www.chameleon.io/blog/mastering-product-tours
- Chameleon benchmark: https://www.chameleon.io/benchmark-report-2022
- NN/g mobile tutorials: https://www.nngroup.com/articles/mobile-tutorials/
- Canva growth process: https://www.appcues.com/blog/canva-growth-process
- Tutorial design (Cao and Liu 2022, citing Andersen et al. 2012): https://pmc.ncbi.nlm.nih.gov/articles/PMC9676530/
- The First Hour Experience (Cheung et al. 2014): https://www.microsoft.com/en-us/research/ (First20Hour preprint PDF)
- Kazemitabaar et al. CHI 2023: terpconnect.umd.edu (PDF)
- Kazemitabaar et al. Koli Calling 2023: https://arxiv.org/html/2309.14049v1
- Steering User Behavior with Badges: https://www.cs.cornell.edu/home/kleinber/www13-badges.pdf
- Duolingo streaks: https://blog.duolingo.com/how-duolingo-streak-builds-habit/

Search only (not opened): Fortnite tycoon and genre lists (fortnite.gg returned 403), Roblox third-party
vulnerability docs, Construct 3 example browser and "Next Steps" guide, Unreal Game Animation Sample page, Rocket
League workshop maps, LittleBigPlanet tutorial pages.
