//! Pivots, bounding boxes, resizing and model scale (Roblox's `PVInstance`, `BasePart:Resize`,
//! `Model:ScaleTo` and `Model:GetBoundingBox`). Luau and the Studio building tools share these.
//!
//! - A part's frame (`CFrame`) is its `Position` and `Orientation` (degrees, applied Y then X
//!   then Z). Its pivot is `CFrame * PivotOffset`.
//! - A Model's pivot is its `PrimaryPart`'s pivot when that is a BasePart inside the model;
//!   otherwise `WorldPivot` once it has been set; before that, the centre of the bounding box
//!   (axes aligned with the world).
//! - The parts of a Model are all its BasePart descendants, through Folders and nested Models.
use crate::cframe::{CFrame, cross, dot};
use crate::value::{Value, Vec3};
use crate::{CoreError, InstanceId, World};

/// Smallest size of a part on each axis (Roblox's minimum part size).
pub const MIN_PART_SIZE: f64 = 0.001;

/// Largest size of a part on each axis (Roblox's maximum part size).
pub const MAX_PART_SIZE: f64 = 2048.0;

/// A part size with each axis clamped to `MIN_PART_SIZE..=MAX_PART_SIZE`, as Roblox clamps
/// `BasePart.Size` on assignment.
pub fn clamp_part_size(size: Vec3) -> Vec3 {
    let c = |v: f64| v.clamp(MIN_PART_SIZE, MAX_PART_SIZE);
    Vec3::new(c(size.x), c(size.y), c(size.z))
}

fn size_in_limits(size: Vec3) -> bool {
    [size.x, size.y, size.z]
        .iter()
        .all(|v| (MIN_PART_SIZE..=MAX_PART_SIZE).contains(v))
}

/// `cf` with its rotation made orthonormal (Gram-Schmidt on the right and up axes), as Roblox
/// does with the CFrames it stores. Errors when the rotation is degenerate or not finite.
pub fn orthonormal(cf: CFrame, property: &str) -> Result<CFrame, CoreError> {
    if !cf.is_finite() {
        return Err(wrong(property, "all numeric components must be finite"));
    }
    let right = cf.right_vector();
    let up = cf.up_vector();
    let rm = right.magnitude();
    let degenerate = || wrong(property, "the rotation is degenerate");
    if rm < 1e-9 {
        return Err(degenerate());
    }
    let rest = up - right * (dot(up, right) / (rm * rm));
    if rest.magnitude() < 1e-9 * up.magnitude().max(1.0) {
        return Err(degenerate());
    }
    let out = cf.orthonormalize();
    if out.is_finite() {
        Ok(out)
    } else {
        Err(degenerate())
    }
}

/// Property writes planned by an operation. They are all checked before any is applied, so a
/// failing operation changes nothing.
#[derive(Default)]
struct Plan(Vec<(InstanceId, &'static str, Value)>);

impl Plan {
    fn push(&mut self, id: InstanceId, prop: &'static str, value: Value) {
        self.0.push((id, prop, value));
    }

    /// The writes that put a part at `cf` (orthonormal).
    fn part_cframe(&mut self, id: InstanceId, cf: CFrame) {
        self.push(id, "Position", Value::Vector3(cf.position));
        self.push(id, "Orientation", Value::Vector3(cf.orientation_degrees()));
    }

    fn cframe(&mut self, id: InstanceId, prop: &'static str, cf: CFrame) {
        self.push(id, prop, Value::CFrame(Box::new(cf)));
    }

    /// Errors (naming `what`) unless every planned value is finite; then applies them all.
    fn apply(self, world: &mut World, what: &str) -> Result<(), CoreError> {
        if let Some((_, prop, _)) = self.0.iter().find(|(_, _, v)| !v.has_finite_components()) {
            return Err(wrong(
                prop,
                &format!("{what} would make it too large to represent; nothing was changed"),
            ));
        }
        for (id, prop, value) in self.0 {
            world.set(id, prop, value)?;
        }
        Ok(())
    }
}

/// Overlap below this depth (studs) counts as touching, not colliding, in [`resize`].
const TOUCH_TOLERANCE: f64 = 1e-4;

/// A face of a part (Roblox `Enum.NormalId`), in Roblox's item order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalId {
    /// +X.
    Right,
    /// +Y.
    Top,
    /// +Z.
    Back,
    /// -X.
    Left,
    /// -Y.
    Bottom,
    /// -Z.
    Front,
}

impl NormalId {
    /// Every face, in the item order of `Enum.NormalId` (values 0 to 5).
    pub const ALL: [NormalId; 6] = [
        NormalId::Right,
        NormalId::Top,
        NormalId::Back,
        NormalId::Left,
        NormalId::Bottom,
        NormalId::Front,
    ];

    /// The `Enum.NormalId` item name.
    pub const fn name(self) -> &'static str {
        match self {
            NormalId::Right => "Right",
            NormalId::Top => "Top",
            NormalId::Back => "Back",
            NormalId::Left => "Left",
            NormalId::Bottom => "Bottom",
            NormalId::Front => "Front",
        }
    }

    /// Local axis (0 = X, 1 = Y, 2 = Z) and direction (+1 or -1) of the face normal.
    pub const fn axis(self) -> (usize, f64) {
        match self {
            NormalId::Right => (0, 1.0),
            NormalId::Top => (1, 1.0),
            NormalId::Back => (2, 1.0),
            NormalId::Left => (0, -1.0),
            NormalId::Bottom => (1, -1.0),
            NormalId::Front => (2, -1.0),
        }
    }
}

