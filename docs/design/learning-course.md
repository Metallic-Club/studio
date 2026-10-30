# Learning course: design

Date: 2026-10-30. Status: **design** (docs only; nothing here is implemented). Issue #157, epic #20 (M7).

The spec's goal list asks for "a learning section: an interactive course from basics to advanced". This document
says what the course teaches, in what order, what a lesson file looks like, how Studio runs and checks lessons, how
progress and credentials work, and how the course stays readable and safe for young creators.

Read with:
- `docs/research/2026-10-30-templates-and-onboarding.md` (onboarding flow, the 7-step in-template tutorial, tracks
  and achievements in 4.4) and `docs/templates/` (`README.md` section 5 for `tutorial.toml`, `quality-bar.md`).
- `docs/research/2026-10-30-studio-ui-design.md` (5.4 Ctrl K "Learn" group, 5.8 guide strip, 5.9 Explain, 5.10
  Litha, 5.12 accessibility).
- `docs/research/2026-10-30-litha.md` (the five modes, the hint ladder, the age policy in 9.2).
- `docs/design/look-presets.md` on branch `studio/144-look-presets` (Looks and Themes; this doc uses its names).

Labels:
- **Sourced**: a page fetched this session, or a source an earlier Metallic doc fetched (that doc is named).
- **Search only**: a search summary said it; the page was not opened. Treat as weaker.
- **Judgment**: my reasoning.
- **Assistant's call**: a decision made while the owner was away. Section 10 lists them all.

---

## 0. Decisions in short

1. **You learn by changing a working game, then by making your own.** Every unit ends in a project you can play, and
   every lesson starts from a project that already runs (2.1, 2.2).
2. **Ten tracks, 53 units and 5 capstones, about 190 lessons of 5 to 12 minutes each.** First steps, Build, Script, Multiplayer, UI,
   Data, Art and sound, 2D, Game design, Ship. Each unit names the engine issues it needs, and Studio shows a lesson
   only when the running engine has the API it uses (section 3).
3. **One lesson format and one runner for course lessons and template tutorials.** A lesson is a folder of text
   files: `lesson.toml` (steps, checks, hints), a start state, a solution, optional Luau check scripts and
   translations. `tutorial.toml` from the templates doc becomes a lesson with `kind = "tour"` (section 4).
4. **Studio checks by doing.** Steps pass on a world query, a headless script test, an Output match or a Studio
   event. No step passes because the learner pressed Next (section 4.4).
5. **Hints climb the same four-rung ladder as Litha's Hint mode.** Authored hints work offline; Litha, when it is on,
   rewrites a hint around the learner's own code. The solution is always one click away and costs nothing (4.5).
6. **No streaks, no hearts, no timers.** Progress shows what you made. Badges are earned by a project that passes its
   checks, and link to that project (section 6).
7. **Local first and offline.** The whole course ships inside the Studio install. Progress lives on the PC. Platform
   sync comes with M7 accounts and is off for under-13 accounts until parental consent exists (5.3, 7.4).
8. **Lessons cannot rot.** CI runs every lesson's solution against its checks, and checks that the start state fails
   them, on every engine change (4.8).
9. **Reading level is a test, not a hope.** Step text has word limits and a readability gate per track, checked in CI
   (7.1).

---

## 1. Principles

1. **Build real games.** The first lesson starts in Parkour. The last lesson of each track is a small game you made.
   Scratch data shows that making an original project early predicts later use of new programming concepts, while
   remixing alone does not (Sourced in the templates doc 2.3, Cheng and Hill 2022). So each unit ends with a "Make"
   step that leaves the template behind.
2. **Short lessons.** 3 to 7 steps, 5 to 12 minutes. Tours of 3 to 4 steps finished at 72% to 74%; 7 or more at 16%
   (Sourced in the templates doc 2.3, Chameleon). A lesson that needs more steps splits into two.
3. **Check by doing.** The learner acts in the real editor and Studio observes the result. Active learning cut STEM
   failure rates from 33.8% to 21.8% across 225 studies (Search only: Freeman et al., PNAS 2014).
4. **Read before you write.** Script lessons follow Predict, Run, Investigate, Modify, Make (PRIMM). In 13 schools
   with 493 learners aged 11 to 14, PRIMM classes did better on the post-test than the control group (Search only:
   Sentance, Waite and Kallia 2019).
5. **Show a worked example, then fade it.** Worked examples beat unguided problem solving for novices; the benefit
   shrinks as skill grows, and removing steps one at a time eases the move to solving alone (Sourced: Wikipedia,
   worked-example effect, citing Sweller and Cooper 1985, Kalyuga et al. 2000, Renkl, Atkinson and Grosse 2004).
   Units fade: lesson 1 shows the full code, lesson 2 leaves one line blank, the project leaves the learner with a
   goal and the checks.
6. **Label the steps of a solution.** Short subgoal labels ("Find the player", "Stop double counting") improved
   novice app-building performance by 8% and helped students at risk of failing most (Search only: Margulieux et
   al. 2012 and 2020). Code in lessons carries one comment per subgoal.
7. **Hold the answer back, never lock it away.** A plain GPT-4 chat raised practice scores 48% and cut later exam
   scores 17%; a hint-only tutor raised practice 127% with no exam loss (Sourced in the Litha doc 2.2, Bastani et
   al., PNAS 2025). Hints come first. The solution stays one click away, because a learner stuck for 20 minutes
   quits.
8. **No streak pressure.** Broken streaks reduce later engagement, and people often abandon the activity after a
   break (Search only: Silverman and Barasch, Journal of Consumer Research 2023). The templates research already set
   "achievements count things made, no daily streaks" (templates doc 4.4). The course follows it.
9. **Teach design craft, not only buttons.** Yamamura's Dojo teaches fairness and testing in short lessons (Sourced
   in the templates doc 2.2). Mark Brown's Platformer Toolkit teaches feel by letting you move 30 sliders and play
   (Search only, gmtk.itch.io). The Game design track and the design notes in templates do this.
10. **Lessons live in the editor.** No separate website course for the core path. The guide points at the real
    control and checks the real result (Studio UI doc 3.4 and 5.8). Roblox's Core path checks your work only
    against a downloadable place file, and Unity's and Godot's tutorials drift from the current editor (2.1;
    templates doc 2.2).

---

## 2. Evidence: what other courses do

### 2.1 Platforms

| Course | What it does | What to take | What to avoid | Source |
|---|---|---|---|---|
| Roblox Creator Hub | Three short getting-started tutorials, short "use case" tutorials by topic, and three long curriculum paths (environment art, gameplay scripting on a laser tag game, UI) | Topic-grouped short tutorials; a "Core" curriculum that rebuilds one platformer in 3 chapters and 10 lessons | The Core path is for people "familiar with general coding concepts"; true beginners go elsewhere first. Progress is checked only by comparing your place to a downloaded file. No time estimates. The Core path offers "Build with Assistant" as an equal option to building yourself | Sourced: create.roblox.com tutorials and Core curriculum pages |
| Unity Learn | Pathways: Essentials (2 weeks), Junior Programmer (12 weeks), Creative Core (10 weeks); missions, XP per tutorial, a shareable badge at the end | Missions that ask you to show a skill; a credential you can share | Forum threads report tutorials on old editor versions, progress that stopped registering, and pathways that would not load; Unity says it will replace Junior Programmer content | Sourced: learn.unity.com/pathways; Search only: Unity Discussions threads |
| GDevelop | In-app tutorials as JSON data: each step names an element to highlight and one `nextStepTrigger` (value changed, object added, preview launched, editor active); `isCheckpoint` splits progress; text per locale inside the file | Tutorials as open data files; triggers observed in the editor; checkpoints | Triggers watch the UI (CSS selectors), so a redesign breaks them; one trigger per step | Sourced: GDevelop-tutorials README and REFERENCE.md |
| Scratch | In-editor tutorials stepped with arrows; printable cards that show what you make on the front and how on the back; "Imagine, Create, Share" sessions | The finished result first, then the steps; interest-based paths (music, dance, catch game) | Steps advance on a click, with no check | Sourced: codeweek.eu blog by Scratch Foundation and MIT; Search only: scratchfoundation.org |
| Code.org CS Discoveries | Game Lab unit: short lessons (5 min warm-up, 40 min activity, 5 min wrap-up), a mini-project after a few lessons | A mini-project every 3 or 4 lessons | Built for a teacher-led classroom, not self-paced | Search only: curriculum.code.org |
| Codecademy and similar | Instruction, editor and checker side by side (judgment: the common product shape) | Instant feedback on each step | "Tutorial hell": learners finish guided courses and cannot build alone. The claims are anecdotal and often from competitors | Search only |
| Exercism | Concept exercises teach one idea and unlock practice exercises; learning mode by default, practice mode unlocks everything | A concept graph decides what unlocks; an expert can skip | Learners report confusion over locked exercises | Search only: exercism.org docs and forum |
| Rustlings | Small broken exercises run in watch mode; `h` shows a hint; a fixed order "best for newcomers"; `r` resets an exercise | Fix-the-broken-thing exercises; reset per exercise; hints on demand | Terminal only | Sourced: rustlings.rust-lang.org/usage |
| Brilliant | About 15 minutes a day of interactive problems; hints that guide without giving answers; review sets that mix earlier lessons | Problem first, short text; spaced review | Learners who want a long explanation first find it thin | Search only: brilliant.org, a 2026 review |
| Duolingo | Streaks; a streak animation raised day-7 retention 1.7% | Small, honest effects of visible progress | Streak anxiety; users who reset long streaks to escape the pressure; streak breaks lead to quitting | Sourced in the templates doc; Search only: JCR 2023, press |
| Hedy | A language that adds syntax level by level until it is Python | Introduce syntax one rule at a time | A separate language; we teach real Luau | Search only: Hedy papers |

