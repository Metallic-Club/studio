//! What a click in the viewport selects (#354): the nearest part the pointer's ray meets, each part taken as its box
//! (every shape, for now), and of that part what is selectable: the outermost model it is in under Workspace, as in
//! Roblox Studio, or the part. The outlines are these boxes too: a part's own, a model's around its parts.

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
    pub fn hit(&self, (origin, dir): Ray) -> Option<f32> {
        let inverse = self.rot.inverse();
        let (o, d) = (inverse * (origin - self.center), inverse * dir);
        let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
        for axis in 0..3 {
            if d[axis].abs() < 1e-9 {
                if o[axis].abs() > self.half[axis] {
                    return None;
                }
                continue;
            }
            let a = (-self.half[axis] - o[axis]) / d[axis];
            let b = (self.half[axis] - o[axis]) / d[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        (near <= far && far >= 0.0).then_some(near.max(0.0))
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

/// The box an instance's outline is: a part's own, else the world-aligned box round the parts inside it.
pub fn outline_box(dm: &DataModel, id: Id) -> Option<Bounds> {
    if let Some(own) = part_box(dm, id) {
        return Some(own);
    }
    let mut span: Option<(Vec3, Vec3)> = None;
    for part in dm.descendants(id).ok()?.filter_map(|inside| part_box(dm, inside)) {
        for corner in part.corners() {
            span = Some(span.map_or((corner, corner), |(min, max)| (min.min(corner), max.max(corner))));
        }
    }
    let (min, max) = span?;
    Some(Bounds {
        center: (min + max) * 0.5,
        half: (max - min) * 0.5,
        rot: Quat::IDENTITY,
    })
}

/// The Workspace, where everything the viewport shows is.
pub fn workspace(dm: &DataModel) -> Option<Id> {
    dm.registry().class_id("Workspace").and_then(|class| dm.service(class))
}

/// The nearest part `ray` meets in Workspace, and how far along it.
pub fn nearest_part(dm: &DataModel, ray: Ray) -> Option<(Id, f32)> {
    let world = workspace(dm)?;
    dm.descendants(world)
        .ok()?
        .filter_map(|id| part_box(dm, id).and_then(|bounds| bounds.hit(ray)).map(|t| (id, t)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// What a click on `part` selects: the outermost model it is in under Workspace, or the part itself.
pub fn selectable(dm: &DataModel, part: Id) -> Id {
    let world = workspace(dm);
    let mut chosen = part;
    let mut at = dm.parent(part).ok().flatten();
    while let Some(parent) = at.filter(|&parent| Some(parent) != world) {
        if is_a(dm, parent, "Model") {
            chosen = parent;
        }
        at = dm.parent(parent).ok().flatten();
    }
    chosen
}

/// What a click along `ray` selects, if anything.
pub fn pick(dm: &DataModel, ray: Ray) -> Option<Id> {
    nearest_part(dm, ray).map(|(part, _)| selectable(dm, part))
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
}