fn wrong(property: &str, reason: &str) -> CoreError {
    CoreError::InvalidValue {
        property: property.into(),
        reason: reason.into(),
    }
}

fn vec_prop(world: &World, id: InstanceId, prop: &str) -> Result<Vec3, CoreError> {
    match world.get_ref(id, prop)? {
        Value::Vector3(v) => Ok(*v),
        _ => Err(wrong(prop, "expected a Vector3")),
    }
}

fn cframe_prop(world: &World, id: InstanceId, prop: &str) -> Result<CFrame, CoreError> {
    match world.get_ref(id, prop)? {
        Value::CFrame(cf) => Ok(**cf),
        _ => Err(wrong(prop, "expected a CFrame")),
    }
}

fn set_cframe_prop(
    world: &mut World,
    id: InstanceId,
    prop: &str,
    cf: CFrame,
) -> Result<(), CoreError> {
    world.set(id, prop, Value::CFrame(Box::new(cf)))
}

fn axis(v: Vec3, i: usize) -> f64 {
    [v.x, v.y, v.z][i]
}

fn with_axis(v: Vec3, i: usize, value: f64) -> Vec3 {
    let mut a = [v.x, v.y, v.z];
    a[i] = value;
    Vec3::new(a[0], a[1], a[2])
}

/// All descendants of `id`, parents first, in tree order.
fn descendants(world: &World, id: InstanceId) -> Vec<InstanceId> {
    let mut out = Vec::new();
    let mut stack: Vec<InstanceId> = world.children(id).iter().rev().copied().collect();
    while let Some(cur) = stack.pop() {
        out.push(cur);
        stack.extend(world.children(cur).iter().rev().copied());
    }
    out
}

fn is_descendant(world: &World, id: InstanceId, ancestor: InstanceId) -> bool {
    let mut cur = world.parent(id);
    while let Some(p) = cur {
        if p == ancestor {
            return true;
        }
        cur = world.parent(p);
    }
    false
}

/// `BasePart.CFrame`: the part's `Position` and `Orientation` as one frame.
pub fn part_cframe(world: &World, id: InstanceId) -> Result<CFrame, CoreError> {
    Ok(CFrame::from_position_orientation(
        vec_prop(world, id, "Position")?,
        vec_prop(world, id, "Orientation")?,
    ))
}

/// Writes `BasePart.CFrame` as `Position` and `Orientation`. The rotation is made orthonormal
/// first; nothing changes if it is degenerate or not finite.
pub fn set_part_cframe(world: &mut World, id: InstanceId, cf: CFrame) -> Result<(), CoreError> {
    let cf = orthonormal(cf, "CFrame")?;
    let mut plan = Plan::default();
    plan.part_cframe(id, cf);
    plan.apply(world, "CFrame")
}

/// The model's `PrimaryPart` when it is a live BasePart inside the model.
pub fn primary_part(world: &World, model: InstanceId) -> Option<InstanceId> {
    match world.get_ref(model, "PrimaryPart") {
        Ok(Value::Ref(Some(p)))
            if world.is_a(*p, "BasePart") && is_descendant(world, *p, model) =>
        {
            Some(*p)
        }
        _ => None,
    }
}