### 2.2 Learning research used here

| Finding | Use in the course | Source |
|---|---|---|
| Worked examples help novices; benefit falls with expertise; fading helps the move to solving alone | Each unit fades from full example to blank goal; experienced learners can test out (6.2) | Sourced: Wikipedia, worked-example effect |
| Parsons problems (reorder given lines) give the same learning as writing code in less time, with less load | A `parsons` step kind for the first use of each new construct | Search only: Ericson et al.; ITiCSE 2023 working group |
| PRIMM: read and predict before writing | Script units open with a Predict step | Search only: Sentance et al. 2019 |
| Subgoal labels improve novice performance | One comment per subgoal in lesson code | Search only: Margulieux et al. |
| Spaced retrieval practice helps in intro STEM courses, though results vary by course | Optional 1-question warm-ups from earlier units; never graded | Search only: Int. J. STEM Education 2024 |
| Novice misconceptions: variables, references, how the machine runs the program (the notional machine) | Lessons that show values changing; targeted feedback for known mistakes (4.6) | Search only: Qian and Lehman 2017; Sorva 2013 |
| Luau-specific traps: arrays start at 1; indexing nil after `FindFirstChild`; only `false` and `nil` are falsy; `:` versus `.` | Each trap has an authored feedback message and a Litha eval case | Search only: Roblox docs, DevForum, Luau changelog |
| One-shot AI answers hurt later skill; hint-only tutors do not | Litha in lessons runs Explain, Hint and Show me only (4.5) | Sourced in the Litha doc 2.2 |
| Dyslexia fonts do not raise reading speed or accuracy | No special font; size and spacing settings instead (7.3) | Search only: Wery and Diliberto 2016, 12 students |

Judgment: the evidence points the same way from many directions. Short active steps, examples that fade, hints
before answers, and checks on the real result. The common failure across Roblox, Unity and Godot is content that
drifts from the engine. CI on every lesson answers that.

---

## 3. Curriculum

### 3.1 Shape

- **Tracks** group units by skill. Tracks are not a strict order; the course map suggests one (3.3).
- **Units** have 3 to 5 lessons plus a project, about 45 to 75 minutes in all.
- **Lessons** have 3 to 7 steps, 5 to 12 minutes.
- **Concepts** (`events`, `remote-events`, `tables`...) are the unlock graph, as in Exercism. A lesson lists the
  concepts it teaches and the ones it needs. A learner who lacks a concept sees "Uses: events (S7)" with a link, not
  a lock. Assistant's call: suggest, never lock.
- Unit ids are a track letter and a number (S7). Lesson ids add a number (S7.2). The letter M is not used, to avoid
  confusion with milestones; Multiplayer is N.

### 3.2 Tracks

| Track | Letter | For | Units | Start template |
|---|---|---|---|---|
| First steps | F | Everyone, first hour | 4 | Parkour |
| Build | B | Levels from Parts, looks and lighting, models | 6 | Parkour, Empty 3D, Build Sandbox |
| Script | S | Luau from zero to modules, input and types | 12 | Parkour, small lesson scenes |
| Multiplayer | N | Server and client, RemoteEvents, trust, testing with friends | 5 | Parkour, Party Rounds, FPS Arena |
| UI | U | ScreenGui to accessible menus | 5 | Tycoon, Parkour |
| Data | D | Leaderstats to safe saving | 3 | Tycoon, Simulator |
| Art and sound | A | Sound, animation, VFX, game feel, materials | 5 | Horror, Arena Fighter, FPS Arena |
| 2D | T | 2D worlds, tiles, platformer feel | 3 | Platformer (2D) |
| Game design | G | Loops, fair levels, playtesting, multiplayer design, rewards | 5 | Any |
| Ship | P | Performance, publishing, selling fairly, live updates, tools | 5 | The learner's own game |

Total: 53 units in 3.4 plus 5 capstones. At 3 to 4 lessons a unit, the course has about 190 lessons and 30 to 40
hours of guided work, plus projects.

### 3.3 Suggested path

```
F1-F4 ─┬─ B1-B3 ─────────────── G1-G2 ───────────────┐
       └─ S1-S7 ─┬─ N1-N3 ─ U1-U3 ─ D1-D2 ─ G3-G5 ─ P1-P3 ── capstone: your own game, published
                 ├─ S8-S12 (any time after S7)
                 ├─ A1-A5 (as M6 ships), T1-T3 (as M3 2D ships)
                 └─ B4-B6, N4-N5, U4-U5, D3, P4-P5 (advanced)
```

Assistant's call: the Learn home shows "Next for you" as three cards from this path, following the templates doc
4.4 "Next steps panel". It never shows a percentage of the whole course; 190 lessons would discourage a beginner.

### 3.4 Units

"Needs" lists the issues or API groups (A1 to A28, `docs/templates/README.md` section 6) whose features the unit
uses. "Ships" is the earliest milestone in which every need can be met. "no issue" means no issue exists yet; section
9 proposes them.

**F: First steps** (all ship with M1 features; content ships with the lesson runner)

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| F1 Play and look around | Press Play and Stop; move the camera; know what the green Play markers mean | Finish Parkour once, then find the finish from the editor camera | #9, #10, #11, #8 | M1 |
| F2 Change the world | Select, move, rotate, scale, duplicate, undo; read Properties | Make one jump easier and one harder, then play both | #10 | M1 |
| F3 Your first script change | Open a script at a `-- TRY:` line; change a number; read Output | Make the moving platform faster; print a message when it turns | #7, #11, #131 | M1 |
| F4 Save and share | Save; rename the game; "Save and share" zip; what publishing will add | A renamed Parkour variant shared as a zip | #9, #12; publish needs M2 platform | M1 |

