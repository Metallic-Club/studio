//! Touches between characters: one character's touch volumes against another's.
//!
//! Characters the controller steps leave each other out of their scene queries (where the other
//! one's parts would be boxes posed a step late) and meet here instead: capsule against capsule,
//! all 16 volumes of each (the movement capsule and the 15 body parts, each grown by the touch
//! margin). Each contact is stored once per pair of characters and reported once, by whichever
//! character's step sees it start or end, as [`CharacterEvent::Touched`] /
//! [`CharacterEvent::TouchEnded`] with the other character's part as `part` and the stepping
//! character's part as `hit`; the runtime fires both sides.
//!
//! Cost per step of one character: one bounds check per other stepped character, then at most
//! [`MAX_CHARACTER_PAIRS`] characters whose volumes' bounds overlap (pairs already touching
//! first, then the nearest), each with at most 16 x 16 capsule tests behind box checks.
//! Characters the controller does not step (NPC Models, remote characters on a client) have no
//! volumes here; their parts stay ordinary scene parts.
use crate::CharacterEvent;
use crate::avatar::PartIds;
use crate::body::{BODY_PART_COUNT, BodyPart, ROOT_HIT};
use metallic_core::InstanceId;
use rapier3d::parry::bounding_volume::{Aabb, BoundingVolume};
use rapier3d::parry::query::intersection_test;
use rapier3d::parry::shape::Capsule;
use rapier3d::prelude::Pose;

/// Most other characters one character's step tests its volumes against.
pub const MAX_CHARACTER_PAIRS: usize = 16;

/// Volumes per character: the 15 body parts, then the movement capsule ([`ROOT_HIT`]).
pub(crate) const VOLUMES: usize = BODY_PART_COUNT + 1;

/// A character's touch volumes in world space after its last step.
#[derive(Clone, Copy)]
pub(crate) struct WorldVolumes {
    pub shapes: [Capsule; VOLUMES],
    pub boxes: [Aabb; VOLUMES],
    pub bounds: Aabb,
}

/// What one character's last pass against the others cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PairCost {
    /// Characters tested (at most [`MAX_CHARACTER_PAIRS`]).
    pub characters: usize,
    /// Capsule tests (at most 256 per character).
    pub tests: usize,
    /// Overlapping characters left out because the cap was reached.
    pub dropped: usize,
}

/// The current contacts of two characters, `a` below `b` in id order; each contact is
/// `(a's part, b's part)`, sorted.
struct Pair {
    a: InstanceId,
    b: InstanceId,
    contacts: Vec<(InstanceId, InstanceId)>,
}

/// Contacts between characters and reusable buffers.
#[derive(Default)]
pub(crate) struct CharacterPairs {
    pairs: Vec<Pair>,
    /// Pairs emptied this step, kept for their buffers.
    spare: Vec<Vec<(InstanceId, InstanceId)>>,
    /// Characters overlapping the one stepped: (priority, character).
    near: Vec<(f32, InstanceId)>,
    next: Vec<(InstanceId, InstanceId)>,
    pub cost: std::collections::HashMap<InstanceId, PairCost>,
}

/// The part of `ids` for volume `i`.
fn part_of(ids: &PartIds, i: usize) -> Option<InstanceId> {
    if i == usize::from(ROOT_HIT) {
        ids.root()
    } else {
        BodyPart::ALL.get(i).and_then(|&p| ids.body(p))
    }
}

/// Squared distance between the centres of two boxes.
fn centre_distance_sq(a: &Aabb, b: &Aabb) -> f32 {
    (a.center() - b.center()).length_squared()
}

