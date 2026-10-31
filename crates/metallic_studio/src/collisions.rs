//! The Collisions chip (building-tools spec 3.1): with it on, a drag stops where a moved part
//! would overlap a part that is not moving.
//!
//! Parts are oriented boxes; two boxes overlap when no separating axis exists (the 15-axis test
//! for boxes). Boxes that only touch do not count: each box is shrunk by [`TOUCH`] first, so a part
//! can still sit flush on another one.
//!
//! A [`Guard`] is built once per drag. It puts every part that is not moving into a uniform grid
//! of [`CELL`]-stud cells (a broadphase), so a test only looks at nearby parts and scales to
//! scenes of 10,000 parts and large selections. Parts the selection already overlaps when the drag
//! starts are ignored for the whole drag: a part sunk into its rim can still be nudged out, and an
//! Alt-drag copy can leave its source. If a test would need more than [`MAX_TESTS`] box tests the
//! guard fails safe: it reports the motion as blocked and [`Guard::overloaded`] says why.

use std::collections::{HashMap, HashSet};

use metallic_core::InstanceId;
use metallic_render::glam::{DMat3, DVec3};

use crate::gizmo::GizmoDrag;
use crate::picking::PickCache;
use crate::transform::PartTransform;

/// How far boxes may sink into each other (per side, studs) and still count as touching.
pub const TOUCH: f64 = 0.01;
/// Edge of a broadphase cell in studs.
pub const CELL: f64 = 8.0;
/// A part spanning more cells than this is kept in a short list tested every time.
pub const MAX_PART_CELLS: usize = 4096;
/// Most box tests in one [`Guard::blocked`] call; beyond it the guard fails safe (blocked).
pub const MAX_TESTS: usize = 50_000;

/// True when two oriented boxes overlap by more than [`TOUCH`].
pub fn boxes_overlap(a: &PartTransform, b: &PartTransform) -> bool {
    let ha = (a.size * 0.5 - DVec3::splat(TOUCH)).max(DVec3::ZERO);
    let hb = (b.size * 0.5 - DVec3::splat(TOUCH)).max(DVec3::ZERO);
    let ra = DMat3::from_quat(a.rotation);
    let rb = DMat3::from_quat(b.rotation);
    let axes_a = [ra.x_axis, ra.y_axis, ra.z_axis];
    let axes_b = [rb.x_axis, rb.y_axis, rb.z_axis];
    let d = b.position - a.position;
    let separated = |axis: DVec3| {
        let len = axis.length_squared();
        if len < 1e-12 {
            return false; // parallel edges: covered by the face axes
        }
        let pa: f64 = (0..3).map(|i| ha[i] * axes_a[i].dot(axis).abs()).sum();
        let pb: f64 = (0..3).map(|i| hb[i] * axes_b[i].dot(axis).abs()).sum();
        d.dot(axis).abs() >= pa + pb
    };
    for axis in axes_a.iter().chain(axes_b.iter()) {
        if separated(*axis) {
            return false;
        }
    }
    for x in &axes_a {
        for y in &axes_b {
            if separated(x.cross(*y)) {
                return false;
            }
        }
    }
    true
}

/// The world-axis box around a part.
fn bounds(t: &PartTransform) -> (DVec3, DVec3) {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    for c in t.corners() {
        lo = lo.min(c);
        hi = hi.max(c);
    }
    (lo, hi)
}

type Cell = (i64, i64, i64);

fn cell_range(lo: DVec3, hi: DVec3) -> (Cell, Cell, usize) {
    let a = (lo / CELL).floor();
    let b = (hi / CELL).floor();
    let n = ((b.x - a.x + 1.0) * (b.y - a.y + 1.0) * (b.z - a.z + 1.0)).max(0.0);
    let count = if n.is_finite() && n < usize::MAX as f64 {
        n as usize
    } else {
        usize::MAX
    };
    (
        (a.x as i64, a.y as i64, a.z as i64),
        (b.x as i64, b.y as i64, b.z as i64),
        count,
    )
}

struct Other {
    transform: PartTransform,
    lo: DVec3,
    hi: DVec3,
}

/// The parts one drag must not run into, with their broadphase grid.
pub struct Guard {
    serial: u64,
    others: Vec<Other>,
    cells: HashMap<Cell, Vec<u32>>,
    large: Vec<u32>,
    /// Per part: the query that last looked at it (dedupes parts found in several cells).
    stamp: Vec<u32>,
    query: u32,
    overloaded: bool,
}