**B: Build**

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| B1 Parts and the Explorer | Part shapes, Anchored, CanCollide, Size, Folders and Models, naming | Bridge a gap and build a 5-jump section (example lesson B1.3) | #2, #10 | M1 |
| B2 Levels that play well | Jump distances from WalkSpeed and JumpPower, checkpoint spacing, readable hazards, design notes | A 2-minute course a friend can finish on the second try | #5, #10, design notes (#83) | M1 |
| B3 Colour, material and Themes | Material, Color, Transparency; Theme roles with tags; one-click Themes | Restyle a level with roles so every Theme works on it | A3 (#78), #83, #144, #64, #146 | M3 |
| B4 Light and atmosphere | Lighting service, lighting presets, Looks, time of day, fog, point and spot lights | One level in day, dusk and night versions | #149, #64, #68, #153, #144 | M5 |
| B5 Models and assets | Import glTF, the asset library, pivots, scale, licences of assets | Dress a greybox level with library models | M4 (#17), #36 | M4 |
| B6 Modelling in Studio | Edit mode, extrude, bevel, booleans, UVs | Model one prop and use it in a game | #136, M6 (#19) | M6 |

**S: Script** (Luau)

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| S1 Values and variables | `print`, numbers, strings, `local`, `nil`, `..`; a variable holds one value at a time | A script that prints a countdown message | #7 | M1 |
| S2 Decisions | `if`, `elseif`, comparisons, `==` versus `=`, `and`, `or`, `not`; only `false` and `nil` are falsy | Lava that only hurts when a switch part is red | #7 | M1 |
| S3 Loops and time | `while`, numeric `for`, `task.wait`, why an endless loop without a wait is stopped | A platform that moves back and forth; a blinking light | #7, #118 | M1 |
| S4 Functions | Parameters, `return`, local functions, calling before defining | A `makeStep(position)` function that builds stairs | #7 | M1 |
| S5 Tables | Arrays start at 1, `#`, dictionaries, generalized `for` loops, `table.insert` | A list of checkpoint names printed in order | #7 | M1 |
| S6 Instances from code | `game`, `workspace`, `script.Parent`, `FindFirstChild`, `WaitForChild`, `Instance.new`, `Clone`, `Destroy` | A script that builds a row of 10 coins | #7 | M1 |
| S7 Events | `Touched`, `Connect`, `Disconnect`, debounce, `GetPlayerFromCharacter`, `Humanoid.Died`, `Heartbeat` | Coins that count once (example lesson S7.2) | #7 | M1 |
| S8 Finding bugs | Read an error, Open line, breakpoints, step, watch, print-debugging; Explain on errors | Fix five planted bugs in a broken copy of Parkour | #22, #131, #23 | M1 (debugger after #22) |
| S9 Modules, attributes and tags | `ModuleScript`, `require`, `GetAttribute`, `GetAttributeChangedSignal`, CollectionService | One `Hazard` module that drives every part tagged `Hazard` | A1, A2, A3 (#78) | M2 |
| S10 Motion | `TweenService`, easing, `CFrame` basics, `Model:PivotTo` | A door that opens smoothly when touched | A6, A12 (#79, #80) | M2 |
| S11 Input, camera and rays | `UserInputService`, `ContextActionService`, `workspace.CurrentCamera`, `workspace:Raycast` | A dash on a key and a laser that stops at walls | A7, A19, A20 (#80) | M2 |
| S12 Code that lasts | One job per script, names, `--!strict` types, Problems panel, reading other people's code | Refactor a 300-line script into modules with types | A1 (#78), Luau type checker (no issue) | M3 |

**N: Multiplayer**

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| N1 Server and client | `Script` versus `LocalScript`, what each can see, replication of Instances and properties | Show a message on one screen only, then on every screen | #7; real clients need #15, #156 | M1 (one process), M2 (real) |
| N2 RemoteEvents | `FireServer`, `OnServerEvent`, `FireClient`, `FireAllClients`; what a remote carries | A boost button that asks the server (example lesson N2.3) | #7, A21 (#79) | M2 |
| N3 Never trust the client | Check type, range, rate and ownership on every remote; cooldowns per player | Harden three template remotes against bad data | #7, #15 | M2 |
| N4 Test with friends | Local server with 2 to 8 players, simulated lag, late joiners, leaving mid-round | Fix a round script that breaks when a player leaves | #15, #156, A10 (no issue) | M2 |
| N5 Rounds and teams | Round state on the server, teams, server time, unreliable remotes | A two-team tag game with rounds | A9 (no issue), A16 (no issue), A1 (#78) | M3 |

**U: UI**

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| U1 Screens and labels | `ScreenGui`, `Frame`, `TextLabel`, `UDim2` scale versus offset | A timer and checkpoint counter that fit any window | #8 | M1 |
| U2 Buttons and menus | `TextButton`, `Activated`, opening and closing panels | A pause menu with Resume and Settings | A21 (#79) | M2 |
| U3 Layout and HUD | List and grid layouts, padding, anchor points, safe areas | A shop panel with 6 items that scales | A21 (#79) | M2 |
| U4 UI in the world | `BillboardGui`, `SurfaceGui`, `ProximityPrompt` | A sign that shows a price and a prompt to buy | A15, A21 (no issue for A15) | M3 |
| U5 UI everyone can use | Contrast 4.5:1, 18 px text at 1080p, colour plus shape, gamepad and keyboard reach, reduce-motion settings | Audit and fix a deliberately bad HUD | quality bar F; contrast checker (no issue) | M3 |

**D: Data**

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| D1 Scores and values | Value objects, `leaderstats`, the player list | A coin counter on the player list | A8 (no issue) | M3 |
| D2 Saving | `DataStoreService`, `GetAsync`, `SetAsync`, `UpdateAsync`, `pcall`, `BindToClose` | Coins that come back next session | #82 | M3 (local), M2+ platform store |
| D3 Data you can trust | Retries, session locks, schema versions and migration, never losing a purchase | Migrate a save format from version 1 to 2 with no loss | #82, platform store | M4 |

**A: Art and sound**

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| A1 Sound | `Sound`, `SoundService`, 3D roll-off, sound groups, volume balance | Footsteps, a door creak and quiet ambience in Horror | #81 | M3 |
| A2 Animation | `Animator`, clips, keys and curves in the Animate workspace, markers | A wave emote and an attack with a hit marker | #40, #76, #133 | M6 |
| A3 Visual effects | `ParticleEmitter`, `Beam`, `Trail`, the VFX stack editor | A pickup sparkle and a dash trail | #41, #133 | M6 |
| A4 Game feel | Tweens, camera shake, hit-stop, sound on impact, with a setting to reduce motion | Make a jump and a hit feel good, then add the reduce-motion setting | #79, #81, #41 | M6 |
| A5 Materials and shaders | Material properties, the node editor, sandboxed shaders | A water material and a dissolve effect | #75, #74, #73 | M6 |

**T: 2D**

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| T1 2D worlds | `Part2D`, `Camera2D`, Layers, the 2D view preset | A one-screen 2D level with a goal | #126, #71, #132, A23 (#84) | M3 |
| T2 Tiles and sprites | `TileMap2D`, tile painting, sprite sheets, pixel snapping | A tiled level with an animated sprite | #71, #132, #77 | M3 |
| T3 Platformer feel | Coyote time, jump buffering, variable jump height | Tune the Platformer until a tester says it feels fair | #84, #126 | M3 |

**G: Game design**

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| G1 The core loop | What the player does every 30 seconds; goal, action, feedback, reward | Write and play-test a one-sentence loop on a template | Templates | M1 |
| G2 Fair levels | Teach a move before testing it; difficulty curves; readable hazards (the Dojo idea) | Redesign the first minute of a level for a first-time player | #10, design notes (#83) | M1 |
| G3 Playtesting | Watch silently, note where players stop, change one thing, test again | Two playtest rounds with notes on a friend's run | Play (#11); N4 for remote friends | M1 |
| G4 Multiplayer design | Co-op versus competitive, griefing, team size, catching up, spawn safety | Add spawn protection and a comeback rule to a team game | N5 | M3 |
| G5 Rewards without tricks | Progression, rewards for skill and making, why streaks and loot boxes hurt players | Replace a timer-based reward with one players earn | D1 | M3 |

**P: Ship**

| Unit | Goals | Project | Needs | Ships |
|---|---|---|---|---|
| P1 Performance | Part counts, the profiler, scripts per frame, connection leaks, streaming | Get a 5,000-part level from 40 to 60 fps on Low | #23, #121, #66 | M2 |
| P2 Publishing | Title, description, thumbnail, visibility, content rating, private tests | Publish privately, then to friends | Publish dialog (M2 platform, no studio issue) | M2 |
| P3 Selling fairly | Game passes and products, clear prices, no pay-to-win pressure, no paid random boxes, rules for young players | Design a fair store for your game and review it with the checklist | Platform economy (#20, plateform repo) | M7 |
| P4 Live games | Versions, data migration, changelogs, backups, rolling back | Ship an update without breaking saves | #17, #82 | M4 |
| P5 Tools and plugins | Studio plugins, Ctrl K commands, sharing code safely | A plugin that tags selected parts as hazards | M4 plugins (#17) | M4 |

**Capstones** (one per track group, open brief, automated must-have checks plus optional peer review):

| Capstone | Brief | Must-have checks |
|---|---|---|
| C1 First original game | A game that is not a template copy, finishable in 2 to 5 minutes | Win or finish state reached in a scripted run; at least 3 scripts the learner wrote; not equal to any template scene |
| C2 Multiplayer game | 2 to 8 players, server decides | Every remote passes the bad-data test; late joiner sees current state; nobody gets stuck when a player leaves |
| C3 Saved progress game | Progress survives a restart | Data round-trips through the emulated store; a version 1 save loads in version 2 |
| C4 Polished game | Sound, feel, readable UI, runs well | No contrast failures; 60 fps on the reference scene budget; reduce-motion setting works |
| C5 Published game | On the platform with honest store and page | Published (private or public); store passes the P3 checklist |

### 3.5 What gates a lesson's release

- A lesson declares `requires_api`: the classes and members it uses. Studio reads the engine API index (Litha's
  generator, #141) and hides lessons whose API is missing. Hidden lessons show in the course map as "Coming with
  <milestone>". Assistant's call: no fake previews, no "coming soon" counters.
- A lesson ships in the Studio build only if its CI checks pass (4.8). A unit appears when its first lesson ships.

---

## 4. Lesson format

### 4.1 Files

Lessons live in the Studio repo under `learn/`, next to `templates/`, and install with Studio.

```
learn/
  course.toml                       tracks, units, order, concept ids
  concepts.toml                     concept id, name, one-line definition (also the glossary)
  F/F1-play/unit.toml
  B/B1-parts/unit.toml
  B/B1-parts/B1.3-bridge-the-gap/
    lesson.toml                     steps, checks, hints, feedback, metadata
    start.toml                      where the lesson project comes from (template + patch, or scene)
    start/                          optional files the patch adds (scenes, scripts)
    solution/                       per-step solution patches and the final scripts
    checks/*.luau                   optional script tests
    media/                          optional images (PNG, at most 200 KB each), no text in images
    l10n/<locale>.toml              translations keyed by step id
```

Why TOML: `tutorial.toml` and `template.toml` are TOML already; scenes are RON. TOML keeps prose readable in a diff,
one key per line. Scripts are plain `.luau` files so the type checker and diff tools work on them. Assistant's call.

### 4.2 `lesson.toml` fields

Top level:

| Field | Meaning |
|---|---|
| `id`, `version` | Stable id (`S7.2`) and an integer that rises on each content change |
| `kind` | `lesson` (course), `tour` (a template's tutorial), `project` (unit project), `capstone` |
| `title`, `minutes` | Title and honest time estimate measured with testers |
| `teaches`, `needs` | Concept ids taught and assumed |
| `requires_api` | Classes and members used; gates visibility (3.5) |
| `reading_level` | `starter`, `core` or `advanced`; sets the CI limits in 7.1 |
| `litha` | Modes allowed during this lesson; default `["explain", "hint", "show_me"]` |
| `goal` | One sentence: what the learner will have made |

Each `[[step]]`:

| Field | Meaning |
|---|---|
| `id`, `kind` | `do` (act in the editor), `predict` (choose an answer before running), `parsons` (order given lines), `quiz` (check understanding), `read` (short text, a Next button, at most one per lesson), `make` (open goal) |
| `text` | At most 50 words; placeholders `{key:duplicate}` resolve to the learner's keymap, `{path:...}` to a clickable Explorer path |
| `subgoal` | Optional label shown as a code comment or a step tag ("Find the player") |
| `highlight` | UI target to ring (4.3); several allowed |
| `check` | What passes the step (4.4); `all` and `any` combine checks |
| `hints` | Ladder of up to four authored hints (4.5) |
| `feedback` | Messages for known wrong states (4.6) |
| `solution` | Patch that completes the step (4.7) |
| `checkpoint` | `true` takes a snapshot on entering the step, so "Reset step" works |
| `more` | Optional deeper text behind a "Why?" link, not counted in the word limit |

### 4.3 Highlight targets

Targets name things, not screen positions, so a layout change does not break lessons (GDevelop's CSS selectors do
break). Studio keeps one registry of named controls; each control registers its id when drawn.

| Target | Example |
|---|---|
| Named control | `toolbar.play`, `toolbar.stop`, `panel.output`, `panel.properties`, `palette`, `menu.insert.part` |
| Explorer row | `explorer:Workspace.Course.Gap1` |
| Properties row | `properties:Workspace.Course.Gap1#Anchored` |
| Viewport object | `viewport:Workspace.Course.Gap1` (ring around the object on screen; arrow at the edge if off screen) |
| Selected object's property | `properties:selection#Anchored` (the row for whatever is selected) |
| Script lines | `script:ServerScriptService.Coins#L4-L7` |
| Game settings knob | `settings:JumpBoost` |
| Litha | `litha.hint` |

Studio draws the 2 px accent coach ring from the UI doc 5.8 and never blocks input. If a target does not exist
(the learner renamed it), the step shows "Can't find Gap1. Reset step?" instead of a ring in the wrong place.

### 4.4 Checks

One step can combine several checks with `all = [...]` or `any = [...]`.

| Kind | Fields | Passes when | Runs |
|---|---|---|---|
| Studio event | `event` = one of `play_started`, `play_stopped`, `selected`, `property_changed`, `setting_changed`, `look_applied`, `inserted`, `script_edited`, `touched`, `remote_fired`, `printed`, `renamed_game`, `published` (the templates README list) | The event fires with matching fields | On the event |
| World query | `world` = selector (`path`, `under` + `class`, `tagged`, `named`, `in_box`) plus `count`, `props` (exact, `min`, `max`, `near`) | The edit world matches | After each `History` commit that touches the selector, debounced 250 ms |
| Script test | `test` = a file in `checks/` | The test calls `Learn.pass()` and no `Learn.fail()` or error happens | When the learner presses Check or stops Play; headless on a snapshot |
| Output match | `output` = text with `*` wildcards, `source` (Server, Client), `times` (exact or range), `in_last_play` | Output rows of the last Play match | On Stop |
| Source shape | `source` = script path plus `uses` (constructs such as `for`, `if`, `Connect`, `function`) | The script's syntax tree contains them | On save. Only in steps that teach a construct, never as the only check |
| Answer | `answer` = option id for `predict` and `quiz` | The learner picks it | On choice. A wrong pick shows that option's explanation and lets them try again |
| Parsons | `order` = line ids, with `alternatives` for valid swaps | The order matches or the assembled script passes the step's test | On Check |

**Script tests** run in a headless runtime, the same one `metallic-player --headless` uses (plan Task 8), on a copy
of the learner's edit world. The runtime keeps the sandbox: time budget, memory cap, no files or network. Tests get
a `Learn` library:

```lua
Learn.spawnCharacter(name)         -- a test player with a character at the SpawnLocation
Learn.touch(part, character)       -- fires Touched as if the character walked into the part
Learn.fireServer(player, remoteName, ...)  -- a client message from that test player
Learn.step(seconds)                -- advances fixed 60 Hz frames; task.wait inside game scripts follows it
Learn.output()                     -- Output rows so far, as {source, text}
Learn.expect(value, label)         -- :toBe(x), :toBeNear(x, tolerance), :toExist(), :toBeNil()
Learn.pass(message) / Learn.fail(message)
```

Rules (assistant's calls):
- Fixed timestep and a fixed `Random` seed, so a test gives the same result every run. `task.wait`, `task.delay`,
  `time()` and `os.clock()` all follow the simulated clock inside a test, so cooldown code behaves as in a real game.
- A test stops after 30 simulated seconds or 3 wall-clock seconds, and reports "Your game took too long to answer"
  instead of hanging Studio.
- A failing test returns the label of the failed expectation. Authors write labels for learners: "The coin should
  disappear after the first touch", not "expected nil, got Instance".
- Studio shows "Checking..." for at most 2 seconds before showing the result.

### 4.5 Hints, solution and Litha

Authored hints follow Litha's Hint ladder (Litha doc section 3):

| Rung | Authored hint | Litha, when on |
|---|---|---|
| 1 Question | A question that points at the idea ("What is `hit` when your foot touches the coin?") | Asks the same question about the learner's own code |
| 2 Where | Where to look ("Look at line 6 and the Output after you touch the coin") | Names the line in the learner's script |
| 3 Highlight | Rings the exact line, property or knob | Uses `highlight` on the learner's version |
| 4 Pseudo-code | Plain-words steps ("If there is no player, stop. Otherwise...") | Pseudo-code fitted to the learner's names |

- The hint button shows rung 1 first, then the next rung on each press. Rungs used are stored on the PC only and
  never shown as a score.
- **Show solution** is always present. The first press asks once: "Try one more hint first?" with two buttons. After
  that it applies or shows the solution with no penalty and no message about it. Applying is one undo step.
- When Litha is on, the lesson sends Litha the step text, the authored hints and the solution as hidden context, so
  Litha's hint stays on the authored path and does not invent a different fix. Litha's eval suite (Litha doc 11)
  adds a case per lesson: Hint mode must not contain the step's solution.
- Litha modes in lessons: Explain, Hint and Show me only, because Do it would pass the step for the learner. A
  `make` step may allow Do it with me so Litha can scaffold and hand back `-- YOUR TURN:` pieces. Outside lessons,
  Litha's own defaults apply. Assistant's call.
- Without Litha (offline, switched off, under 13), lessons work the same with the authored hints.

### 4.6 Feedback for known mistakes

A `[[step.feedback]]` entry fires when a check fails and its `when` matches. It shows one sentence in the step.

| `when` | Example message |
|---|---|
| `output_contains = "attempt to index nil"` | "Something you asked for was not there. Check it before you use it: `if player then`." |
| `source_has = "for i = 0"` | "Lists in Luau start at 1, not 0." |
| `syntax_error = "="` near `if` | "In an `if`, compare with `==`. One `=` stores a value." |
| `world` selector: the bridge part is not anchored | "Your bridge fell. Set Anchored so it stays put." |

Each track keeps a shared list of these in `learn/<track>/feedback.toml`, so a mistake met in S2 gets the same
words in N3. Litha's eval cases reuse the list.

### 4.7 Solution and reset

- `start.toml` builds the lesson project: a template id and version, or a scene from `start/`, plus a patch.
- A patch is a list of the same command kinds as Litha's `propose_change` (Litha doc 5.3): `SetProperty`,
  `SetAttribute`, `InsertInstance`, `DeleteInstance`, `Reparent`, `Rename`, `EditScript`, `CreateScript`. One
  format serves lesson setup, solutions and Litha proposals, and every patch is one undo step.
- `solution/<step>.toml` completes one step from the state a correct learner would be in. `solution/final/` holds the
  finished scripts.
- **Reset step** restores the snapshot taken when the step began. **Reset lesson** rebuilds the start. Before either
  reset, Studio saves the learner's current state to `Learn/<lesson>/attempts/<time>/`, keeping the last 5, so a
  reset never destroys work. Rustlings' `r` resets an exercise the same way (Sourced).

### 4.8 Versioning and CI

- `version` rises on every content change. A learner mid-lesson continues if their current step id still exists;
  otherwise Studio says "This lesson changed. Your project is kept." and starts the new version at the first step
  whose check does not pass yet.
- Completed lessons stay completed across versions.
- Template tours carry `template_version`; a tour runs only on projects from that template version or newer.
- **CI per lesson** (headless, on every engine and every `learn/` change):
  1. The validator accepts `lesson.toml` (schema, ids, word limits, reading level, targets in the registry).
  2. Applying all step solutions to the start passes every check, step by step.
  3. The start state fails the first `do` step's check, so the step is not passed by doing nothing.
  4. Every `requires_api` member exists in the API index for this build.
  5. Every translation either matches the English source hash or is marked stale (7.2).

---

## 5. Lesson runner in Studio

### 5.1 Where it sits

| Place | What it shows | Source |
|---|---|---|
| Start screen, **Learn** view next to New and Recent | Track map, "Next for you" (3 cards), units with time estimates, "Continue S7.2" when a lesson is open | UI doc 5.8 adds the view |
| **Learn** dock tab | The current lesson: step list with checks, the active step's text, Hint, Show solution, Reset, "Why?" | Assistant's call: during a lesson the right dock splits, Learn above (min 180 px), Properties and Litha tabs below, so step text and Properties are visible together |
| **Guide strip**, bottom left of the viewport | Collapsed form: "B1.3 · step 3 of 6", one sentence, the next action. Clicking it opens the Learn tab | UI doc 5.8 and 5.5 overlays |
| **Ctrl K** "Learn" group | Lessons and units by name and length | UI doc 5.4 |
| **Help > Learn**, and "Learn more" in Properties tooltips | Opens the lesson or unit that teaches that property | UI doc 5.3 |
| Output error rows, **Explain** | Without Litha, opens the lesson that teaches the error, when one exists | UI doc 5.9 |

- A lesson opens its own project in `Documents\Metallic\Learn\<lesson id>`, never in one of the learner's games. The
  status bar shows "Lesson · S7.2". A `make` or project step offers **Keep as my game**, which copies the project to
  `Documents\Metallic\Games`.
- No pop-ups and no stencil overlays. The ring and the strip never block input (UI doc 5.8).
- Passed steps tick with a short check mark and a sound only if UI sounds are on. Reduced motion turns the
  animation off (UI doc 5.12).

### 5.2 Checks engine

- Runs in Studio, not on a server.
- Event checks subscribe to the Studio event bus that #83's tutorial runner needs anyway.
- World checks run against the edit world after `History` commits, debounced, and only for selectors the current
  step uses. A query over 50k Instances must finish in 2 ms; the selector index uses paths and tags, not a scan.
- Script tests run on a worker thread in the headless runtime on a world snapshot. One test runs at a time; a new
  Check cancels the running one.
- Output checks read the `OutputSink` records kept from the last Play.
- During Play, `do` steps that need the edit world wait until Stop and say so in the step: "Stop to check".

### 5.3 Progress storage

Local first:
- `%APPDATA%\Metallic\learn\progress.toml`, human-readable. Per lesson: status (`not_started`, `in_progress`,
  `done`), version done, current step, steps passed, time started and finished, and which badge projects passed.
  Hint rungs used are kept for the learner's own view only ("You used hints on 3 steps").
- Lesson projects stay in `Documents\Metallic\Learn`. Deleting that folder resets nothing in progress; Studio
  rebuilds a lesson's project from its start if it is missing.
- Export and import of progress as a file, for a learner moving PCs before sync exists.

Platform sync later (M7, assistant's call):
- Opt-in, through the signed-in account. Merge rule: a step passed anywhere stays passed; the newest timestamp wins
  for the current step.
- Off for under-13 accounts until the platform has verifiable parental consent that covers it (7.4).
- What syncs: lesson ids, statuses, timestamps, badge ids with the project id. Never project contents, hint counts
  or answers.

### 5.4 Offline behaviour

- The course, its media and its checks ship in the Studio install. Assistant's call: a size budget of 30 MB for all
  of `learn/` (text is tiny; images are capped at 200 KB each and used only where a picture teaches).
- Everything works offline except Litha, publishing steps and sync. Publishing steps fall back to "Save and share"
  zips, as the templates doc 4.1 does before M2.
- Course updates arrive with Studio updates (Velopack, spec 7 1C). Assistant's call: no separate content download
  in v1; one version of Studio means one version of the course and its API.

---

## 6. Assessment, credentials and the path to publishing

### 6.1 No grades

There are no scores, no timed tests and no pass marks on quizzes. Quiz and predict steps let the learner try again
and explain each wrong option. Duolingo-style hearts or lives do not exist.

### 6.2 Skill badges tied to made things

- A **skill badge** is earned when a unit's project passes its checks in a project the learner made. The badge links
  to that project (a local file now, a published game later). Example: "Events: coins that count once, made
  2026-11-12".
- Badges count making, as the templates doc 4.4 set: First Play, Made it mine, First script change, First publish
  stay as achievements; skill badges add one per unit.
- **Test out**: each unit has a "Skip ahead" option that opens the unit project directly. Passing it gives the badge
  and marks the lessons done. Experienced creators skip, and the expertise reversal effect says long examples slow
  them down (2.2).
- **Track badges** need the track's units and its capstone.
- Badges are private by default. From M7 the learner may show them on their profile (not for under-13 accounts
  without consent).
- Assistant's call: no exams and no paid certificates. If the platform later wants a "Metallic Certified Creator"
  credential, it should be a reviewed capstone, not a quiz.

### 6.3 Spaced review without streaks

- A lesson may open with one **warm-up** question from a unit finished 2 or more days ago. It is skippable and never
  graded.
- The Learn home has a **Review** card with three short questions on older concepts. It appears only when the
  learner opens Learn. No notifications, no reminders, no counts of days.
- An optional weekly goal "make something this week" is the only goal, as the templates doc allows. Missing a week
  shows nothing.

### 6.4 Creator path to publishing

1. F1 to F4 inside Parkour: play, change, script one line, share a zip.
2. A template of the learner's choice through its tour, then three "Next steps" mods (templates doc 4.4).
3. Script, Build and Design units with their projects.
4. Capstone C1: an original game. The templates doc's rule applies: a public publish of an unchanged template is
   refused; private publishing is always allowed.
5. P2 Publishing: private publish, then friends.
6. P3 Selling fairly before enabling paid items. Assistant's call, owner to confirm: the platform asks a creator to
   finish P3 (about 10 minutes) before their first paid product goes live. Metallic will have young players, and
   P3 is where the platform's rules on prices and random rewards are taught.
7. C5: a published game with an honest store.

---

## 7. Accessibility and age

### 7.1 Reading level

| Level | Used by | Limits checked in CI |
|---|---|---|
| `starter` | F, B1 to B3, S1 to S7, G1 to G3, U1 | Step text at most 50 words; sentences at most 15 words; Flesch-Kincaid grade 5 or lower (about age 10 to 11) |
| `core` | Most other units | At most 50 words; sentences at most 20 words; grade 7 or lower (about age 12, Litha's beginner target) |
| `advanced` | D3, N5, P1, P3 to P5, S12, A5 | At most 60 words; grade 8 or lower, inside WCAG 3.1.5's lower secondary level |

- WCAG 3.1.5 (AAA) asks for extra help when text needs more than lower secondary reading ability (Search only: W3C
  Understanding 3.1.5). Advanced units stay within it.
- The grade formula runs on English text with code spans removed. Other languages get a human review instead.
- Terms link to the glossary in `concepts.toml` on first use in a lesson ("event: something that happens, like a
  touch, that your code can react to").
- One idea per step. Code identifiers stay in English in every language, because the API is English.

### 7.2 Localisation plan

- English first. All text is in lesson files; no text in images or in Luau strings the learner must not change.
- Translations live in `l10n/<locale>.toml`, keyed by step id and field, each with the hash of the English source.
  When English changes, CI marks the translation stale and Studio shows that step in English until it is fixed.
  Assistant's call: no mixing of a stale translation with new steps.
- Comments and strings inside starter scripts are translated; identifiers and API names are not.
- Plurals and numbers through Fluent-style messages in Studio's own UI (the `fluent` crates are Apache-2.0 OR MIT,
  Search only); lessons need few plurals and keep plain strings.
- Order (judgment, to confirm with platform data once it exists): Spanish, Portuguese (Brazil), French, German, then
  Indonesian, Japanese, Korean, Simplified Chinese and Russian. Machine draft, then review by a fluent reviewer
  before release. A translation counts as shipped only after a first-time tester in that language finishes F1 to F3.
- Right-to-left languages wait for egui RTL support; not verified.

### 7.3 Screen readers, keyboard and reading support

- The Learn tab uses AccessKit roles (UI doc 5.12): the step list is a list, the active step a heading plus text,
  results a live region ("Step 3 passed", "Not yet: your bridge fell").
- Every highlight target has a spoken name. "Show me" for a viewport object also selects it, so a keyboard user
  can change it through Properties without the 3D view.
- Parsons steps work with the keyboard: arrow keys pick a line, Alt+Up and Alt+Down move it.
- Predict and quiz options are buttons with full text, never colour only.
- **Read aloud** button per step using the operating system's text-to-speech (assistant's call; crate choice is a
  build-time decision, permissive licence only).
- Text size follows the interface size setting (90% to 150%). No special dyslexia font: OpenDyslexic showed no gain
  in reading rate or accuracy (Search only: Wery and Diliberto 2016). Line spacing and size settings help more
  evenly.
- Lesson videos, if any, have captions and a text alternative. Assistant's call: lessons use still images, not video,
  in v1.
- Lessons never require fast reflexes to pass a check. A check that needs reaching a checkpoint offers "Teleport me
  there" after two tries.

### 7.4 Age policy

Consistent with the Litha doc 9.2:
- The course itself needs no account and sends nothing off the PC. Studio does not know a learner's age offline, so
  local mode treats every user as possibly a child: no telemetry by default, no social features.
- **Litha in lessons:** 18+ from M4, teens from the M7 teen launch, off for under-13 until the platform has
  verifiable parental consent and a legal review. Lessons work fully without it.
- **Under 13 with an account:** progress sync, public badges, peer review of capstones and sharing lesson projects
  stay off until parental consent covers them. Private badges and local progress work.
- No lesson asks for personal information. No step asks a learner to type their name, age, school or location.
- Content stays general audience: no gore, hazards are lava and spikes, Horror lessons have a "less scary" setting.
- Opt-in telemetry for improving lessons (step drop-off, failed checks, hint rungs) is off by default, and never
  collected for under-13 accounts without consent.

---

## 8. Example lesson files

Three full lessons. Paths, classes and members follow `docs/templates/README.md` section 6 (the M1 API plus the
groups each lesson lists).

### 8.1 B1.3 Bridge the gap (starter, world queries and events)

`learn/B/B1-parts/B1.3-bridge-the-gap/lesson.toml`:

```toml
id = "B1.3"
version = 1
kind = "lesson"
title = "Bridge the gap"
minutes = 6
goal = "You will build a bridge so every player can cross the second gap."
teaches = ["duplicate", "move-tool", "anchored"]
needs = ["select", "play-stop"]
requires_api = ["Part.Anchored", "Part.Size", "Checkpoint"]
reading_level = "starter"
litha = ["explain", "hint", "show_me"]

[[step]]
id = "try"
kind = "do"
text = "Press Play and try to reach Checkpoint 2. The second gap is too wide. Press Stop when you have seen it."
highlight = ["toolbar.play"]
check = { all = [{ event = "play_started" }, { event = "play_stopped" }] }
hints = [
  "Where is the Play button?",
  "Look at the top of the window, in the middle.",
  { highlight = "toolbar.play" },
]

[[step]]
id = "why"
kind = "predict"
text = "Why can't you make the jump?"
options = [
  { id = "wide", text = "The gap is wider than a jump can reach.", explain = "Yes. A running jump covers about 12 studs. This gap is 20." },
  { id = "slow", text = "The character is too slow.", explain = "Speed helps a little, but no run-up crosses 20 studs." },
  { id = "bug", text = "It is a bug in the game.", explain = "The level is built this way on purpose, so you can fix it." },
]
check = { answer = "wide" }

[[step]]
id = "select"
kind = "do"
checkpoint = true
text = "Click the platform just before the gap, {path:Workspace.Course.Ledge2}. Its name appears in Properties."
highlight = ["viewport:Workspace.Course.Ledge2", "explorer:Workspace.Course.Ledge2"]
check = { event = "selected", path = "Workspace.Course.Ledge2" }
hints = [
  "Which platform is right before the wide gap?",
  "It is called Ledge2. You can also click it in the Explorer.",
  { highlight = "explorer:Workspace.Course.Ledge2" },
]
solution = "solution/select.toml"

[[step]]
id = "duplicate"
kind = "do"
checkpoint = true
subgoal = "Make a copy"
text = "Press {key:duplicate} to copy it. The copy sits in the same place, so you will not see it move yet."
check = { event = "inserted", class = "Part", under = "Workspace.Course" }
hints = [
  "How do you make a second one of something?",
  "Look in the Edit menu for Duplicate.",
  { highlight = "menu.edit.duplicate" },
]
solution = "solution/duplicate.toml"

[[step]]
id = "place"
kind = "do"
checkpoint = true
subgoal = "Put it in the gap"
text = "Use the Move tool ({key:tool_move}) to drag the copy into the middle of the gap. Keep its top level with the ledges."
highlight = ["viewport:Workspace.Course.Gap2"]
check = { world = { under = "Workspace.Course", class = "Part", in_box = { center = [0, 20, 140], size = [16, 3, 12] } }, count = { min = 1 }, props = { "Size.Y" = { min = 0.5 } } }
hints = [
  "Where should the copy end up?",
  "Drag the arrow that points along the gap. Watch the grey marker in the middle of the gap.",
  { highlight = "viewport:Workspace.Course.Gap2" },
  "Pick the Move tool. Drag the copy's long arrow until it is between both ledges. Use the up arrow to match their height.",
]
solution = "solution/place.toml"

[[step.feedback]]
when = { world = { under = "Workspace.Course", class = "Part", in_box = { center = [0, 14, 140], size = [16, 6, 12] } }, count = { min = 1 } }
text = "Your bridge is in the gap but too low. Players would fall onto it. Lift it level with the ledges."

[[step]]
id = "anchor"
kind = "do"
text = "Tick Anchored on your bridge in Properties, so it does not fall when the game starts."
highlight = ["properties:selection#Anchored"]
check = { world = { under = "Workspace.Course", class = "Part", in_box = { center = [0, 20, 140], size = [16, 3, 12] } }, count = { min = 1 }, props = { Anchored = true } }
more = "Parts that are not anchored follow physics. They fall, roll and get pushed. Floors and walls are almost always anchored."
hints = [
  "What happens to an unanchored part when Play starts?",
  "Look for Anchored in the Properties panel.",
  { highlight = "properties:selection#Anchored" },
]
solution = "solution/anchor.toml"

[[step]]
id = "cross"
kind = "do"
text = "Press Play and cross your bridge to Checkpoint 2."
highlight = ["toolbar.play"]
check = { event = "touched", path = "Workspace.Checkpoints.Checkpoint2" }
hints = [
  "Did your bridge stay where you put it?",
  "If it fell, Stop and check Anchored again.",
]
teleport_after_tries = 2
```

`solution/place.toml`:

```toml
[[command]]
kind = "SetProperty"
path = "Workspace.Course.Ledge2 (2)"
property = "Position"
value = { Vector3 = [0, 20, 140] }
```

### 8.2 S7.2 Coins that count (starter, PRIMM, Parsons, script test, Output check)

`learn/S/S7-events/S7.2-coins-that-count/lesson.toml`:

```toml
id = "S7.2"
version = 1
kind = "lesson"
title = "Coins that count"
minutes = 12
goal = "Each coin counts once, for the player who touched it."
teaches = ["touched-event", "get-player-from-character", "debounce"]
needs = ["events-connect", "if", "nil", "generalized-for"]
requires_api = ["BasePart.Touched", "RBXScriptSignal.Connect", "Players.GetPlayerFromCharacter", "Instance.Destroy", "Instance.GetChildren"]
reading_level = "starter"
litha = ["explain", "hint", "show_me"]

[[step]]
id = "predict"
kind = "predict"
text = "Line 8 prints something when a coin is touched. What will it print when you walk into a coin?"
highlight = ["script:ServerScriptService.Coins#L7-L9"]
options = [
  { id = "part", text = "The name of a body part, like LeftFoot", explain = "Yes. hit is the part that touched the coin." },
  { id = "player", text = "Your player name", explain = "Not quite. Touched gives the part that touched, not the player." },
  { id = "coin", text = "Coin", explain = "The coin is the part being touched. hit is the other part." },
]
check = { answer = "part" }

[[step]]
id = "run"
kind = "do"
text = "Press Play, walk into one coin, then Stop. Look at the Output."
highlight = ["toolbar.play", "panel.output"]
check = { output = "*", source = "Server", times = { min = 2 }, in_last_play = true }
hints = ["Did you touch a coin before you pressed Stop?"]

[[step]]
id = "investigate"
kind = "quiz"
text = "One touch printed several lines. Why?"
options = [
  { id = "parts", text = "Each body part that touches the coin fires Touched", explain = "Yes. Feet, legs and the root part can all touch it." },
  { id = "broken", text = "The script is broken", explain = "It works as written. Touched fires for every touching part." },
]
check = { answer = "parts" }

[[step]]
id = "find-player"
kind = "parsons"
subgoal = "Find the player"
text = "Put these lines in order so the script prints the player's name. One line is not needed."
lines = [
  { id = "a", code = "local character = hit.Parent" },
  { id = "b", code = "local player = Players:GetPlayerFromCharacter(character)" },
  { id = "c", code = "if player then" },
  { id = "d", code = "    print(player.Name .. \" got a coin\")" },
  { id = "e", code = "end" },
  { id = "x", code = "local player = hit.Name", distractor = true },
]
check = { order = ["a", "b", "c", "d", "e"] }
target = "script:ServerScriptService.Coins#L7-L9"
hints = [
  "What is the parent of a foot?",
  "A foot sits inside the character Model. Players can turn a character into a player.",
  { highlight = "script:ServerScriptService.Coins#L7-L9" },
  "Get the character from hit. Ask Players for its player. Only print if there is one.",
]

[[step.feedback]]
when = { parsons_used = "x" }
text = "hit.Name is a body part's name, like LeftFoot. Find the character first."

[[step]]
id = "once"
kind = "do"
checkpoint = true
subgoal = "Count it once"
text = "Make each coin count once: before printing, destroy the coin. Then add 1 to coinsCollected and print it."
highlight = ["script:ServerScriptService.Coins#L8"]
check = { test = "checks/once.luau" }
hints = [
  "If the coin is gone, can it be touched again?",
  "Put coin:Destroy() inside the if player then block, before the print.",
  { highlight = "script:ServerScriptService.Coins#L8" },
  "Inside the if: destroy the coin. Add 1 to coinsCollected. Print the player's name and the count.",
]
solution = "solution/once.toml"

[[step.feedback]]
when = { output_contains = "attempt to index nil with 'Name'" }
text = "player is nil when a part that is not a character touches the coin. Keep the print inside if player then."

[[step.feedback]]
when = { syntax_error = true, source_has = ".Touched.Connect" }
text = "Signals use a colon: coin.Touched:Connect(...)."

[[step]]
id = "all-coins"
kind = "do"
subgoal = "Do it for every coin"
text = "The script only handles Coin1. Wrap your code in a for loop over Coins:GetChildren() so every coin works."
check = { all = [{ test = "checks/all.luau" }, { source = "ServerScriptService.Coins", uses = ["for"] }] }
hints = [
  "How do you run the same code for each item in a list?",
  "Look at lesson S5.3, Loops over lists.",
  { highlight = "script:ServerScriptService.Coins#L6" },
  "for each coin in Coins:GetChildren(): connect its Touched to your code.",
]
solution = "solution/all.toml"

[[step.feedback]]
when = { source_has = "for i = 0" }
text = "Lists in Luau start at 1. A for loop over GetChildren() needs no number at all."
```

`start/ServerScriptService/Coins.server.luau` (the script the lesson starts with):

```lua
-- Prints who touched Coin1. You will make every coin count once.
local Players = game:GetService("Players")
local Coins = workspace:WaitForChild("Coins")
local coinsCollected = 0

local coin = Coins:WaitForChild("Coin1")
coin.Touched:Connect(function(hit)
    print(hit.Name)
end)
```

`solution/final/Coins.server.luau`:

```lua
-- Each coin counts once, for the player who touched it.
local Players = game:GetService("Players")
local Coins = workspace:WaitForChild("Coins")
local coinsCollected = 0

for _, coin in Coins:GetChildren() do
    coin.Touched:Connect(function(hit)
        -- Find the player
        local character = hit.Parent
        local player = Players:GetPlayerFromCharacter(character)
        if player then
            -- Count it once
            coin:Destroy()
            coinsCollected += 1
            print(player.Name .. " got a coin. Total: " .. coinsCollected)
        end
    end)
end
```

`checks/once.luau`:

```lua
-- Touches Coin1 twice with the same character; the coin must count once.
local Players = game:GetService("Players")
local character = Learn.spawnCharacter("Tester")
local coin = workspace.Coins.Coin1

Learn.touch(coin, character)
Learn.step(0.2)
Learn.touch(coin, character)  -- a second touch must do nothing
Learn.step(0.2)

Learn.expect(workspace.Coins:FindFirstChild("Coin1"), "Coin1 should be gone after the first touch"):toBeNil()

local count = 0
for _, row in Learn.output() do
    if string.find(row.text, "Tester got a coin", 1, true) then
        count += 1
    end
end
Learn.expect(count, "The message should print once, not " .. count .. " times"):toBe(1)
Learn.pass("Coin1 counts once")
```

### 8.3 N2.3 Ask the server (core, RemoteEvents and server checks)

Starts from a small scene: a `ScreenGui` with a `TextButton` "Boost", a `RemoteEvent`
`ReplicatedStorage.Remotes.RequestBoost`, a client script that sends a jump power with the request, and a server
script that trusts it.

`learn/N/N2-remote-events/N2.3-ask-the-server/lesson.toml`:

```toml
id = "N2.3"
version = 1
kind = "lesson"
title = "Ask the server"
minutes = 12
goal = "The boost button works for players and does nothing useful for a cheater."
teaches = ["server-decides", "remote-validation", "per-player-cooldown"]
needs = ["remote-events", "tables-dictionaries", "if"]
requires_api = ["RemoteEvent.FireServer", "RemoteEvent.OnServerEvent", "TextButton.Activated", "Humanoid.JumpPower", "Instance.FindFirstChildOfClass", "task.delay", "os.clock"]
reading_level = "core"
litha = ["explain", "hint", "show_me"]

[[step]]
id = "try"
kind = "do"
text = "Press Play and click Boost. Jump. Then Stop."
highlight = ["toolbar.play"]
check = { all = [{ event = "remote_fired", name = "RequestBoost" }, { event = "play_stopped" }] }

[[step]]
id = "spot"
kind = "predict"
text = "Read line 9 of BoostController. The client sends the jump power. What could a cheater send instead?"
highlight = ["script:StarterPlayer.StarterPlayerScripts.BoostController#L9"]
options = [
  { id = "any", text = "Any number they like, such as 5000", explain = "Yes. A cheater can change what their own client sends." },
  { id = "same", text = "Only 75, because that is in the script", explain = "The script runs on the player's PC. A cheater can change it or send their own message." },
]
check = { answer = "any" }
more = "Code in a LocalScript runs on the player's computer. Anything it sends to the server may be changed. The server must decide."

[[step]]
id = "server-decides"
kind = "do"
checkpoint = true
subgoal = "The server picks the number"
text = "In BoostService, stop using the power the client sent. Set JumpPower to BOOST_POWER instead."
highlight = ["script:ServerScriptService.BoostService#L12-L16"]
check = { test = "checks/server_decides.luau" }
hints = [
  "Which script should choose the jump power?",
  "Line 14 uses requestedPower, which came from the client.",
  { highlight = "script:ServerScriptService.BoostService#L14" },
  "Replace requestedPower with BOOST_POWER. You can delete the requestedPower parameter.",
]
solution = "solution/server_decides.toml"

[[step]]
id = "cooldown"
kind = "do"
checkpoint = true
subgoal = "One boost every 10 seconds"
text = "A cheater can also click a thousand times. Keep the last boost time per player in lastBoost, and ignore requests less than 10 seconds apart."
highlight = ["script:ServerScriptService.BoostService#L5"]
check = { test = "checks/cooldown.luau" }
hints = [
  "How does the server know when this player last boosted?",
  "lastBoost is a table. Use the player as the key and os.clock() as the value.",
  { highlight = "script:ServerScriptService.BoostService#L5" },
  "At the start of the handler: if lastBoost[player] and now - lastBoost[player] < COOLDOWN, return. Otherwise store now and boost.",
]
solution = "solution/cooldown.toml"

[[step.feedback]]
when = { output_contains = "attempt to perform arithmetic on nil" }
text = "The first time, lastBoost[player] is nil. Check it exists before you subtract."

[[step]]
id = "bad-data"
kind = "do"
text = "Check that junk does no harm. Press Check: it sends a string, nothing and a table to RequestBoost."
check = { test = "checks/bad_data.luau" }
hints = [
  "Does your handler still use anything the client sent?",
  "After the last step, the server should ignore every argument.",
]

[[step]]
id = "make"
kind = "make"
text = "Your turn: add a speed boost the same way. The server picks the speed and the cooldown."
check = { test = "checks/speed.luau" }
litha = ["explain", "hint", "show_me", "do_with_me"]
hints = [
  "Which parts of the jump boost can you copy?",
  "You need a new RemoteEvent, a button and a handler with its own cooldown table.",
]
```

`checks/cooldown.luau`:

```lua
-- Two quick requests boost once; a request after the cooldown boosts again.
local Tester = Learn.spawnCharacter("Tester")
local player = game:GetService("Players").Tester
local humanoid = Tester:FindFirstChildOfClass("Humanoid")

Learn.fireServer(player, "RequestBoost")
Learn.step(0.1)
Learn.expect(humanoid.JumpPower, "The first request should boost"):toBe(75)

Learn.step(5.5)   -- the boost lasts 5 s, so JumpPower is back to 50
Learn.fireServer(player, "RequestBoost")
Learn.step(0.1)
Learn.expect(humanoid.JumpPower, "A second request 5.6 s later should be ignored"):toBe(50)

Learn.step(5)
Learn.fireServer(player, "RequestBoost")
Learn.step(0.1)
Learn.expect(humanoid.JumpPower, "A request after 10 s should boost again"):toBe(75)
Learn.pass("Cooldown works")
```

`checks/bad_data.luau`:

```lua
-- Junk arguments must not raise errors or change JumpPower beyond the server's value.
local Tester = Learn.spawnCharacter("Tester")
local player = game:GetService("Players").Tester
local humanoid = Tester:FindFirstChildOfClass("Humanoid")

Learn.fireServer(player, "RequestBoost", "lots")
Learn.fireServer(player, "RequestBoost", nil)
Learn.fireServer(player, "RequestBoost", { JumpPower = 9000 })
Learn.step(0.1)

for _, row in Learn.output() do
    if row.source == "Server" and string.find(row.text, "error", 1, true) then
        Learn.fail("The server printed an error for junk data: " .. row.text)
    end
end
Learn.expect(humanoid.JumpPower <= 75, "JumpPower should never go above 75"):toBe(true)
Learn.pass("Junk data does no harm")
```

`solution/final/BoostService.server.luau`:

```lua
-- Gives a player a short jump boost when they ask, at most once every 10 seconds.
local Players = game:GetService("Players")
local RequestBoost = game:GetService("ReplicatedStorage").Remotes.RequestBoost

local BOOST_POWER = 75
local BOOST_SECONDS = 5
local COOLDOWN = 10
local lastBoost = {}  -- player -> os.clock() of their last boost

RequestBoost.OnServerEvent:Connect(function(player)
    -- One boost every 10 seconds
    local now = os.clock()
    if lastBoost[player] and now - lastBoost[player] < COOLDOWN then
        return
    end
    lastBoost[player] = now

    -- The server picks the number
    local humanoid = player.Character and player.Character:FindFirstChildOfClass("Humanoid")
    if not humanoid then
        return
    end
    humanoid.JumpPower = BOOST_POWER
    task.delay(BOOST_SECONDS, function()
        if humanoid.Parent then
            humanoid.JumpPower = 50
        end
    end)
end)
```

Note: `FindFirstChildOfClass` is in API group A4 (no issue yet) and `os.clock` must be confirmed in the sandbox's
standard library (#7). The lesson's `requires_api` hides it until both exist. `lastBoost` is never cleared when a
player leaves; unit N4 teaches `PlayerRemoving` and returns to this script.

---

## 9. Phased plan (proposed issues, not created)

**Phase L0: format and proof (with M1 and M2)**

1. **Learn: lesson format schema and validator.** `lesson.toml`, `start.toml`, patches, `l10n` files; a validator
   with word limits, ids, targets and reading-level checks. Docs in `learn/README.md`.
2. **Learn: headless lesson CI runner.** For each lesson: solutions pass their checks, the start fails the first
   check, `requires_api` exists. Runs on engine and `learn/` changes.
3. **Learn: `Learn` test library for script checks.** `spawnCharacter`, `touch`, `fireServer`, `step`, `output`,
   `expect`, `pass`, `fail` in the headless runtime with a fixed timestep and seed.
4. **Studio: UI target registry.** Named controls register ids when drawn; Explorer, Properties, viewport and script
   line targets; a debug overlay that shows ids in dev mode.
5. **Content: First steps track F1 to F4 on Parkour.** 14 lessons at starter level, with a first-time tester
   timing for each.

**Phase L1: the runner (with #83, M3)**

6. **Studio: lesson runner (Learn tab, guide strip, steps, checks).** One runner for lessons and template tours;
   replaces the tutorial runner part of #83.
7. **Studio: checks engine.** Event, world query, script test, Output and answer checks; debounce, cancel, 2 s
   feedback target.
8. **Studio: lesson projects, reset and attempts.** Projects under `Documents\Metallic\Learn`, step snapshots,
   reset step and lesson, last 5 attempts kept, "Keep as my game".
9. **Studio: local progress store and Learn home.** `progress.toml`, export and import, Next for you, track map,
   Ctrl K Learn group, Learn view on the start screen.
10. **Content: Script S1 to S8 and Build B1 to B2.** About 45 lessons; the shared feedback lists for S and B.
11. **Content: Multiplayer N1 to N3 and UI U1.** Lessons that run in one process now and with real clients after
    #156.

**Phase L2: deeper teaching (M4)**

12. **Studio: predict, quiz and Parsons steps.** Option explanations, keyboard-operable Parsons blocks, source-shape
    checks on the Luau syntax tree.
13. **Studio: misconception feedback.** `when` matching on Output, syntax errors, source shape and world state;
    shared per-track lists.
14. **Learn: Litha hint hook.** Send step text, authored hints and the solution as hidden context; mode limits in
    lessons; one eval case per lesson that Hint mode never reveals the solution (with #143).
15. **Studio: warm-ups and review card.** Spaced questions from finished units; no notifications.
16. **Content: Data, UI, 2D and Game design units** as their features land (#82, #79, #126, #84).

**Phase L3: credentials and reach (M6 and M7)**

17. **Learn: skill badges and test out.** Unit projects as badge checks; Skip ahead; capstone must-have checks.
18. **Platform: progress sync and public badges** (plateform repo). Opt-in sync, merge rule, under-13 rules.
19. **Learn: localisation pipeline.** Source hashes, stale detection, English fallback per step; Spanish and
    Portuguese (Brazil) first.
20. **Studio: read aloud and Learn accessibility pass.** OS text-to-speech, AccessKit roles and live regions,
    screen reader test of F1 to F3.
21. **Content: Art and sound, Ship and capstones** as M6 and the platform economy land.

**Missing engine issues this course depends on** (from section 3.4):

22. **Luau API: value objects and leaderstats (A8).** Needed by D1, Tycoon and Simulator.
23. **Luau API: teams (A9) and more remotes (A16).** Needed by N5 and G4.
24. **Luau API: character control and more Instance members (A10, A4).** `PlayerRemoving`, `LoadCharacter`,
    `FindFirstChildOfClass`; needed by N2.3, N4 and most templates.
25. **Luau API: ProximityPrompt (A15) and world UI (BillboardGui, SurfaceGui).** Needed by U4.
26. **Studio: Luau type checker in Problems.** Needed by S12 and Litha's `check_luau`.

---

## 10. Assistant's calls (overturn any)

- Ten tracks, the unit list and the letter N for Multiplayer.
- Suggest prerequisites, never lock lessons; no whole-course percentage.
- TOML lesson files with Luau check scripts; one format for lessons and template tours.
- Patches use Litha's `propose_change` command kinds.
- Highlight targets by name, with a registry; never screen coordinates.
- Script tests in the headless runtime with fixed timestep and seed; 30 simulated or 3 real seconds cap.
- Solution always available, with one "Try one more hint first?" prompt and no penalty.
- Litha in lessons limited to Explain, Hint and Show me (Do it with me in `make` steps).
- Learn tab splits the right dock above Properties during a lesson.
- Lesson projects in `Documents\Metallic\Learn`, last 5 attempts kept on reset.
- Local `progress.toml`; platform sync opt-in at M7; off for under-13 until parental consent.
- Course ships inside the install, 30 MB budget, no separate content downloads in v1.
- No grades, no streaks, no hearts; badges tied to made things; test out per unit; no exams or paid certificates.
- Finishing P3 Selling fairly before a creator's first paid product (owner to confirm; it touches the platform).
- Reading-level limits per track in CI; no dyslexia font; read aloud through the OS.
- Localisation order and English fallback for stale steps.
- Still images, no video, in v1.

---

## 11. Gaps and open questions

- **Networking design (#156) is not merged.** N1 to N5 assume Roblox-style replication and RemoteEvents; lesson
  details may change when it lands.
- **No `docs/luau-api.md` yet** (Task 7). `requires_api` gating needs it and the API index generator (#141).
- **No issues yet** for API groups A4, A8, A9, A10, A15, A16 and world UI, or for a Luau type checker in Problems
  (9, items 22 to 26).
- **Publishing and economy** live in the platform repo; P2, P3 and C5 wait for that design. The P3 rule needs the
  owner.
- **Under-13 policy** depends on the platform's parental consent and a legal review, as in the Litha doc.
- **Text-to-speech crate and egui right-to-left support** are not checked.
- **Evidence limits.** Many learning-research claims here come from search summaries, not full papers (labelled).
  The Duolingo streak and Codecademy criticism is partly anecdotal. Lesson time estimates are guesses until testers
  time them.
- **Content cost.** About 190 lessons is a large writing job. Phases L0 and L1 prove the format on about 70 lessons
  before the rest are written.

---

## 12. Sources

Fetched this session:
- Roblox tutorials index: https://create.roblox.com/docs/tutorials
- Roblox Core curriculum: https://create.roblox.com/docs/tutorials/curriculums/core
- Unity Learn pathways: https://learn.unity.com/pathways
- Rustlings usage: https://rustlings.rust-lang.org/usage/
- GDevelop in-app tutorials README: https://raw.githubusercontent.com/GDevelopApp/GDevelop-tutorials/main/docs/inAppTutorial/README.md
- GDevelop in-app tutorials reference: https://raw.githubusercontent.com/GDevelopApp/GDevelop-tutorials/main/docs/inAppTutorial/REFERENCE.md
- Worked-example effect: https://en.wikipedia.org/wiki/Worked-example_effect
- Scratch Foundation and MIT, New Pathways into creating with code: https://codeweek.eu/blog/new-pathways-into-creating-with-code/

Search summaries only (pages not opened):
- Roblox curriculum overview: https://create.roblox.com/docs/tutorials/curriculum-overview
- Unity Learn forum threads: https://discussions.unity.com/t/current-tutorials/940642 ,
  https://discussions.unity.com/t/unity-learn-create-with-code-not-marking-completed-things-complete/899902 ,
  https://discussions.unity.com/t/pathway-progression-doesnt-work-for-me/1579420
- Scratch in-editor tutorials: https://www.scratchfoundation.org/learn/learning-library/in-editor-tutorials
- Code.org CS Discoveries unit 3: https://curriculum.code.org/csd-20/unit3/
- Exercism unlocking: https://exercism.org/docs/building/product/unlocking-exercises
- Brilliant: https://brilliant.org/ and https://skillscouter.com/brilliant-review-math-science-coding/
- Streaks, Silverman and Barasch 2023: https://www.colorado.edu/business/news/2023/04/20/research-streaks-marketing-tech-barasch
- Streak commentary: https://thedecisionlab.com/insights/consumer-insights/streak-creep-the-perils-of-too-much-gamification
- Parsons problems working group 2023: https://repository.falmouth.ac.uk/5216/
- PRIMM: https://static.teachcomputing.org/pedagogy/QR11-PRIMM.pdf
- Subgoal labels: https://en.wikipedia.org/wiki/Subgoal_labeling and https://doaj.org/article/9a2c85d66fd54ec9ab019e9a510bbf17
- Spaced retrieval in intro STEM: https://link.springer.com/article/10.1186/s40594-024-00468-5
- Freeman et al. 2014: https://pmc.ncbi.nlm.nih.gov/articles/PMC4060683
- Bastani et al. 2025: https://knowledge.wharton.upenn.edu/article/without-guardrails-generative-ai-can-harm-education/
- Qian and Lehman 2017 (via citing papers): https://studylib.net/doc/27644813/student-misconception-in-programming
- Notional machine (Sorva, via citing papers): https://arxiv.org/pdf/2608.00844
- Luau traps: https://create.roblox.com/docs/scripting , https://devforum.roblox.com/t/partsi-is-nil/3861508 ,
  https://luau-lang.org/2021/07/30/luau-recap-july-2021.html
- Hedy: https://www.hedy.org/research/Gradual_Programming_in_Hedy_A_First_User_Study_2021.pdf
- Platformer Toolkit: https://gameworldobserver.com/2022/09/05/mark-brown-platformer-toolkit-hits-100k-downloads-plays
- WCAG 3.1.5: https://www.w3.org/WAI/WCAG21/Understanding/reading-level
- OpenDyslexic study: https://pmc.ncbi.nlm.nih.gov/articles/PMC5629233
- Fluent licences: https://licenses.dev/cargo/fluent/0.16.1

Cited from earlier Metallic docs: Cheng and Hill 2022, Chameleon tour completion, Duolingo day-7 retention, Yamamura's
Dojo, Kazemitabaar et al. (templates doc); Bastani et al. details and the hint ladder (Litha doc); Autodesk and NN/g
onboarding findings (Studio UI doc).