impl CharacterPairs {
    /// Call after `me` stepped with `mine`: tests it against `others` (every other live, stepped
    /// character with volumes) and reports the contacts that started or ended.
    pub fn update<'a>(
        &mut self,
        me: InstanceId,
        mine: (&WorldVolumes, &PartIds),
        others: impl Iterator<Item = (InstanceId, &'a WorldVolumes, &'a PartIds)> + Clone,
        events: &mut Vec<CharacterEvent>,
    ) {
        let (my_volumes, my_ids) = mine;
        // 1. Who overlaps: pairs already touching first, then the nearest.
        self.near.clear();
        for (other, volumes, _) in others.clone() {
            if other == me || !volumes.bounds.intersects(&my_volumes.bounds) {
                continue;
            }
            let touching = self.find(me, other).is_some();
            let priority = if touching {
                -1.0
            } else {
                centre_distance_sq(&volumes.bounds, &my_volumes.bounds)
            };
            self.near.push((priority, other));
        }
        let mut cost = PairCost::default();
        if self.near.len() > MAX_CHARACTER_PAIRS {
            cost.dropped = self.near.len() - MAX_CHARACTER_PAIRS;
            self.near
                .select_nth_unstable_by(MAX_CHARACTER_PAIRS, |x, y| x.0.total_cmp(&y.0));
            self.near.truncate(MAX_CHARACTER_PAIRS);
        }
        cost.characters = self.near.len();

        // 2. Pairs of `me` with a character no longer near end.
        let mut i = 0;
        while i < self.pairs.len() {
            let p = &self.pairs[i];
            let other = if p.a == me {
                Some(p.b)
            } else if p.b == me {
                Some(p.a)
            } else {
                None
            };
            match other {
                Some(o) if !self.near.iter().any(|&(_, n)| n == o) => {
                    let pair = self.pairs.swap_remove(i);
                    end_pair(&pair, events);
                    self.recycle(pair.contacts);
                }
                _ => i += 1,
            }
        }

        // 3. Capsule tests against each near character.
        for k in 0..self.near.len() {
            let other = self.near[k].1;
            let Some((_, theirs, their_ids)) = others.clone().find(|(o, _, _)| *o == other) else {
                continue;
            };
            let (a, b) = if me.to_bits() < other.to_bits() {
                (me, other)
            } else {
                (other, me)
            };
            self.next.clear();
            for (i, mine) in my_volumes.shapes.iter().enumerate() {
                if !my_volumes.boxes[i].intersects(&theirs.bounds) {
                    continue;
                }
                for (j, their) in theirs.shapes.iter().enumerate() {
                    if !my_volumes.boxes[i].intersects(&theirs.boxes[j]) {
                        continue;
                    }
                    cost.tests += 1;
                    let hit = intersection_test(&Pose::IDENTITY, mine, &Pose::IDENTITY, their)
                        .is_ok_and(|hit| hit.intersecting);
                    if let (true, Some(my_part), Some(their_part)) =
                        (hit, part_of(my_ids, i), part_of(their_ids, j))
                    {
                        self.next.push(if a == me {
                            (my_part, their_part)
                        } else {
                            (their_part, my_part)
                        });
                    }
                }
            }
            self.next.sort_unstable();
            self.next.dedup();
            let index = match self.find(a, b) {
                Some(index) => index,
                None if self.next.is_empty() => continue,
                None => {
                    let contacts = self.spare.pop().unwrap_or_default();
                    self.pairs.push(Pair { a, b, contacts });
                    self.pairs.len() - 1
                }
            };
            let pair = &mut self.pairs[index];
            let report = |(pa, pb): (InstanceId, InstanceId), start: bool| {
                // From `me`'s side: the other's part is touched by mine.
                let (part, hit) = if a == me { (pb, pa) } else { (pa, pb) };
                if start {
                    CharacterEvent::Touched {
                        character: me,
                        part,
                        hit,
                    }
                } else {
                    CharacterEvent::TouchEnded {
                        character: me,
                        part,
                        hit,
                    }
                }
            };
            crate::body::diff_sorted(&pair.contacts, &self.next, |c, start| {
                events.push(report(c, start))
            });
            std::mem::swap(&mut pair.contacts, &mut self.next);
            if pair.contacts.is_empty() {
                let pair = self.pairs.swap_remove(index);
                self.recycle(pair.contacts);
            }
        }
        self.cost.insert(me, cost);
    }

    /// Ends every contact of `character` with other characters (it died, or is gone); each
    /// TouchEnded is reported by the other character.
    pub fn end(&mut self, character: InstanceId, events: &mut Vec<CharacterEvent>) {
        let mut i = 0;
        while i < self.pairs.len() {
            if self.pairs[i].a == character || self.pairs[i].b == character {
                let pair = self.pairs.swap_remove(i);
                end_pair_for(&pair, character, events);
                self.recycle(pair.contacts);
            } else {
                i += 1;
            }
        }
        self.cost.remove(&character);
    }

    /// The other characters' parts touching `character`'s parts, as `(mine, theirs)`.
    pub fn touching(&self, character: InstanceId) -> Vec<(InstanceId, InstanceId)> {
        let mut out = Vec::new();
        for p in &self.pairs {
            if p.a == character {
                out.extend(p.contacts.iter().copied());
            } else if p.b == character {
                out.extend(p.contacts.iter().map(|&(x, y)| (y, x)));
            }
        }
        out
    }

    fn find(&self, x: InstanceId, y: InstanceId) -> Option<usize> {
        self.pairs
            .iter()
            .position(|p| (p.a == x && p.b == y) || (p.a == y && p.b == x))
    }

    fn recycle(&mut self, mut contacts: Vec<(InstanceId, InstanceId)>) {
        contacts.clear();
        self.spare.push(contacts);
    }
}

