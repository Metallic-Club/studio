//! What a click in the viewport selects (#354): the nearest part the pointer's ray meets, each by its actual shape
//! (#494, [`super::shapes`]), and of that part what is selectable: the outermost model it is in under Workspace, as in
//! Roblox Studio, or the part. The outlines are these boxes too: a part's own, a model's around its parts.

use metallic_engine::metallic_classes::base::world_object;
use metallic_engine::metallic_core::glam::{Quat, Vec3};
use metallic_engine::metallic_core::{DataModel, Id, Value};

use super::view::Ray;

/// A box in the world: where its centre is, half its size along its own axes, and how it is turned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// Its centre.
    pub center: Vec3,
    /// Half its size along each of its axes.
    pub half: Vec3,
    /// How it is turned.
    pub rot: Quat,
}

impl Bounds {
    /// Its eight corners, in the order [`EDGES`] names them.
    pub fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|i| {
            let sign = |bit: usize| if i & bit == 0 { -1.0 } else { 1.0 };
            self.center + self.rot * (self.half * Vec3::new(sign(1), sign(2), sign(4)))
        })
    }

    /// The centre of its face on the `sign` side (+1 or -1) of its axis `axis`.
    pub fn face(&self, axis: usize, sign: f32) -> Vec3 {
        let mut offset = Vec3::ZERO;
        offset[axis] = sign * self.half[axis];
        self.center + self.rot * offset
    }

    /// The outward direction of that face.
    pub fn outward(&self, axis: usize, sign: f32) -> Vec3 {
        let mut dir = Vec3::ZERO;
        dir[axis] = sign;
        (self.rot * dir).normalize()
    }

    /// Where `ray` first meets it (the distance along it), if it does: 0 from inside.
    pub fn hit(&self, ray: Ray) -> Option<f32> {
        self.hit_face(ray).map(|(t, _)| t)
    }

    /// Where `ray` first meets it, and the outward normal of the face it meets there (facing the ray from inside).
    pub fn hit_face(&self, (origin, dir): Ray) -> Option<(f32, Vec3)> {
        let inverse = self.rot.inverse();
        let (o, d) = (inverse * (origin - self.center), inverse * dir);
        let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
        let mut face = Vec3::ZERO;
        for axis in 0..3 {
            if d[axis].abs() < 1e-9 {
                if o[axis].abs() > self.half[axis] {
                    return None;
                }
                continue;
            }
            let a = (-self.half[axis] - o[axis]) / d[axis];
            let b = (self.half[axis] - o[axis]) / d[axis];
            if a.min(b) > near {
                near = a.min(b);
                face = Vec3::ZERO;
                face[axis] = -d[axis].signum();
            }
            far = far.min(a.max(b));
        }
        (near <= far && far >= 0.0).then(|| (near.max(0.0), self.rot * face))
    }
}

/// The twelve edges of a box, as pairs of [`Bounds::corners`].
pub const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// Whether `id` is a `class` (or inherits from it).
pub fn is_a(dm: &DataModel, id: Id, class: &str) -> bool {
    dm.registry()
        .class_id(class)
        .is_some_and(|class| dm.is_a(id, class).unwrap_or(false))
}

/// A part's box: its `CFrame` and `Size`. `None` for anything that is not a part.
pub fn part_box(dm: &DataModel, id: Id) -> Option<Bounds> {
    if !is_a(dm, id, "BasePart") {
        return None;
    }
    let (Ok(Value::CFrame(frame)), Ok(Value::Vector3(size))) =
        (dm.get_by_name(id, "CFrame"), dm.get_by_name(id, "Size"))
    else {
        return None;
    };
    Some(Bounds {
        center: frame.position(),
        half: size * 0.5,
        rot: frame.rotation(),
    })
}

