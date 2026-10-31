# 267: touch volumes follow the animated body

Issue #267, from #76 (PR #266). Before this change the invisible six-part R6 figure did the character's touches and
collisions, so lava and triggers reacted to fixed boxes instead of the animated limbs.

## What changed

- **Character parts (`metallic_physics::avatar`).** A character is now a `HumanoidRootPart` (2 x 2 x 1, CanCollide)
  plus the 15 R15 body parts, named as in Roblox: `Head`, `UpperTorso`, `LowerTorso`, `Left/RightUpperArm`,
  `Left/RightLowerArm`, `Left/RightHand`, `Left/RightUpperLeg`, `Left/RightLowerLeg` and `Left/RightFoot`. The body
  parts have CanCollide off, so the physics keeps them as kinematic sensors that never collide with or push
  anything; rays skip them. Nothing is matched by name (review B1: the first version skipped every R15-named part in
  any Humanoid Model, which took the physics away from NPCs).
- **Touch volumes (`metallic_physics::body`).**
  - Each body part holds one capsule, fixed in its own frame. The rest layout comes from the default avatar's joints
    (a table in the module docs).
  - `BodyPose` gives each part's frame relative to the root centre.
  - After moving, the controller does one broad-phase AABB lookup around the whole body. It keeps at most
    `MAX_TOUCH_CANDIDATES` (256) parts and narrow-tests each one against the movement capsule and the 15 capsules,
    each grown by 0.1 stud. Parts already touching are kept first, then the nearest to the root; the rest are counted
    (`TouchQueryCost::dropped`, `capped_queries`) and a warning is printed the first time a character hits the cap.
  - Contacts are a sorted `Vec` diffed against the previous step, and the buffers are reused, so nothing is
    allocated once warm.
- **Events.**
  - `CharacterEvent::Touched { character, part, hit }` and the new `TouchEnded` are reported per (character part,
    scene part) pair. `hit` is the body part, or the `HumanoidRootPart` for the movement capsule. The capsule still
    reports so that walls and bars the capsule stops at still count: it keeps the body about 1 stud from a wall,
    farther than the torso reaches.
  - Death ends every contact (TouchEnded), and so does a character destroyed or replaced without dying. Destroyed
    touched parts get no TouchEnded.
- **Runtime (`metallic_runtime::body_pose`).**
  - `BodyAnimator` runs the locomotion over the skeleton on the CPU (clip sampling, blending, joint hierarchy). It
    depends on `metallic_asset` and `metallic_anim` only, not on `metallic_render`, so a headless Simulation (#309)
    can run it. The drawn `Avatar` wraps the same animator.
  - `BodyRig` maps each part to a VRM bone and skins the part's rest frame rigidly with that bone's matrix.
  - Each tick, after the controller step, `CharacterBodies::animate` advances the character's animator and sets the
    pose for the next step (one tick behind). A test checks the headless pose matches the drawn avatar's.
  - `feed_events` fires both sides: `part.Touched(limb)` and `limb.Touched(part)`, and the same for `TouchEnded`.
- **Scripts.** `BasePart.TouchEnded` was added (`ScriptHost::touch_ended`). Part-to-part contacts still report only
  `Touched`.

## Tests

- **Physics, `tests/character.rs`:**
  - a pointing hand touches a sensor that the idle body does not;
  - volumes never push or block (a loose post stays put, and rays pass through);
  - every Touched has a matching TouchEnded;
  - the query cost is bounded: 4,000 far parts are never looked at, and a dense cloud is capped at 256 parts and
    16 tests per part;
  - the R15 names and sizes.
- **Physics, `body.rs` unit tests:** the mirror, feet and head extents, the yaw, the contact diff, and query plus
  commit allocating nothing over 600 warm steps.
- **Runtime, `body_pose.rs`:**
  - the rest table matches the GLB joints (within 1 cm);
  - the rest avatar gives `BodyPose::rest`;
  - the volumes follow the run clip (the arm swings more than 0.8 stud, the foot lifts) and the jump clip (the feet
    tuck);
  - computing the pose allocates nothing;
  - with the real run clip on a standing character, the hand swings into a sensor (`RightHand` Touched, then
    TouchEnded each stride), while the idle avatar beside the sensor never touches it.
- **Scripts:** a `TouchEnded` test.

## Review fixes (FIX-FIRST on 51a967fa)

B1 name matching removed; CPU pose without the renderer; TouchEnded on destroy or replace; cap keeps existing
contacts and the nearest parts and counts the rest; both sides fire; `step` reuses its collider and walk buffers.
New tests: NPC with R15-named parts keeps solid physics (and renames change nothing), contacts survive the cap,
destroyed or replaced characters end their contacts, headless animator matches the avatar, animating allocates
nothing, and an end-to-end `Game::frame` test (`tests/character_touches.rs`).

Build note: the shared `CARGO_TARGET_DIR` can hold another worktree's artifacts of the same crates that look fresh;
touch this worktree's sources before building.

## Not done

- No part-to-part `TouchEnded`, and no limb-vs-limb touches between characters (follow-up issue from the
  coordinator).
- The capsule's touches still report as `HumanoidRootPart`.