fn world_pivot_set(world: &World, model: InstanceId) -> bool {
    matches!(world.get_ref(model, "WorldPivotSet"), Ok(Value::Bool(true)))
}

/// `Model.WorldPivot` as Luau reads it: the stored value once set, else the bounding-box centre.
pub fn world_pivot(world: &World, model: InstanceId) -> Result<CFrame, CoreError> {
    if world_pivot_set(world, model) {
        cframe_prop(world, model, "WorldPivot")
    } else {
        Ok(unset_pivot(world, model))
    }
}

/// Sets `Model.WorldPivot` (made orthonormal); from now on it no longer follows the bounding box.
pub fn set_world_pivot(world: &mut World, model: InstanceId, cf: CFrame) -> Result<(), CoreError> {
    let cf = orthonormal(cf, "WorldPivot")?;
    set_cframe_prop(world, model, "WorldPivot", cf)?;
    world.set(model, "WorldPivotSet", Value::Bool(true))
}

/// Pivot of a model whose WorldPivot was never set: the world-aligned bounding-box centre.
fn unset_pivot(world: &World, model: InstanceId) -> CFrame {
    match bounds_in(world, model, CFrame::IDENTITY) {
        Some((center, _)) => CFrame::from_position(center),
        None => CFrame::IDENTITY,
    }
}

/// `PVInstance:GetPivot()` for a BasePart or a Model.
pub fn get_pivot(world: &World, id: InstanceId) -> Result<CFrame, CoreError> {
    if world.is_a(id, "BasePart") {
        Ok(part_cframe(world, id)? * cframe_prop(world, id, "PivotOffset")?)
    } else if world.is_a(id, "Model") {
        match primary_part(world, id) {
            Some(p) => get_pivot(world, p),
            None => world_pivot(world, id),
        }
    } else if world.is_alive(id) {
        Err(wrong("Pivot", "only a BasePart or a Model has a pivot"))
    } else {
        Err(CoreError::InvalidInstance)
    }
}

/// `PVInstance:PivotTo(target)`: moves a part, or every part of a model, so the pivot lands on
/// `target` (made orthonormal). A Model's WorldPivot becomes `target` (or moves with its
/// PrimaryPart), and the WorldPivots of nested Models move with their parts. Every new value is
/// computed and checked first: if any would not be finite, it errors and nothing changes.
pub fn pivot_to(world: &mut World, id: InstanceId, target: CFrame) -> Result<(), CoreError> {
    let target = orthonormal(target, "CFrame")?;
    let mut plan = Plan::default();
    if world.is_a(id, "BasePart") {
        let offset = cframe_prop(world, id, "PivotOffset")?;
        plan.part_cframe(id, target * offset.inverse());
        return plan.apply(world, "PivotTo");
    }
    let current = get_pivot(world, id)?;
    let delta = target * current.inverse();
    let has_primary = primary_part(world, id).is_some();
    for d in descendants(world, id) {
        if world.is_a(d, "BasePart") {
            plan.part_cframe(d, delta * part_cframe(world, d)?);
        } else if world.is_a(d, "Model") && world_pivot_set(world, d) {
            plan.cframe(
                d,
                "WorldPivot",
                delta * cframe_prop(world, d, "WorldPivot")?,
            );
        }
    }
    if !has_primary {
        plan.cframe(id, "WorldPivot", target);
        plan.push(id, "WorldPivotSet", Value::Bool(true));
    } else if world_pivot_set(world, id) {
        plan.cframe(
            id,
            "WorldPivot",
            delta * cframe_prop(world, id, "WorldPivot")?,
        );
    }
    plan.apply(world, "PivotTo")
}