/// Reports the end of every contact of `pair`, from `a`'s side.
fn end_pair(pair: &Pair, events: &mut Vec<CharacterEvent>) {
    for &(pa, pb) in &pair.contacts {
        events.push(CharacterEvent::TouchEnded {
            character: pair.a,
            part: pb,
            hit: pa,
        });
    }
}

/// Reports the end of every contact of `pair` from the side of the character that is not
/// `gone`.
fn end_pair_for(pair: &Pair, gone: InstanceId, events: &mut Vec<CharacterEvent>) {
    for &(pa, pb) in &pair.contacts {
        let (character, part, hit) = if pair.a == gone {
            (pair.b, pa, pb)
        } else {
            (pair.a, pb, pa)
        };
        events.push(CharacterEvent::TouchEnded {
            character,
            part,
            hit,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{BodyPose, TOUCH_MARGIN};
    use crate::to_vector;
    use metallic_core::{Vec3, World};
    use rapier3d::parry::shape::Shape;

    /// A new character and its part ids.
    fn character(world: &mut World) -> (InstanceId, PartIds) {
        let players = world.root("Players").unwrap();
        let player = world.create("Player", Some(players)).unwrap();
        let model = crate::spawn_character(world, player).unwrap();
        let mut ids = PartIds::default();
        ids.refresh(world, model);
        (model, ids)
    }

    fn rest_volumes(root: Vec3) -> WorldVolumes {
        let pose = BodyPose::rest(0.0);
        let margin = TOUCH_MARGIN as f32;
        let shapes: [Capsule; VOLUMES] = std::array::from_fn(|i| match BodyPart::ALL.get(i) {
            Some(&p) => {
                let (a, b) = pose.world_ends(p, root);
                Capsule::new(to_vector(a), to_vector(b), p.radius() as f32 + margin)
            }
            None => Capsule::new(
                to_vector(root - Vec3::new(0.0, 1.5, 0.0)),
                to_vector(root + Vec3::new(0.0, 1.0, 0.0)),
                1.1,
            ),
        });
        let boxes = shapes.map(|c| c.compute_aabb(&Pose::IDENTITY));
        let mut bounds = boxes[0];
        for b in &boxes[1..] {
            bounds.merge(b);
        }
        WorldVolumes {
            shapes,
            boxes,
            bounds,
        }
    }

    #[test]
    fn touches_between_characters_do_not_allocate_once_warm() {
        let mut world = World::new_with_services();
        let (a, a_ids) = character(&mut world);
        let (b, b_ids) = character(&mut world);
        let va = rest_volumes(Vec3::new(0.0, 3.0, 0.0));
        let near = rest_volumes(Vec3::new(1.2, 3.0, 0.0));
        let far = rest_volumes(Vec3::new(1.2, 3.0, 9.0));
        let mut pairs = CharacterPairs::default();
        let mut events = Vec::with_capacity(1 << 16);
        // Alternate near and far so contacts start and end every step.
        let frame = |pairs: &mut CharacterPairs, events: &mut Vec<CharacterEvent>, k: usize| {
            let vb = if k.is_multiple_of(2) { &near } else { &far };
            pairs.update(a, (&va, &a_ids), [(b, vb, &b_ids)].into_iter(), events);
        };
        for k in 0..100 {
            frame(&mut pairs, &mut events, k);
        }
        events.clear();
        let ((), allocations) = crate::alloc_count::count(|| {
            for k in 0..200 {
                frame(&mut pairs, &mut events, k);
            }
        });
        assert!(!events.is_empty(), "the contacts should change");
        assert_eq!(allocations, 0);
    }
}
