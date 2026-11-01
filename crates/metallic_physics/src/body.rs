//! The character's touch volumes: one capsule per R15 body part, carried by the animated bones.
//!
//! Each [`BodyPart`] is a capsule (a segment with a radius) fixed in its part's frame. A
//! [`BodyPose`] places every part's frame relative to the `HumanoidRootPart` centre. The runtime
//! evaluates the avatar's animation on the CPU each tick (no renderer needed, so a headless
//! server computes the same limbs) and hands the result to
//! [`crate::CharacterController::set_body_pose`]. Without one, for example before the first
//! tick, the controller uses [`BodyPose::rest`], the avatar's rest pose turned to the
//! character's facing.
//!
//! The volumes are queries of the controller. The body part instances that hold them have
//! CanCollide off, so they are sensors and never collide with or push anything. After each move,
//! [`TouchVolumes::query`] does one broad-phase lookup around the whole body and tests at most
//! [`MAX_TOUCH_CANDIDATES`] parts against the movement capsule and the 15 capsules: first the
//! parts already touching, then the nearest others. Parts beyond the cap are counted
//! ([`crate::TouchQueryCost`]). The narrow-phase cost per character per step is therefore
//! bounded, and once the buffers are warm nothing is allocated.
//!
//! Rest layout (studs, relative to the root centre, facing -Z, so a character's left is -X).
//! These are the default avatar's joints from `art/avatar/out/default_avatar.glb`: 1 stud is
//! 0.28 m and the root centre is 3 studs above the feet. The runtime tests check this table
//! against the model.
//!
//! | Part          | From                 | To                   | Radius |
//! |---------------|----------------------|----------------------|--------|
//! | Head          | (0, 1.54, 0.04)      | (0, 2.11, 0.04)      | 0.39   |
//! | UpperTorso    | (-0.14, 0.66, 0.02)  | (0.14, 0.66, 0.02)   | 0.34   |
//! | LowerTorso    | (-0.11, -0.04, 0)    | (0.11, -0.04, 0)     | 0.32   |
//! | LeftUpperArm  | (-0.44, 0.74, 0)     | (-0.70, 0.04, -0.03) | 0.17   |
//! | LeftLowerArm  | (-0.70, 0.04, -0.03) | (-1.01, -0.76, -0.16)| 0.16   |
//! | LeftHand      | (-1.01, -0.76, -0.16)| (-1.20, -1.32, -0.31)| 0.16   |
//! | LeftUpperLeg  | (-0.25, -0.38, 0)    | (-0.45, -1.53, -0.01)| 0.25   |
//! | LeftLowerLeg  | (-0.45, -1.53, -0.01)| (-0.66, -2.60, 0.09) | 0.20   |
//! | LeftFoot      | (-0.66, -2.82, 0.25) | (-0.70, -2.82, -0.61)| 0.18   |
//!
//! The right side mirrors X. Each body part instance is a box that holds its capsule: its Size
//! is the capsule's extent in the rest frame.
use crate::limbs::WorldVolumes;
use crate::{PhysicsWorld, to_vector};
use metallic_core::{CFrame, InstanceId, Vec3};
use rapier3d::parry::bounding_volume::{Aabb, BoundingVolume};
use rapier3d::parry::shape::{Capsule, Shape};
use rapier3d::prelude::{ColliderHandle, Pose, QueryFilter, Vector};

/// Number of [`BodyPart`]s.
pub const BODY_PART_COUNT: usize = 15;
/// Most parts one character's touch query looks at in a step. Parts beyond this in a dense
/// pile are ignored until others leave, which keeps the cost per step bounded.
pub const MAX_TOUCH_CANDIDATES: usize = 256;
/// What a character's last touch query cost (see
/// [`crate::CharacterController::touch_query_cost`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TouchQueryCost {
    /// Parts tested, at most [`MAX_TOUCH_CANDIDATES`].
    pub parts: usize,
    /// Narrow-phase shape tests, at most 16 per part.
    pub tests: usize,
    /// Parts left out because the cap was reached (the farthest from the root).
    pub dropped: usize,
    /// Queries of this character so far that reached the cap.
    pub capped_queries: u64,
    /// Other characters its volumes were tested against, at most
    /// [`crate::MAX_CHARACTER_PAIRS`].
    pub characters: usize,
    /// Capsule tests against those characters, at most 256 each.
    pub character_tests: usize,
    /// Overlapping characters left out because that cap was reached.
    pub characters_dropped: usize,
}

