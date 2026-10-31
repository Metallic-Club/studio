# 267: touch volumes follow the animated body

Issue #267, from #76 (PR #266). Before this change the invisible six-part R6 figure did the character's touches and
collisions, so lava and triggers reacted to fixed boxes instead of the animated limbs.

## What changed

- **Character parts (`metallic_physics::avatar`).** A character is now a `HumanoidRootPart` (2 x 2 x 1, CanCollide)
  plus the 15 R15 body parts, named as in Roblox: `Head`, `UpperTorso`, `LowerTorso`, `Left/RightUpperArm`,
  `Left/RightLowerArm`, `Left/RightHand`, `Left/RightUpperLeg`, `Left/RightLowerLeg` and `Left/RightFoot`. The body
  parts have CanCollide off and get no Rapier body (`PhysicsWorld::sync_from` skips them), so they never collide
  with or push anything. Only the root keeps a kinematic body.
- **Touch volumes (`metallic_physics::body`).**
  - Each body part holds one capsule, fixed in its own frame. The rest layout comes from the default avatar's joints
    (a table in the module docs).
  - `BodyPose` gives each part's frame relative to the root centre.
  - After moving, the controller does one broad-phase AABB lookup around the whole body. It keeps at most
    `MAX_TOUCH_CANDIDATES` (256) parts and narrow-tests each one against the movement capsule and the 15 capsules,
    each grown by 0.1 stud.
  - Contacts are a sorted `Vec` diffed against the previous step, and the buffers are reused, so nothing is
    allocated once warm.
- **Events.**
  - `CharacterEvent::Touched { character, part, hit }` and the new `TouchEnded` are reported per (character part,
    scene part) pair. `hit` is the body part, or the `HumanoidRootPart` for the movement capsule. The capsule still
    reports so that walls and bars the capsule stops at still count: it keeps the body about 1 stud from a wall,
    farther than the torso reaches.
  - Death ends every contact (TouchEnded). Destroyed parts get no TouchEnded.
- **Runtime (`metallic_runtime::body_pose`).**
  - `BodyRig` maps each part to a VRM bone and skins the part's rest frame rigidly with that bone's matrix.
  - `Game::update_avatar` calls `set_body_pose` every frame with `Avatar::body_pose`.
  - The ticks of the next frame use that pose, so it lags by one frame. Remote characters and the server have no
    avatar and use `BodyPose::rest`.
  - `feed_events` forwards `Touched` and `TouchEnded` with the body part as `hit`.
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

## Not done

- No part-to-part `TouchEnded`.
- The capsule's touches still report as `HumanoidRootPart`.