/// Centre and size of the box around every part of `model`, measured along the axes of `frame`
/// (its position is ignored). The centre is in world space. `None` when the model has no parts.
fn bounds_in(world: &World, model: InstanceId, frame: CFrame) -> Option<(Vec3, Vec3)> {
    let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut any = false;
    for d in descendants(world, model) {
        if !world.is_a(d, "BasePart") {
            continue;
        }
        let (Ok(cf), Ok(size)) = (part_cframe(world, d), vec_prop(world, d, "Size")) else {
            continue;
        };
        any = true;
        let half = size * 0.5;
        for corner in 0..8 {
            let sign = |bit: usize| if corner & bit == 0 { -1.0 } else { 1.0 };
            let local = Vec3::new(half.x * sign(1), half.y * sign(2), half.z * sign(4));
            let p = frame.vector_to_object(cf.point_to_world(local));
            lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
    }
    any.then(|| (frame.vector_to_world((lo + hi) * 0.5), hi - lo))
}

/// `Model:GetBoundingBox()`: the box around every part of the model, oriented like the
/// PrimaryPart if there is one, else like the pivot. An empty model gives its pivot and size 0.
pub fn bounding_box(world: &World, model: InstanceId) -> Result<(CFrame, Vec3), CoreError> {
    let orientation = match primary_part(world, model) {
        Some(p) => part_cframe(world, p)?,
        None => get_pivot(world, model)?,
    }
    .rotation_only();
    Ok(match bounds_in(world, model, orientation) {
        Some((center, size)) => (orientation.translated(center), size),
        None => (get_pivot(world, model)?, Vec3::default()),
    })
}

/// `Model:GetScale()`.
pub fn model_scale(world: &World, model: InstanceId) -> f64 {
    match world.get_ref(model, "ScaleFactor") {
        Ok(Value::Number(s)) => *s,
        _ => 1.0,
    }
}

/// `Model:ScaleTo(scale)`: sizes and positions of every part become `scale` times their
/// values at scale 1, about the model's pivot (which stays put). Part `PivotOffset`s and nested
/// Models' WorldPivots scale with them; nested Models' scales multiply. `scale` is absolute:
/// `ScaleTo(2)` then `ScaleTo(1)` restores the original.
///
/// Every new value is computed and checked first. If a part would end up outside
/// `MIN_PART_SIZE..=MAX_PART_SIZE` on any axis, or any value would not be finite, it errors and
/// nothing changes (`GetScale` included).
pub fn scale_to(world: &mut World, model: InstanceId, scale: f64) -> Result<(), CoreError> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(wrong("Scale", "must be a positive finite number"));
    }
    let factor = scale / model_scale(world, model);
    if !(factor.is_finite() && factor > 0.0) {
        return Err(wrong("Scale", "the change of scale is too large"));
    }
    let center = get_pivot(world, model)?.position;
    let about = |p: Vec3| center + (p - center) * factor;
    let mut plan = Plan::default();
    for d in descendants(world, model) {
        if world.is_a(d, "BasePart") {
            let size = vec_prop(world, d, "Size")? * factor;
            if !size_in_limits(size) {
                return Err(wrong(
                    "Size",
                    &format!(
                        "ScaleTo({scale}) would make a part {} by {} by {} studs; each axis must \
                         be {MIN_PART_SIZE} to {MAX_PART_SIZE}. Nothing was changed",
                        size.x, size.y, size.z
                    ),
                ));
            }
            let mut offset = cframe_prop(world, d, "PivotOffset")?;
            offset.position = offset.position * factor;
            let position = about(vec_prop(world, d, "Position")?);
            plan.push(d, "Position", Value::Vector3(position));
            plan.push(d, "Size", Value::Vector3(size));
            plan.cframe(d, "PivotOffset", offset);
        } else if world.is_a(d, "Model") {
            if world_pivot_set(world, d) {
                let mut cf = cframe_prop(world, d, "WorldPivot")?;
                cf.position = about(cf.position);
                plan.cframe(d, "WorldPivot", cf);
            }
            let inner = model_scale(world, d) * factor;
            if !(inner.is_finite() && inner > 0.0) {
                return Err(wrong("Scale", "a nested model's scale would overflow"));
            }
            plan.push(d, "ScaleFactor", Value::Number(inner));
        }
    }
    if world_pivot_set(world, model) {
        let mut cf = cframe_prop(world, model, "WorldPivot")?;
        cf.position = about(cf.position);
        plan.cframe(model, "WorldPivot", cf);
    }
    plan.push(model, "ScaleFactor", Value::Number(scale));
    plan.apply(world, "ScaleTo")
}

/// An oriented box: centre frame and half size.
#[derive(Clone, Copy)]
struct Obb {
    frame: CFrame,
    half: Vec3,
}

impl Obb {
    fn axes(&self) -> [Vec3; 3] {
        [
            self.frame.right_vector(),
            self.frame.up_vector(),
            self.frame.back_vector(),
        ]
    }