impl TouchQueryCost {
    pub(crate) fn with_characters(mut self, c: crate::limbs::PairCost) -> TouchQueryCost {
        self.characters = c.characters;
        self.character_tests = c.tests;
        self.characters_dropped = c.dropped;
        self
    }
}

/// How much the touch query grows the movement capsule and every body volume, in studs.
pub(crate) const TOUCH_MARGIN: f64 = 0.1;

/// One R15 body part of a character, named as in Roblox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum BodyPart {
    Head,
    UpperTorso,
    LowerTorso,
    LeftUpperArm,
    LeftLowerArm,
    LeftHand,
    RightUpperArm,
    RightLowerArm,
    RightHand,
    LeftUpperLeg,
    LeftLowerLeg,
    LeftFoot,
    RightUpperLeg,
    RightLowerLeg,
    RightFoot,
}

/// A rest capsule: ends in the rest frame (studs from the root centre) and radius.
struct Rest {
    a: (f64, f64, f64),
    b: (f64, f64, f64),
    radius: f64,
}

const fn rest(a: (f64, f64, f64), b: (f64, f64, f64), radius: f64) -> Rest {
    Rest { a, b, radius }
}

/// The left side and the middle, indexed like [`BodyPart::ALL`]; the right side mirrors X.
const REST: [Rest; BODY_PART_COUNT] = [
    rest((0.0, 1.54, 0.04), (0.0, 2.11, 0.04), 0.39),
    rest((-0.14, 0.66, 0.02), (0.14, 0.66, 0.02), 0.34),
    rest((-0.11, -0.04, 0.0), (0.11, -0.04, 0.0), 0.32),
    rest((-0.44, 0.74, 0.0), (-0.70, 0.04, -0.03), 0.17),
    rest((-0.70, 0.04, -0.03), (-1.01, -0.76, -0.16), 0.16),
    rest((-1.01, -0.76, -0.16), (-1.20, -1.32, -0.31), 0.16),
    rest((0.44, 0.74, 0.0), (0.70, 0.04, -0.03), 0.17),
    rest((0.70, 0.04, -0.03), (1.01, -0.76, -0.16), 0.16),
    rest((1.01, -0.76, -0.16), (1.20, -1.32, -0.31), 0.16),
    rest((-0.25, -0.38, 0.0), (-0.45, -1.53, -0.01), 0.25),
    rest((-0.45, -1.53, -0.01), (-0.66, -2.60, 0.09), 0.20),
    rest((-0.66, -2.82, 0.25), (-0.70, -2.82, -0.61), 0.18),
    rest((0.25, -0.38, 0.0), (0.45, -1.53, -0.01), 0.25),
    rest((0.45, -1.53, -0.01), (0.66, -2.60, 0.09), 0.20),
    rest((0.66, -2.82, 0.25), (0.70, -2.82, -0.61), 0.18),
];

fn v((x, y, z): (f64, f64, f64)) -> Vec3 {
    Vec3::new(x, y, z)
}

impl BodyPart {
    /// Every body part, in index order.
    pub const ALL: [BodyPart; BODY_PART_COUNT] = [
        BodyPart::Head,
        BodyPart::UpperTorso,
        BodyPart::LowerTorso,
        BodyPart::LeftUpperArm,
        BodyPart::LeftLowerArm,
        BodyPart::LeftHand,
        BodyPart::RightUpperArm,
        BodyPart::RightLowerArm,
        BodyPart::RightHand,
        BodyPart::LeftUpperLeg,
        BodyPart::LeftLowerLeg,
        BodyPart::LeftFoot,
        BodyPart::RightUpperLeg,
        BodyPart::RightLowerLeg,
        BodyPart::RightFoot,
    ];