/// The box an instance's outline is: round it and the parts inside it, along its axes (#438: a part's own; #413: a
/// model's pivot's, turning with it), or the world's when it has none. A part with none inside is its own box.
pub fn outline_box(dm: &DataModel, id: Id) -> Option<Bounds> {
    let own = part_box(dm, id);
    let mut inside = dm
        .descendants(id)
        .ok()?
        .filter_map(|inside| part_box(dm, inside))
        .peekable();
    if inside.peek().is_none() {
        return own;
    }
    let rot = match own {
        Some(own) => own.rot,
        None => world_object::pivot(dm, id).map_or(Quat::IDENTITY, |pivot| pivot.rotation()),
    };
    let back = rot.inverse();
    let mut span: Option<(Vec3, Vec3)> = None;
    for part in own.into_iter().chain(inside) {
        for corner in part.corners().map(|corner| back * corner) {
            span = Some(span.map_or((corner, corner), |(min, max)| (min.min(corner), max.max(corner))));
        }
    }
    let (min, max) = span?;
    Some(Bounds {
        center: rot * ((min + max) * 0.5),
        half: (max - min) * 0.5,
        rot,
    })
}

/// The Workspace, where everything the viewport shows is.
pub fn workspace(dm: &DataModel) -> Option<Id> {
    dm.registry().class_id("Workspace").and_then(|class| dm.service(class))
}

/// Where `ray` first meets part `id`'s actual shape (#494, [`super::shapes`]): its box first, the shape only when that
/// is met.
pub fn part_hit(dm: &DataModel, id: Id, ray: Ray) -> Option<f32> {
    let bounds = part_box(dm, id)?;
    bounds.hit(ray)?;
    super::shapes::hit(&bounds, &super::shapes::shape_of(dm, id), ray)
}

