# 371 (item 2): limb-against-limb touches between characters

Issue #371, item 2 (item 1, part-to-part `TouchEnded`, is #417, which took over #381). Before this change one character's limbs met
another character only as scene parts: its parts' boxes, posed a step late. A pointing hand touched the other's
`HumanoidRootPart` but not its limbs, and both characters' steps reported the same contact, so scripts got it twice.

## Decision

Yes: as in Roblox, both characters' parts touch. `limb.Touched(otherLimb)` and `otherLimb.Touched(limb)` each fire
once per contact.

## What changed

- **Scene query.** A character's scene query skips the parts of every other character the controller steps (their
  parts' Models have a movement state). Characters it does not step, such as NPC Models or remote characters on a
  client, stay ordinary scene parts.
- **Pairs (`metallic_physics::limbs`).**
  - After its step, a character is tested against every other stepped, live character whose volume bounds overlap
    its own. All 16 volumes of each are compared (the movement capsule and the 15 body capsules, each grown by the
    touch margin), capsule against capsule, behind box checks.
  - Each pair of characters stores its contacts once, as sorted part pairs. Whichever character's step sees a contact
    start or end reports it: `CharacterEvent::Touched`/`TouchEnded` with the other character's part as `part` and its
    own part as `hit`. The runtime fires both sides.
  - Dying, being destroyed, being replaced or respawning ends the character's pairs, reported from the survivor's
    side.
- **Bounded cost.**
  - Per step: one bounds check per other stepped character, then at most `MAX_CHARACTER_PAIRS` (16) overlapping
    characters (pairs already touching first, then the nearest), at most 256 capsule tests each.
  - The rest are counted in `TouchQueryCost::characters_dropped`, next to `characters` and `character_tests`.
  - Nothing is allocated once warm.
- **Known limit.** A character first stepped this tick is still a scene part to the others for that one step, so a
  contact that exists from its first tick reports once as a scene touch and then once as a pair.

## Tests

`metallic_physics/tests/character.rs`:
- a pointing hand touches the other character's limbs, not only its root, and each contact is reported once
  although both characters step;
- separating ends every contact once;
- dying or being destroyed ends the contacts;
- a 41-character crowd stays within the caps and reaches them;
- an unstepped character is still touched as scene parts.

`limbs.rs`: the pair pass makes no allocations over 200 warm steps whose contacts change each step.