    /// Position in [`BodyPart::ALL`] and in [`BodyPose::parts`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The instance name, as in Roblox (`"Head"`, `"UpperTorso"`, `"LeftHand"`, …).
    pub const fn name(self) -> &'static str {
        match self {
            BodyPart::Head => "Head",
            BodyPart::UpperTorso => "UpperTorso",
            BodyPart::LowerTorso => "LowerTorso",
            BodyPart::LeftUpperArm => "LeftUpperArm",
            BodyPart::LeftLowerArm => "LeftLowerArm",
            BodyPart::LeftHand => "LeftHand",
            BodyPart::RightUpperArm => "RightUpperArm",
            BodyPart::RightLowerArm => "RightLowerArm",
            BodyPart::RightHand => "RightHand",
            BodyPart::LeftUpperLeg => "LeftUpperLeg",
            BodyPart::LeftLowerLeg => "LeftLowerLeg",
            BodyPart::LeftFoot => "LeftFoot",
            BodyPart::RightUpperLeg => "RightUpperLeg",
            BodyPart::RightLowerLeg => "RightLowerLeg",
            BodyPart::RightFoot => "RightFoot",
        }
    }

    /// The body part with this instance name.
    pub fn from_name(name: &str) -> Option<BodyPart> {
        BodyPart::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The capsule's two ends in the rest frame: studs from the root centre, facing -Z.
    pub fn rest_ends(self) -> (Vec3, Vec3) {
        let r = &REST[self.index()];
        (v(r.a), v(r.b))
    }

    /// The capsule's radius in studs (before the touch margin).
    pub fn radius(self) -> f64 {
        REST[self.index()].radius
    }

    /// Centre of the part (and of its capsule) in the rest frame.
    pub fn rest_centre(self) -> Vec3 {
        let (a, b) = self.rest_ends();
        (a + b) * 0.5
    }

    /// The capsule's ends in the part's own frame (around its centre).
    pub fn local_ends(self) -> (Vec3, Vec3) {
        let (a, b) = self.rest_ends();
        let half = (b - a) * 0.5;
        (half * -1.0, half)
    }

    /// The part's Size: the capsule's extent along the axes of the rest frame.
    pub fn size(self) -> Vec3 {
        let (a, b) = self.rest_ends();
        let d = self.radius() * 2.0;
        Vec3::new(
            (b.x - a.x).abs() + d,
            (b.y - a.y).abs() + d,
            (b.z - a.z).abs() + d,
        )
    }
}

/// Where every body part is, relative to the `HumanoidRootPart` centre.
///
/// `parts[i]` belongs to `BodyPart::ALL[i]`. Its position is the part centre minus the root
/// centre, in world axes, and its rotation is the part's world rotation. At rest, facing
/// `Orientation.Y = 0`, every rotation is the identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyPose {
    /// One frame per body part, in [`BodyPart::ALL`] order.
    pub parts: [CFrame; BODY_PART_COUNT],
}

impl BodyPose {
    /// The rest pose for a character facing `yaw` degrees around +Y (0 looks along -Z).
    pub fn rest(yaw: f64) -> BodyPose {
        let turn = CFrame::from_position_orientation(Vec3::default(), Vec3::new(0.0, yaw, 0.0));
        BodyPose {
            parts: BodyPart::ALL.map(|p| turn * CFrame::from_position(p.rest_centre())),
        }
    }

    /// True when every frame is finite.
    pub fn is_finite(&self) -> bool {
        self.parts.iter().all(|c| c.is_finite())
    }

    /// The capsule ends of `part` in world space for a root centre at `root`.
    pub fn world_ends(&self, part: BodyPart, root: Vec3) -> (Vec3, Vec3) {
        let frame = self.parts[part.index()];
        let (a, b) = part.local_ends();
        (
            root + frame.point_to_world(a),
            root + frame.point_to_world(b),
        )
    }
}