impl Guard {
    /// The guard for `drag`: every part in `picks` that the drag does not move, minus the parts
    /// its selection already overlaps at the start.
    pub fn for_drag(drag: &GizmoDrag, picks: &PickCache) -> Guard {
        let mut starts = Vec::new();
        drag.starts(&mut starts);
        Guard::new(
            drag.serial(),
            picks.iter().map(|(id, t, _)| (id, *t)),
            drag.moving(),
            &starts,
        )
    }

    /// The guard from `parts` (id and transform), leaving out `moving` and whatever `starts` (the
    /// moved parts at the start) already overlap. `serial` names the drag.
    pub fn new(
        serial: u64,
        parts: impl Iterator<Item = (InstanceId, PartTransform)>,
        moving: &HashSet<InstanceId>,
        starts: &[PartTransform],
    ) -> Guard {
        let mut guard = Guard {
            serial,
            others: Vec::new(),
            cells: HashMap::new(),
            large: Vec::new(),
            stamp: Vec::new(),
            query: 0,
            overloaded: false,
        };
        for (id, transform) in parts {
            if moving.contains(&id) {
                continue;
            }
            let (lo, hi) = bounds(&transform);
            if !(lo.is_finite() && hi.is_finite()) {
                continue;
            }
            let index = guard.others.len() as u32;
            guard.others.push(Other { transform, lo, hi });
            let (a, b, count) = cell_range(lo, hi);
            if count > MAX_PART_CELLS {
                guard.large.push(index);
                continue;
            }
            for x in a.0..=b.0 {
                for y in a.1..=b.1 {
                    for z in a.2..=b.2 {
                        guard.cells.entry((x, y, z)).or_default().push(index);
                    }
                }
            }
        }
        guard.stamp = vec![0; guard.others.len()];
        // Overlaps that exist when the drag starts are not the drag's doing: ignore those parts.
        let mut ignore: HashSet<u32> = HashSet::new();
        for s in starts {
            guard.visit(s, |i, other| {
                if boxes_overlap(s, &other.transform) {
                    ignore.insert(i);
                }
                false
            });
        }
        if !ignore.is_empty() {
            for v in guard.cells.values_mut() {
                v.retain(|i| !ignore.contains(i));
            }
            guard.large.retain(|i| !ignore.contains(i));
        }
        guard.overloaded = false;
        guard
    }

    /// The drag this guard was built for.
    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// True when the last [`Guard::blocked`] call hit [`MAX_TESTS`] and failed safe.
    pub fn overloaded(&self) -> bool {
        self.overloaded
    }

    /// True when any of `moved` overlaps a guarded part (or the check was too big: fail safe).
    pub fn blocked(&mut self, moved: &[PartTransform]) -> bool {
        self.overloaded = false;
        let mut tests = 0usize;
        for m in moved {
            let mut over = false;
            let hit = self.visit(m, |_, other| {
                tests += 1;
                if tests > MAX_TESTS {
                    over = true;
                    return true;
                }
                boxes_overlap(m, &other.transform)
            });
            if over {
                self.overloaded = true;
                return true;
            }
            if hit {
                return true;
            }
        }
        false
    }