    /// Half the length of the box's shadow on the unit axis `l`.
    fn radius(&self, l: Vec3) -> f64 {
        let [x, y, z] = self.axes();
        self.half.x * dot(x, l).abs()
            + self.half.y * dot(y, l).abs()
            + self.half.z * dot(z, l).abs()
    }

    /// True when the boxes overlap deeper than `TOUCH_TOLERANCE` (separating axis test).
    fn overlaps(&self, other: &Obb) -> bool {
        let d = other.frame.position - self.frame.position;
        let (a, b) = (self.axes(), other.axes());
        let mut axes: Vec<Vec3> = a.iter().chain(b.iter()).copied().collect();
        for u in a {
            for v in b {
                let c = cross(u, v);
                if c.magnitude() > 1e-9 {
                    axes.push(c.unit());
                }
            }
        }
        axes.iter()
            .all(|&l| dot(d, l).abs() < self.radius(l) + other.radius(l) - TOUCH_TOLERANCE)
    }
}

fn part_box(world: &World, id: InstanceId) -> Result<Obb, CoreError> {
    Ok(Obb {
        frame: part_cframe(world, id)?,
        half: vec_prop(world, id, "Size")? * 0.5,
    })
}

/// `BasePart:Resize(face, delta)`: moves one face by `delta` studs along its normal while the
/// opposite face stays put (sizes stay within [`MIN_PART_SIZE`] and [`MAX_PART_SIZE`]). Ball parts grow on every axis;
/// a Cylinder's Y and Z (its round section) grow together.
///
/// Returns `false` and changes nothing when the part can collide and the grown part would newly
/// overlap another colliding part in Workspace.
pub fn resize(
    world: &mut World,
    part: InstanceId,
    face: NormalId,
    delta: f64,
) -> Result<bool, CoreError> {
    if !delta.is_finite() {
        return Err(wrong("Size", "the resize amount must be finite"));
    }
    let cf = part_cframe(world, part)?;
    let size = vec_prop(world, part, "Size")?;
    let (i, sign) = face.axis();
    let target = (axis(size, i) + delta).clamp(MIN_PART_SIZE, MAX_PART_SIZE);
    let shape = match world.get_ref(part, "Shape")? {
        Value::String(s) => s.as_str(),
        _ => "Block",
    };
    let new_size = match shape {
        "Ball" => Vec3::new(target, target, target),
        "Cylinder" if i != 0 => with_axis(with_axis(size, 1, target), 2, target),
        _ => with_axis(size, i, target),
    };
    let grown = target - axis(size, i);
    let normal = cf.vector_to_world(with_axis(Vec3::default(), i, sign));
    let new_cf = cf.translated(normal * (grown * 0.5));
    if grown > 0.0 && matches!(world.get_ref(part, "CanCollide")?, Value::Bool(true)) {
        let old = part_box(world, part)?;
        let new = Obb {
            frame: new_cf,
            half: new_size * 0.5,
        };
        let parts = world
            .root("Workspace")
            .map(|ws| descendants(world, ws))
            .unwrap_or_default();
        for other in parts {
            if other == part
                || !world.is_a(other, "BasePart")
                || !matches!(world.get_ref(other, "CanCollide"), Ok(Value::Bool(true)))
            {
                continue;
            }
            let b = part_box(world, other)?;
            if new.overlaps(&b) && !old.overlaps(&b) {
                return Ok(false);
            }
        }
    }
    let mut plan = Plan::default();
    plan.push(part, "Size", Value::Vector3(new_size));
    plan.push(part, "Position", Value::Vector3(new_cf.position));
    plan.apply(world, "Resize")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obb(pos: Vec3, half: Vec3) -> Obb {
        Obb {
            frame: CFrame::from_position(pos),
            half,
        }
    }

    #[test]
    fn touching_boxes_do_not_overlap() {
        let a = obb(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let touching = obb(Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let deep = obb(Vec3::new(1.5, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        assert!(!a.overlaps(&touching));
        assert!(a.overlaps(&deep));
        // Rotated 45 degrees about Y, its corner reaches 1.414 along X.
        let turned = Obb {
            frame: CFrame::from_position(Vec3::new(2.3, 0.0, 0.0))
                * CFrame::from_euler_xyz(0.0, std::f64::consts::FRAC_PI_4, 0.0),
            half: Vec3::new(1.0, 1.0, 1.0),
        };
        assert!(a.overlaps(&turned));
    }
}