/// Which body part touches: one of the 15 volumes or, for [`ROOT_HIT`], the movement capsule
/// (reported as the `HumanoidRootPart`).
pub(crate) type Hit = u8;
/// [`Hit`] of the movement capsule.
pub(crate) const ROOT_HIT: Hit = BODY_PART_COUNT as Hit;

/// One touching pair: a body part of the character and a part of the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Contact {
    pub hit: Hit,
    pub part: InstanceId,
}

/// A part the touch query will test, and how far its bounds are from the root.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    /// Squared distance from the root centre to the part's bounding box (0 for parts that
    /// already touch, so they are kept first).
    distance: f32,
    handle: ColliderHandle,
    part: InstanceId,
}

/// One character's touch volumes, their current contacts and reusable buffers.
pub(crate) struct TouchVolumes {
    /// The pose from the animated avatar, if the runtime supplied one.
    pub animated: Option<BodyPose>,
    /// Contacts after the last query, sorted.
    pub touching: Vec<Contact>,
    /// Contacts of the query in progress (swapped with `touching`).
    next: Vec<Contact>,
    /// Parts already touching, sorted (for the query in progress).
    kept: Vec<InstanceId>,
    candidates: Vec<Candidate>,
    /// Parts the last query tested (at most [`MAX_TOUCH_CANDIDATES`]).
    pub tested_parts: usize,
    /// Narrow-phase shape tests done by the last query.
    pub tests: usize,
    /// Parts the last query left out because the cap was reached (the farthest ones).
    pub dropped: usize,
    /// Queries so far that hit the cap.
    pub capped_queries: u64,
    /// The volumes in world space after the last query (for touches between characters);
    /// `None` before the first query, after `clear`, or when they were not finite.
    pub world: Option<WorldVolumes>,
}

impl Default for TouchVolumes {
    fn default() -> Self {
        TouchVolumes {
            animated: None,
            touching: Vec::with_capacity(32),
            next: Vec::with_capacity(32),
            kept: Vec::with_capacity(32),
            candidates: Vec::with_capacity(MAX_TOUCH_CANDIDATES),
            tested_parts: 0,
            tests: 0,
            dropped: 0,
            world: None,
            capped_queries: 0,
        }
    }
}

/// Squared distance from `p` to the box `b` (0 inside).
fn distance_sq(b: &Aabb, p: Vector) -> f32 {
    let d = (b.mins - p).max(Vector::ZERO).max(p - b.maxs);
    d.length_squared()
}

impl TouchVolumes {
    /// The pose to use for a character facing `yaw` degrees: the animated one, or the rest pose.
    pub fn pose(&self, yaw: f64) -> BodyPose {
        self.animated.unwrap_or_else(|| BodyPose::rest(yaw))
    }