    /// Calls `f` once for each guarded part whose box meets `t`'s box, until it returns true.
    /// Returns true when `f` did.
    fn visit(&mut self, t: &PartTransform, mut f: impl FnMut(u32, &Other) -> bool) -> bool {
        let (lo, hi) = bounds(t);
        if !(lo.is_finite() && hi.is_finite()) {
            return false;
        }
        self.query = self.query.wrapping_add(1);
        if self.query == 0 {
            self.stamp.fill(0);
            self.query = 1;
        }
        let q = self.query;
        let mut check = |i: u32, others: &[Other], stamp: &mut [u32]| -> bool {
            let slot = &mut stamp[i as usize];
            if *slot == q {
                return false;
            }
            *slot = q;
            let o = &others[i as usize];
            let apart = o.lo.cmpgt(hi).any() || o.hi.cmplt(lo).any();
            !apart && f(i, o)
        };
        for &i in &self.large {
            if check(i, &self.others, &mut self.stamp) {
                return true;
            }
        }
        let (a, b, count) = cell_range(lo, hi);
        if count > MAX_PART_CELLS {
            // A huge moved part: walk the parts instead of the cells.
            for i in 0..self.others.len() as u32 {
                if check(i, &self.others, &mut self.stamp) {
                    return true;
                }
            }
            return false;
        }
        for x in a.0..=b.0 {
            for y in a.1..=b.1 {
                for z in a.2..=b.2 {
                    if let Some(list) = self.cells.get(&(x, y, z)) {
                        for &i in list {
                            if check(i, &self.others, &mut self.stamp) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::World;
    use metallic_render::glam::DQuat;

    fn cube(x: f64, y: f64, size: f64, turn: f64) -> PartTransform {
        PartTransform {
            position: DVec3::new(x, y, 0.0),
            rotation: DQuat::from_rotation_y(turn.to_radians()),
            size: DVec3::splat(size),
        }
    }

    fn ids(n: usize) -> Vec<InstanceId> {
        let mut w = World::new_with_services();
        (0..n)
            .map(|_| w.create("Part", None).expect("part"))
            .collect()
    }

    #[test]
    fn touching_is_allowed_and_overlap_is_not() {
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(2.0, 0.0, 2.0, 0.0)
        ));
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(0.0, 2.0, 2.0, 0.0)
        ));
        assert!(boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(1.5, 0.0, 2.0, 0.0)
        ));
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(5.0, 0.0, 2.0, 0.0)
        ));
    }

    #[test]
    fn turned_boxes_use_their_real_shape() {
        // A cube turned 45 degrees reaches 1.414 along X: it hits a box 2.3 away, an
        // axis-aligned one would not.
        assert!(boxes_overlap(
            &cube(0.0, 0.0, 2.0, 45.0),
            &cube(2.3, 0.0, 2.0, 0.0)
        ));
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(2.3, 0.0, 2.0, 0.0)
        ));
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 45.0),
            &cube(2.5, 0.0, 2.0, 0.0)
        ));
    }

    #[test]
    fn overlaps_present_at_the_start_are_ignored() {
        // A slab sunk into its rim (and an Alt-copy on its source) can still move out.
        let id = ids(3);
        let moving: HashSet<_> = [id[0]].into();
        let start = cube(0.0, 0.0, 2.0, 0.0);
        let rim = (id[1], cube(0.5, 0.0, 2.0, 0.0));
        let wall = (id[2], cube(6.0, 0.0, 2.0, 0.0));
        let mut g = Guard::new(1, [rim, wall].into_iter(), &moving, &[start]);
        assert!(!g.blocked(&[cube(-1.0, 0.0, 2.0, 0.0)]), "leaving the rim");
        assert!(!g.blocked(&[cube(1.0, 0.0, 2.0, 0.0)]), "still ignored");
        assert!(
            g.blocked(&[cube(5.0, 0.0, 2.0, 0.0)]),
            "the wall still blocks"
        );
    }

    #[test]
    fn ten_thousand_parts_stay_cheap_and_correct() {
        // A 100 x 100 floor of 4-stud tiles with a 4-stud gap: only neighbours are tested.
        let n = 100;
        let id = ids(n * n + 1);
        let parts = (0..n * n).map(|k| {
            let (i, j) = ((k % n) as f64, (k / n) as f64);
            (
                id[k],
                PartTransform {
                    position: DVec3::new(i * 8.0, 0.0, j * 8.0),
                    rotation: DQuat::IDENTITY,
                    size: DVec3::splat(4.0),
                },
            )
        });
        let moving: HashSet<_> = [id[n * n]].into();
        let start = PartTransform {
            position: DVec3::new(4.0, 0.0, 4.0),
            rotation: DQuat::IDENTITY,
            size: DVec3::splat(2.0),
        };
        let mut g = Guard::new(1, parts, &moving, &[start]);
        assert!(!g.blocked(&[start]));
        assert!(!g.overloaded());
        let onto = PartTransform {
            position: DVec3::new(8.0, 0.0, 8.0),
            ..start
        };
        assert!(g.blocked(&[onto]));
        // A multi-selection of 200 parts in the gaps is still checked in full.
        let many: Vec<_> = (0..200)
            .map(|k| PartTransform {
                position: DVec3::new(
                    4.0 + 8.0 * (k % 20) as f64,
                    0.0,
                    4.0 + 8.0 * (k / 20) as f64,
                ),
                ..start
            })
            .collect();
        assert!(!g.blocked(&many));
        assert!(!g.overloaded());
    }

    #[test]
    fn too_much_work_fails_safe() {
        // 60,000 parts piled in one place: a test there would exceed the cap, so it blocks.
        let count = MAX_TESTS + 10_000;
        let id = ids(count + 1);
        let parts = (0..count).map(|k| (id[k], cube(10.0, 0.0, 1.0, 0.0)));
        let moving: HashSet<_> = [id[count]].into();
        let mut g = Guard::new(1, parts, &moving, &[cube(-10.0, 0.0, 1.0, 0.0)]);
        // Flush against the pile: every part there must be tested, none overlaps.
        assert!(g.blocked(&[cube(11.0, 0.0, 1.0, 0.0)]));
        assert!(g.overloaded());
    }
}