/// The nearest part `ray` meets in Workspace, by its actual shape (#494), and how far along it.
pub fn nearest_part(dm: &DataModel, ray: Ray) -> Option<(Id, f32)> {
    let world = workspace(dm)?;
    dm.descendants(world)
        .ok()?
        .filter_map(|id| part_hit(dm, id, ray).map(|t| (id, t)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// Whether a part in Workspace stands between `from` and `to` (#571), so that `to` cannot be seen from `from`. The
/// parts `holder` is inside (a light in its lamp) do not count, nor does a wholly transparent part.
pub fn blocked(dm: &DataModel, from: Vec3, to: Vec3, holder: Id) -> bool {
    let distance = (to - from).length();
    let Some(direction) = (to - from).try_normalize() else {
        return false;
    };
    let Some(world) = workspace(dm) else {
        return false;
    };
    let Ok(all) = dm.descendants(world) else {
        return false;
    };
    all.filter(|&id| id != holder)
        .filter(|&id| part_hit(dm, id, (from, direction)).is_some_and(|t| t < distance - 1e-3))
        .filter(|&id| !matches!(dm.get_by_name(id, "Transparency"), Ok(Value::Number(t)) if t >= 1.0))
        .any(|id| !dm.is_descendant_of(holder, id).unwrap_or(false))
}

/// The nearest part's face `ray` meets in Workspace: how far along, and the face's outward normal.
pub fn nearest_face(dm: &DataModel, ray: Ray) -> Option<(f32, Vec3)> {
    let world = workspace(dm)?;
    dm.descendants(world)
        .ok()?
        .filter_map(|id| part_box(dm, id).and_then(|bounds| bounds.hit_face(ray)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// What a click on `part` selects: the outermost model it is in under Workspace; else the outermost part it is in
/// (a part inside another part is a group with it, #493); else the part itself.
pub fn selectable(dm: &DataModel, part: Id) -> Id {
    let world = workspace(dm);
    let (mut model, mut outer_part) = (None, part);
    let mut at = dm.parent(part).ok().flatten();
    while let Some(parent) = at.filter(|&parent| Some(parent) != world) {
        if is_a(dm, parent, "Model") {
            model = Some(parent);
        } else if is_a(dm, parent, "BasePart") {
            outer_part = parent;
        }
        at = dm.parent(parent).ok().flatten();
    }
    model.unwrap_or(outer_part)
}

/// What a click along `ray` selects, if anything.
pub fn pick(dm: &DataModel, ray: Ray) -> Option<Id> {
    nearest_part(dm, ray).map(|(part, _)| selectable(dm, part))
}

/// Every part `ray` meets in Workspace, nearest first: what an Alt+click walks through (#482).
pub fn parts_along(dm: &DataModel, ray: Ray) -> Vec<Id> {
    let Some(world) = workspace(dm) else {
        return Vec::new();
    };
    let Ok(descendants) = dm.descendants(world) else {
        return Vec::new();
    };
    let mut hits: Vec<(Id, f32)> = descendants
        .filter_map(|id| part_hit(dm, id, ray).map(|t| (id, t)))
        .collect();
    hits.sort_by(|a, b| a.1.total_cmp(&b.1));
    hits.into_iter().map(|(id, _)| id).collect()
}

/// How far, in points, the pointer may move between Alt+clicks that still walk on behind (#482).
pub const CYCLE_TOLERANCE: f32 = 4.0;

/// Alt+clicking one spot again and again walks through the parts behind one another there (#482), after
/// Forge-Engine's `PickCycle`.
#[derive(Clone, Debug, Default)]
pub struct PickCycle {
    /// Where the walk began, in points.
    at: [f32; 2],
    /// What was under the pointer then, nearest first.
    ids: Vec<Id>,
    /// Which of them was chosen last.
    next: usize,
}

impl PickCycle {
    /// The part an Alt+click at `at` chooses, given the parts under it nearest first (`hits`): the nearest the first
    /// time; then, clicked again within `tolerance` points with the same parts under it, the next one behind, round
    /// to the nearest after the last. Elsewhere, or over other parts, it starts again; over nothing it chooses nothing.
    pub fn advance(&mut self, at: [f32; 2], hits: &[Id], tolerance: f32) -> Option<Id> {
        if hits.is_empty() {
            self.reset();
            return None;
        }
        let moved = (at[0] - self.at[0]).hypot(at[1] - self.at[1]);
        if !self.ids.is_empty() && moved <= tolerance && self.ids == hits {
            self.next = (self.next + 1) % hits.len();
        } else {
            self.ids = hits.to_vec();
            self.at = at;
            self.next = 0;
        }
        hits.get(self.next).copied()
    }

    /// Forgets the walk: the next Alt+click picks the nearest part again.
    pub fn reset(&mut self) {
        self.ids.clear();
        self.next = 0;
    }
}

#[cfg(test)]
mod tests {
    use metallic_engine::metallic_core::CFrame;

    use super::*;

    fn part_at(dm: &mut DataModel, parent: Id, at: Vec3, size: Vec3) -> Id {
        let part = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
        dm.set_by_name(part, "CFrame", Value::CFrame(CFrame::new(at))).unwrap();
        dm.set_by_name(part, "Size", Value::Vector3(size)).unwrap();
        dm.set_parent(part, Some(parent)).unwrap();
        part
    }

    #[test]
    fn a_ray_meets_turned_boxes_and_misses_beside_them() {
        let turned = Bounds {
            center: Vec3::ZERO,
            half: Vec3::new(4.0, 1.0, 1.0),
            rot: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        };
        // Turned a quarter, the long side runs along Z.
        let t = turned.hit((Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z)).unwrap();
        assert!((t - 6.0).abs() < 1e-4, "{t}");
        assert_eq!(turned.hit((Vec3::new(3.0, 0.0, 10.0), Vec3::NEG_Z)), None);
        assert_eq!(turned.hit((Vec3::ZERO, Vec3::X)), Some(0.0), "from inside");
        assert_eq!(turned.hit((Vec3::new(0.0, 0.0, 10.0), Vec3::Z)), None, "behind the ray");
        assert_eq!(turned.corners().len(), 8);
    }

    #[test]
    fn alt_clicks_walk_through_what_is_behind_and_wrap() {
        let (a, b, c) = (Id::new(1, 0), Id::new(2, 0), Id::new(3, 0));
        let mut cycle = PickCycle::default();
        let at = [10.0, 10.0];
        assert_eq!(cycle.advance(at, &[a, b, c], CYCLE_TOLERANCE), Some(a));
        assert_eq!(cycle.advance([12.0, 11.0], &[a, b, c], CYCLE_TOLERANCE), Some(b));
        assert_eq!(cycle.advance(at, &[a, b, c], CYCLE_TOLERANCE), Some(c));
        assert_eq!(cycle.advance(at, &[a, b, c], CYCLE_TOLERANCE), Some(a), "round again");
        // Somewhere else, or over other parts, it starts over.
        assert_eq!(cycle.advance([40.0, 10.0], &[a, b, c], CYCLE_TOLERANCE), Some(a));
        assert_eq!(cycle.advance([40.0, 10.0], &[b, c], CYCLE_TOLERANCE), Some(b));
        // Over nothing it chooses nothing, and the next walk begins at the nearest.
        assert_eq!(cycle.advance([40.0, 10.0], &[], CYCLE_TOLERANCE), None);
        assert_eq!(cycle.advance([40.0, 10.0], &[b, c], CYCLE_TOLERANCE), Some(b));
    }

    #[test]
    fn a_click_picks_the_nearest_part_and_selects_its_outermost_model() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let world = workspace(&dm).unwrap();
        let near = part_at(&mut dm, world, Vec3::new(0.0, 0.0, 5.0), Vec3::ONE);
        let far = part_at(&mut dm, world, Vec3::ZERO, Vec3::ONE);
        let ray = (Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        assert_eq!(pick(&dm, ray), Some(near));
        assert_eq!(pick(&dm, (Vec3::new(9.0, 0.0, 20.0), Vec3::NEG_Z)), None);

        let model_class = dm.registry().class_id("Model").unwrap();
        let outer = dm.create(model_class).unwrap();
        let inner = dm.create(model_class).unwrap();
        dm.set_parent(outer, Some(world)).unwrap();
        dm.set_parent(inner, Some(outer)).unwrap();
        dm.set_parent(near, Some(inner)).unwrap();
        assert_eq!(pick(&dm, ray), Some(outer), "the outermost model");
        // A model's outline is the box round its parts.
        dm.set_parent(far, Some(inner)).unwrap();
        let outline = outline_box(&dm, outer).unwrap();
        assert!((outline.center - Vec3::new(0.0, 0.0, 2.5)).length() < 1e-5);
        assert!((outline.half - Vec3::new(0.5, 0.5, 3.0)).length() < 1e-5);
        let folder = dm.create(dm.registry().class_id("Folder").unwrap()).unwrap();
        assert!(outline_box(&dm, folder).is_none());
    }

    /// #493: a part inside another part is a group with it; a model round them still wins.
    #[test]
    fn a_click_on_a_part_inside_a_part_selects_the_outermost_part() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let world = workspace(&dm).unwrap();
        let base = part_at(&mut dm, world, Vec3::ZERO, Vec3::ONE);
        let child = part_at(&mut dm, base, Vec3::new(3.0, 0.0, 0.0), Vec3::ONE);
        let grandchild = part_at(&mut dm, child, Vec3::new(6.0, 0.0, 0.0), Vec3::ONE);
        // Through a folder inside a part too.
        let folder = dm.create(dm.registry().class_id("Folder").unwrap()).unwrap();
        dm.set_parent(folder, Some(child)).unwrap();
        let foldered = part_at(&mut dm, folder, Vec3::new(9.0, 0.0, 0.0), Vec3::ONE);
        for part in [base, child, grandchild, foldered] {
            assert_eq!(selectable(&dm, part), base, "{part:?}");
        }
        let at = |x: f32| (Vec3::new(x, 0.0, 20.0), Vec3::NEG_Z);
        assert_eq!(pick(&dm, at(6.0)), Some(base));
        // Alt+click's walk still meets the part itself (#482).
        assert_eq!(parts_along(&dm, at(6.0)), [grandchild]);
        let model = dm.create(dm.registry().class_id("Model").unwrap()).unwrap();
        dm.set_parent(model, Some(world)).unwrap();
        dm.set_parent(base, Some(model)).unwrap();
        assert_eq!(selectable(&dm, grandchild), model, "a model round them wins");
    }
}