    /// Finds what the movement capsule (`probe` at `probe_pose`) and every volume of `pose`
    /// placed at `root` touch, each grown by [`TOUCH_MARGIN`]; sensors included, the
    /// character's own colliders (`own`) skipped. The result goes to `self.next`.
    ///
    /// At most [`MAX_TOUCH_CANDIDATES`] parts are tested. Parts that already touch come first,
    /// so a crowd around the character never ends a contact that still holds; the remaining
    /// slots go to the parts nearest the root. Parts left out are counted in `dropped`.
    pub fn query(
        &mut self,
        physics: &PhysicsWorld,
        skip: &dyn Fn(ColliderHandle) -> bool,
        probe: &Capsule,
        probe_pose: &Pose,
        pose: &BodyPose,
        root: Vec3,
    ) {
        self.next.clear();
        self.candidates.clear();
        self.kept.clear();
        self.tested_parts = 0;
        self.tests = 0;
        self.dropped = 0;
        let margin = TOUCH_MARGIN as f32;
        let volumes: [Capsule; BODY_PART_COUNT] = BodyPart::ALL.map(|p| {
            let (a, b) = pose.world_ends(p, root);
            Capsule::new(to_vector(a), to_vector(b), p.radius() as f32 + margin)
        });
        let identity = Pose::IDENTITY;
        let mut bounds = probe.compute_aabb(probe_pose);
        let boxes: [Aabb; BODY_PART_COUNT] = volumes.each_ref().map(|c| c.compute_aabb(&identity));
        for b in &boxes {
            bounds.merge(b);
        }
        if !finite(&bounds) {
            self.world = None;
            return;
        }
        self.world = Some(world_volumes(&volumes, &boxes, probe, probe_pose, bounds));
        let keep = |h: ColliderHandle, _: &rapier3d::prelude::Collider| !skip(h);
        let queries = physics.queries(QueryFilter::default().predicate(&keep));

        // 1. Parts that touch already.
        self.kept.extend(self.touching.iter().map(|c| c.part));
        self.kept.sort_unstable();
        self.kept.dedup();
        for &part in &self.kept {
            if self.candidates.len() == MAX_TOUCH_CANDIDATES {
                break;
            }
            if let Some(handle) = physics.collider_of(part)
                && !skip(handle)
            {
                self.candidates.push(Candidate {
                    distance: 0.0,
                    handle,
                    part,
                });
            }
        }
        // 2. The nearest of the others. `farthest` indexes the farthest new candidate once the
        // list is full.
        let first_new = self.candidates.len();
        let centre = to_vector(root);
        let mut farthest: Option<usize> = None;
        for (handle, collider) in queries.intersect_aabb_conservative(bounds) {
            let Some(part) = physics.owner_of(handle) else {
                continue;
            };
            if self.kept.binary_search(&part).is_ok() {
                continue;
            }
            let c = Candidate {
                distance: distance_sq(&collider.compute_aabb(), centre),
                handle,
                part,
            };
            if self.candidates.len() < MAX_TOUCH_CANDIDATES {
                self.candidates.push(c);
                continue;
            }
            self.dropped += 1;
            if first_new == self.candidates.len() {
                continue;
            }
            let far = *farthest
                .get_or_insert_with(|| farthest_of(&self.candidates[first_new..]) + first_new);
            if c.distance < self.candidates[far].distance {
                self.candidates[far] = c;
                farthest = Some(farthest_of(&self.candidates[first_new..]) + first_new);
            }
        }
        if self.dropped > 0 {
            self.capped_queries += 1;
        }

        // 3. Narrow phase.
        let probe_box = probe.compute_aabb(probe_pose);
        let mut tests = 0;
        for c in &self.candidates {
            let Some(collider) = queries.colliders.get(c.handle) else {
                continue;
            };
            let other = collider.compute_aabb();
            let mut touches = |shape: &dyn Shape, pose: &Pose| {
                tests += 1;
                let pos12 = pose.inv_mul(collider.position());
                queries
                    .dispatcher
                    .intersection_test(&pos12, shape, collider.shape())
                    .is_ok_and(|hit| hit.intersecting)
            };
            if probe_box.intersects(&other) && touches(probe, probe_pose) {
                self.next.push(Contact {
                    hit: ROOT_HIT,
                    part: c.part,
                });
            }
            for (i, capsule) in volumes.iter().enumerate() {
                if boxes[i].intersects(&other) && touches(capsule, &identity) {
                    self.next.push(Contact {
                        hit: i as Hit,
                        part: c.part,
                    });
                }
            }
        }
        self.tested_parts = self.candidates.len();
        self.tests = tests;
        self.next.sort_unstable();
    }

    /// Makes the last query's contacts current and calls `started` for each new contact and
    /// `ended` for each one that stopped, both in sorted order.
    pub fn commit(&mut self, mut started: impl FnMut(Contact), mut ended: impl FnMut(Contact)) {
        diff(&self.touching, &self.next, &mut started, &mut ended);
        std::mem::swap(&mut self.touching, &mut self.next);
    }

    /// Ends every contact (the character died or is gone): calls `ended` for each.
    pub fn clear(&mut self, ended: impl FnMut(Contact)) {
        self.world = None;
        self.next.clear();
        self.commit(|_| {}, ended);
    }
}

/// The body volumes and the movement capsule (`probe` at `probe_pose`) in world space.
fn world_volumes(
    volumes: &[Capsule; BODY_PART_COUNT],
    boxes: &[Aabb; BODY_PART_COUNT],
    probe: &Capsule,
    probe_pose: &Pose,
    bounds: Aabb,
) -> WorldVolumes {
    let probe_world = Capsule::new(
        probe_pose * probe.segment.a,
        probe_pose * probe.segment.b,
        probe.radius,
    );
    let probe_box = probe.compute_aabb(probe_pose);
    WorldVolumes {
        shapes: std::array::from_fn(|i| volumes.get(i).copied().unwrap_or(probe_world)),
        boxes: std::array::from_fn(|i| boxes.get(i).copied().unwrap_or(probe_box)),
        bounds,
    }
}

/// Index of the candidate with the largest distance (`list` is not empty).
fn farthest_of(list: &[Candidate]) -> usize {
    let mut best = 0;
    for (i, c) in list.iter().enumerate() {
        if c.distance > list[best].distance {
            best = i;
        }
    }
    best
}

fn finite(b: &Aabb) -> bool {
    let ok = |p: Vector| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
    ok(b.mins) && ok(b.maxs)
}

/// Walks two sorted lists: `started` for entries only in `now`, `ended` for those only in
/// `before`.
fn diff(
    before: &[Contact],
    now: &[Contact],
    started: &mut impl FnMut(Contact),
    ended: &mut impl FnMut(Contact),
) {
    diff_sorted(
        before,
        now,
        |c, start| if start { started(c) } else { ended(c) },
    );
}

/// Calls `changed(x, false)` for what is only in `before` and `changed(x, true)` for what is
/// only in `now` (both sorted, no duplicates), in sorted order.
pub(crate) fn diff_sorted<T: Ord + Copy>(
    before: &[T],
    now: &[T],
    mut changed: impl FnMut(T, bool),
) {
    let (mut i, mut j) = (0, 0);
    while i < before.len() || j < now.len() {
        match (before.get(i), now.get(j)) {
            (Some(a), Some(b)) if a == b => {
                i += 1;
                j += 1;
            }
            (Some(a), Some(b)) if a < b => {
                changed(*a, false);
                i += 1;
            }
            (Some(a), None) => {
                changed(*a, false);
                i += 1;
            }
            (_, Some(b)) => {
                changed(*b, true);
                j += 1;
            }
            (None, None) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).magnitude() < 1e-9
    }

    #[test]
    fn names_round_trip_and_indices_match() {
        for (i, p) in BodyPart::ALL.into_iter().enumerate() {
            assert_eq!(p.index(), i);
            assert_eq!(BodyPart::from_name(p.name()), Some(p));
        }
        assert_eq!(BodyPart::from_name("Torso"), None);
    }

    #[test]
    fn right_side_mirrors_left() {
        let pairs = [
            (BodyPart::LeftUpperArm, BodyPart::RightUpperArm),
            (BodyPart::LeftLowerArm, BodyPart::RightLowerArm),
            (BodyPart::LeftHand, BodyPart::RightHand),
            (BodyPart::LeftUpperLeg, BodyPart::RightUpperLeg),
            (BodyPart::LeftLowerLeg, BodyPart::RightLowerLeg),
            (BodyPart::LeftFoot, BodyPart::RightFoot),
        ];
        let m = |p: Vec3| Vec3::new(-p.x, p.y, p.z);
        for (l, r) in pairs {
            let (la, lb) = l.rest_ends();
            let (ra, rb) = r.rest_ends();
            assert!(close(m(la), ra) && close(m(lb), rb), "{l:?}");
            assert_eq!(l.radius(), r.radius());
            assert!(close(l.size(), r.size()));
        }
    }

    #[test]
    fn feet_rest_on_the_floor_and_head_tops_out_at_avatar_height() {
        for foot in [BodyPart::LeftFoot, BodyPart::RightFoot] {
            let (a, b) = foot.rest_ends();
            let bottom = a.y.min(b.y) - foot.radius();
            assert!((bottom + crate::ROOT_HEIGHT).abs() < 1e-9, "{bottom}");
        }
        let (_, top) = BodyPart::Head.rest_ends();
        // 1.54 m at 0.28 m per stud is 5.5 studs, 2.5 above the root centre.
        assert!((top.y + BodyPart::Head.radius() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn rest_pose_turns_with_the_yaw() {
        let p = BodyPose::rest(90.0);
        // Yaw 90 turns -X (the left side) into +Z.
        let (a, _) = p.world_ends(BodyPart::LeftUpperArm, Vec3::new(1.0, 2.0, 3.0));
        let expect = Vec3::new(1.0, 2.0 + 0.74, 3.0 + 0.44);
        assert!((a - expect).magnitude() < 1e-9, "{a:?}");
        assert!(BodyPose::rest(0.0).is_finite());
    }

    #[test]
    fn diff_reports_started_and_ended_in_order() {
        let mut world = metallic_core::World::new_empty();
        let p = world.create("Part", None).unwrap();
        let c = |hit: Hit| Contact { hit, part: p };
        let before = [c(0), c(2), c(5)];
        let now = [c(1), c(2), c(6)];
        let (mut s, mut e) = (Vec::new(), Vec::new());
        diff(&before, &now, &mut |x| s.push(x.hit), &mut |x| {
            e.push(x.hit)
        });
        assert_eq!(s, [1, 6]);
        assert_eq!(e, [0, 5]);
    }

    #[test]
    fn query_and_commit_do_not_allocate_once_warm() {
        use metallic_core::{Value, World};
        let mut world = World::new_with_services();
        let ws = world.root("Workspace").unwrap();
        let mut add = |pos: Vec3, size: Vec3| {
            let id = world.create("Part", Some(ws)).unwrap();
            world.set(id, "Position", Value::Vector3(pos)).unwrap();
            world.set(id, "Size", Value::Vector3(size)).unwrap();
            world.set(id, "Anchored", Value::Bool(true)).unwrap();
        };
        add(Vec3::new(0.0, -2.0, 0.0), Vec3::new(64.0, 4.0, 64.0));
        for i in 0..40 {
            let f = f64::from(i);
            add(
                Vec3::new((f * 0.7).sin(), 1.0 + f * 0.1, (f * 0.3).cos()),
                Vec3::new(0.3, 0.3, 0.3),
            );
        }
        let mut physics = PhysicsWorld::new();
        physics.sync_from(&world);
        let mut volumes = TouchVolumes::default();
        let probe = Capsule::new_y(1.5, 1.1);
        let frame = |volumes: &mut TouchVolumes, k: usize| {
            // Alternate two poses so contacts start and end every step.
            let pose = BodyPose::rest(if k.is_multiple_of(2) { 0.0 } else { 90.0 });
            let root = Vec3::new(0.0, 3.0, 0.0);
            let probe_pose = Pose::from_translation(to_vector(root - Vec3::new(0.0, 0.48, 0.0)));
            volumes.query(&physics, &|_| false, &probe, &probe_pose, &pose, root);
            let (mut started, mut ended) = (0, 0);
            volumes.commit(|_| started += 1, |_| ended += 1);
            started + ended
        };
        for k in 0..20 {
            frame(&mut volumes, k);
        }
        let (changes, allocations) =
            crate::alloc_count::count(|| (0..600).map(|k| frame(&mut volumes, k)).sum::<usize>());
        assert!(changes > 0, "the poses should change the contacts");
        assert!(!volumes.touching.is_empty());
        assert_eq!(allocations, 0);
    }
}
