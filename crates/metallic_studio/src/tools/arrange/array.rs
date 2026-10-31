//! Arrays: linear, radial, grid and along a path, plus the drag-handle and short-form maths
//! (building-tools spec 8.2).
//!
//! The selection is copied as one rigid unit; Models copy whole. Counts:
//! - Linear and path: `count` is the number of new copies.
//! - Radial: `count` is the number of items around the arc, the original included, like slices of
//!   a pie. A full circle of 8 gives 45-degree steps and 7 new copies; `/7` gives 7 items.
//! - Grid: items per axis, the original included (3 x 1 x 3 = 8 new copies).

use glam::{DMat3, DVec3};

use super::math::{Axis, Pose};
use super::path::PathCurve;
use super::plan::{ArrangeError, Plan, PlannedGroup, describe};
use super::scene::{Object, selection_bounds_in, selection_pivot};

/// The most copies one array may plan. The preview budget is 200 ghosts (spec 17); a commit may
/// make more.
pub const MAX_COPIES: usize = 1000;

/// The most parts one array may create (copies times parts in the selection), so a big
/// selection cannot plan millions of entries.
pub const MAX_CREATED_PARTS: usize = 10_000;

/// Name of the Model that holds the result when grouping is on.
pub const GROUP_NAME: &str = "Array";

/// What the linear offset means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetIs {
    /// Centre-to-centre step.
    Distance,
    /// Empty space between neighbouring copies, per axis (a non-zero offset on an axis adds the
    /// selection's size on that axis).
    Gap,
}

/// Linear array options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearOptions {
    /// New copies.
    pub count: usize,
    /// Offset per copy.
    pub offset: DVec3,
    /// What `offset` means.
    pub offset_is: OffsetIs,
    /// Put the copies in a new Model named "Array".
    pub group: bool,
}

/// The centre of a radial array.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RadialCentre {
    /// The selection's pivot.
    Pivot,
    /// The active (last selected) object's pivot.
    Active,
    /// A picked point.
    Point(DVec3),
}

/// Radial array options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialOptions {
    /// Items around the arc, the original included.
    pub count: usize,
    /// Total angle in degrees; 360 (or more) is a full circle.
    pub angle: f64,
    /// World axis to turn about.
    pub axis: Axis,
    /// Centre of turning.
    pub centre: RadialCentre,
    /// Turn each copy with its position (on), or only move it (off).
    pub rotate_copies: bool,
    /// Put the copies in a new Model.
    pub group: bool,
}

/// Grid array options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridOptions {
    /// Items along X, Y and Z, the original included.
    pub counts: [usize; 3],
    /// Centre-to-centre step per axis.
    pub spacing: DVec3,
    /// Put the copies in a new Model.
    pub group: bool,
}

/// How copies fill a path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathFill {
    /// A copy every this many studs of arc length, from the start, as many as fit.
    Spacing(f64),
    /// This many copies, the first at the start and the last at the end.
    Count(usize),
}

/// Array-along-path options.
#[derive(Debug, Clone, PartialEq)]
pub struct PathOptions {
    /// Points the curve passes through, in order (picked parts' positions or clicked points);
    /// at most [`MAX_PATH_POINTS`](super::path::MAX_PATH_POINTS).
    pub points: Vec<DVec3>,
    /// Spacing or count.
    pub fill: PathFill,
    /// Turn copies to follow the path's direction (relative to the direction at its start).
    pub align: bool,
    /// The world axis kept as "up" while aligning.
    pub up: Axis,
    /// Put the copies in a new Model.
    pub group: bool,
}

fn nonempty(objects: &[Object]) -> Result<(), ArrangeError> {
    if objects.is_empty() {
        return Err(ArrangeError::NeedMore { needed: 1, got: 0 });
    }
    Ok(())
}

fn check_copies(n: usize) -> Result<(), ArrangeError> {
    if n == 0 || n > MAX_COPIES {
        return Err(ArrangeError::Invalid(format!(
            "an array makes 1 to {MAX_COPIES} copies (asked for {n})"
        )));
    }
    Ok(())
}

fn finite_vec(v: DVec3, what: &str) -> Result<DVec3, ArrangeError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(ArrangeError::Invalid(format!(
            "{what} must be a finite number"
        )))
    }
}

/// Errors if `copies` copies of `objects` would create more than [`MAX_CREATED_PARTS`] parts.
fn check_created_parts(objects: &[Object], copies: usize) -> Result<(), ArrangeError> {
    let parts: usize = objects.iter().map(|o| o.parts.len()).sum();
    match parts.checked_mul(copies) {
        Some(n) if n <= MAX_CREATED_PARTS => Ok(()),
        _ => Err(ArrangeError::Invalid(format!(
            "an array creates at most {MAX_CREATED_PARTS} parts ({copies} copies of {parts} parts is too many)"
        ))),
    }
}

/// One rigid transform per new copy, applied to the whole selection.
fn plan_copies(objects: &[Object], transforms: &[Pose], group: bool) -> Result<Plan, ArrangeError> {
    let copies = transforms.len();
    check_created_parts(objects, copies)?;
    let mut plan = Plan {
        label: format!(
            "Array {} ({copies} cop{})",
            describe(objects),
            if copies == 1 { "y" } else { "ies" }
        ),
        group: group.then(|| PlannedGroup {
            name: GROUP_NAME.into(),
            parent: objects[0].parent,
        }),
        ..Plan::default()
    };
    for t in transforms {
        for o in objects {
            plan.push_copy(o, &o.transformed(t));
        }
    }
    plan.check_finite()
}

/// The centre-to-centre step of a linear array for a selection of size `size`.
pub fn linear_step(size: DVec3, offset: DVec3, offset_is: OffsetIs) -> DVec3 {
    match offset_is {
        OffsetIs::Distance => offset,
        OffsetIs::Gap => {
            let grow = |o: f64, s: f64| if o == 0.0 { 0.0 } else { o + s * o.signum() };
            DVec3::new(
                grow(offset.x, size.x),
                grow(offset.y, size.y),
                grow(offset.z, size.z),
            )
        }
    }
}

/// The default linear offset: the last smart-duplicate offset, else the selection's size along
/// the world axis closest to the camera's right direction (copies then sit side by side, 0 gap).
pub fn default_linear_offset(
    last_duplicate: Option<DVec3>,
    size: DVec3,
    camera_right: DVec3,
) -> DVec3 {
    if let Some(d) = last_duplicate {
        return d;
    }
    let axis = Axis::ALL
        .into_iter()
        .max_by(|a, b| {
            a.of(camera_right)
                .abs()
                .total_cmp(&b.of(camera_right).abs())
        })
        .unwrap_or(Axis::X);
    let sign = if axis.of(camera_right) < 0.0 {
        -1.0
    } else {
        1.0
    };
    axis.unit() * (axis.of(size) * sign)
}

/// Plans a linear array.
///
/// Errors: [`ArrangeError::NeedMore`] with no objects; [`ArrangeError::Invalid`] for a count
/// outside 1..=[`MAX_COPIES`] or a zero step.
pub fn linear(objects: &[Object], opts: &LinearOptions) -> Result<Plan, ArrangeError> {
    nonempty(objects)?;
    check_copies(opts.count)?;
    let size = selection_bounds_in(objects, &DMat3::IDENTITY).size();
    let offset = finite_vec(opts.offset, "the offset")?;
    let step = linear_step(size, offset, opts.offset_is);
    let len = step.length();
    if !(len.is_finite() && len > 1e-9) {
        return Err(ArrangeError::Invalid(
            "the offset must be finite and not zero".into(),
        ));
    }
    let transforms: Vec<Pose> = (1..=opts.count)
        .map(|i| Pose::translation(step * i as f64))
        .collect();
    plan_copies(objects, &transforms, opts.group)
}

/// The angle of each new copy of a radial array, in degrees from the original.
pub fn radial_angles(count: usize, angle: f64) -> Vec<f64> {
    if count < 2 {
        return Vec::new();
    }
    let full = angle.abs() >= 360.0 - 1e-9;
    let step = if full {
        360.0 * angle.signum() / count as f64
    } else {
        angle / (count - 1) as f64
    };
    (1..count).map(|k| step * k as f64).collect()
}

/// Plans a radial array.
///
/// Errors: [`ArrangeError::NeedMore`] with no objects; [`ArrangeError::Invalid`] for fewer than 2
/// items, too many copies or a zero angle.
pub fn radial(objects: &[Object], opts: &RadialOptions) -> Result<Plan, ArrangeError> {
    nonempty(objects)?;
    if opts.count < 2 {
        return Err(ArrangeError::Invalid(
            "a radial array needs 2 or more items".into(),
        ));
    }
    check_copies(opts.count - 1)?;
    if !opts.angle.is_finite() || opts.angle.abs() < 1e-9 {
        return Err(ArrangeError::Invalid("the angle must not be zero".into()));
    }
    let centre = match opts.centre {
        RadialCentre::Pivot => selection_pivot(objects),
        RadialCentre::Active => objects[objects.len() - 1].pivot().pos,
        RadialCentre::Point(p) => finite_vec(p, "the centre")?,
    };
    let reference = selection_pivot(objects);
    let transforms: Vec<Pose> = radial_angles(opts.count, opts.angle)
        .into_iter()
        .map(|a| {
            let turn = Pose::rotation_about(centre, opts.axis.unit(), a);
            if opts.rotate_copies {
                turn
            } else {
                Pose::translation(turn.point(reference) - reference)
            }
        })
        .collect();
    plan_copies(objects, &transforms, opts.group)
}

/// Plans a grid array.
///
/// Errors: [`ArrangeError::NeedMore`] with no objects; [`ArrangeError::Invalid`] for a zero count
/// on an axis or a result outside 1..=[`MAX_COPIES`] copies.
pub fn grid(objects: &[Object], opts: &GridOptions) -> Result<Plan, ArrangeError> {
    nonempty(objects)?;
    let [nx, ny, nz] = opts.counts;
    let total = nx.saturating_mul(ny).saturating_mul(nz);
    if total == 0 {
        return Err(ArrangeError::Invalid(
            "each grid count must be 1 or more".into(),
        ));
    }
    check_copies(total - 1)?;
    finite_vec(opts.spacing, "the spacing")?;
    let mut transforms = Vec::with_capacity(total - 1);
    for iy in 0..ny {
        for iz in 0..nz {
            for ix in 0..nx {
                if ix == 0 && iy == 0 && iz == 0 {
                    continue;
                }
                let d = DVec3::new(
                    opts.spacing.x * ix as f64,
                    opts.spacing.y * iy as f64,
                    opts.spacing.z * iz as f64,
                );
                transforms.push(Pose::translation(d));
            }
        }
    }
    plan_copies(objects, &transforms, opts.group)
}

/// A rotation whose X column is `tangent` and whose Y column is as close to `up` as possible.
fn path_frame(tangent: DVec3, up: DVec3) -> DMat3 {
    let x = tangent.normalize_or_zero();
    let mut side = x.cross(up);
    if side.length() < 1e-6 {
        // Tangent parallel to up: any perpendicular side works.
        let alt = if x.x.abs() < 0.9 {
            Axis::X.unit()
        } else {
            Axis::Z.unit()
        };
        side = x.cross(alt);
    }
    let z = side.normalize();
    let y = z.cross(x);
    DMat3::from_cols(x, y, z)
}

/// The arc lengths at which copies sit on a curve of length `length`.
pub fn path_stations(length: f64, fill: PathFill) -> Result<Vec<f64>, ArrangeError> {
    match fill {
        PathFill::Spacing(s) => {
            if !(s.is_finite() && s > 1e-6) {
                return Err(ArrangeError::Invalid("the spacing must be positive".into()));
            }
            if !(length.is_finite() && length >= 0.0) {
                return Err(ArrangeError::Invalid(
                    "the path length must be finite".into(),
                ));
            }
            let n = (length / s + 1e-9).floor() + 1.0;
            if n.is_nan() || n > MAX_COPIES as f64 {
                return Err(ArrangeError::Invalid(format!(
                    "an array makes 1 to {MAX_COPIES} copies; this spacing needs more"
                )));
            }
            let n = n as usize;
            check_copies(n)?;
            Ok((0..n).map(|k| s * k as f64).collect())
        }
        PathFill::Count(n) => {
            check_copies(n)?;
            if !(length.is_finite() && length >= 0.0) {
                return Err(ArrangeError::Invalid(
                    "the path length must be finite".into(),
                ));
            }
            if n == 1 {
                return Ok(vec![0.0]);
            }
            Ok((0..n).map(|k| length * k as f64 / (n - 1) as f64).collect())
        }
    }
}

/// The default path spacing: the selection's length along the path's starting direction.
pub fn default_path_spacing(size: DVec3, start_tangent: DVec3) -> f64 {
    let t = start_tangent.normalize_or_zero();
    size.x * t.x.abs() + size.y * t.y.abs() + size.z * t.z.abs()
}

/// Plans an array along a path. Each copy's pivot sits on the curve; the original stays.
///
/// Errors: [`ArrangeError::NeedMore`] with no objects; [`ArrangeError::Invalid`] for a bad path,
/// spacing or count.
pub fn along_path(objects: &[Object], opts: &PathOptions) -> Result<Plan, ArrangeError> {
    nonempty(objects)?;
    let curve = PathCurve::new(&opts.points)?;
    let stations = path_stations(curve.length(), opts.fill)?;
    let pivot = selection_pivot(objects);
    let up = opts.up.unit();
    let start = path_frame(curve.tangent_at(0.0), up);
    let transforms: Vec<Pose> = stations
        .iter()
        .map(|&s| {
            let at = curve.point_at(s);
            let rot = if opts.align {
                path_frame(curve.tangent_at(s), up) * start.transpose()
            } else {
                DMat3::IDENTITY
            };
            // Turn about the pivot, then carry the pivot onto the curve.
            Pose {
                pos: at - rot * pivot,
                rot,
            }
        })
        .collect();
    plan_copies(objects, &transforms, opts.group)
}

/// Where the linear drag handle sits: one step past the last copy. `count` is clamped to
/// [`MAX_COPIES`], so it cannot overflow.
pub fn handle_position(pivot: DVec3, step: DVec3, count: usize) -> DVec3 {
    pivot + step * (count.min(MAX_COPIES) + 1) as f64
}

/// The count after dragging the handle `along` studs in the step's direction (negative removes
/// copies), starting from `start_count`. Clamped to 1..=[`MAX_COPIES`].
pub fn count_from_drag(start_count: usize, along: f64, step: DVec3) -> usize {
    let base = start_count.clamp(1, MAX_COPIES);
    let len = step.length();
    let ratio = along / len;
    if !(len.is_finite() && len > 1e-9 && ratio.is_finite()) {
        return base;
    }
    // Clamp in f64 first so the cast and the add cannot overflow.
    let added = ratio.round().clamp(-(MAX_COPIES as f64), MAX_COPIES as f64) as i64;
    (base as i64 + added).clamp(1, MAX_COPIES as i64) as usize
}

/// How far a drag from `from` to `to` moves along the step direction.
pub fn drag_along(from: DVec3, to: DVec3, step: DVec3) -> f64 {
    let len = step.length();
    let along = (to - from).dot(step) / len;
    if len.is_finite() && len > 1e-9 && along.is_finite() {
        along
    } else {
        0.0
    }
}

/// The count after `notches` wheel clicks over the handle (up adds one each).
pub fn count_from_wheel(count: usize, notches: i32) -> usize {
    let base = count.clamp(1, MAX_COPIES) as i64;
    base.saturating_add(i64::from(notches))
        .clamp(1, MAX_COPIES as i64) as usize
}

/// A parsed Count field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountField {
    /// The count.
    pub count: usize,
    /// Typed as `5x`: the offset is the step per copy (the offset becomes a Distance).
    pub times: bool,
}

/// Parses the Count field: `5` or `5x` (also `x5`, `*5`).
pub fn parse_count(text: &str) -> Option<CountField> {
    let t = text.trim().to_ascii_lowercase();
    let (digits, times) = if let Some(d) = t.strip_suffix('x') {
        (d, true)
    } else if let Some(d) = t.strip_prefix('x').or_else(|| t.strip_prefix('*')) {
        (d, true)
    } else {
        (t.as_str(), false)
    };
    let count: usize = digits.trim().parse().ok()?;
    (1..=MAX_COPIES)
        .contains(&count)
        .then_some(CountField { count, times })
}

/// A parsed radial Angle field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AngleField {
    /// Degrees of total angle.
    Degrees(f64),
    /// `/n`: n items evenly around the full circle (sets Count to n and Angle to 360).
    Divide(usize),
}

/// Parses the radial Angle field: `90`, `90°` or `/7`.
pub fn parse_angle(text: &str) -> Option<AngleField> {
    let t = text.trim();
    if let Some(n) = t.strip_prefix('/') {
        let n: usize = n.trim().parse().ok()?;
        return (2..=MAX_COPIES + 1)
            .contains(&n)
            .then_some(AngleField::Divide(n));
    }
    let d: f64 = t.trim_end_matches('°').trim().parse().ok()?;
    (d.is_finite() && d != 0.0).then_some(AngleField::Degrees(d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::arrange::math::{max_diff, rotation_from_orientation};
    use crate::tools::arrange::path::MAX_PATH_POINTS;
    use crate::tools::arrange::testing::*;

    #[test]
    fn linear_distance_and_gap_steps() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(4.0, 1.0, 2.0), v(0.0, 0.0, 0.0));
        let o = LinearOptions {
            count: 3,
            offset: v(5.0, 0.0, 0.0),
            offset_is: OffsetIs::Distance,
            group: false,
        };
        let plan = linear(&objs(&w, &[a]), &o).unwrap();
        let xs: Vec<f64> = plan.copies.iter().map(|c| c.parts[0].position.x).collect();
        assert_eq!(xs, vec![5.0, 10.0, 15.0]);
        assert_eq!(plan.label, "Array 1 part (3 copies)");
        assert!(plan.changes.is_empty());
        assert!(plan.group.is_none());

        // Gap 1 on X with width 4: step 5. Gap -2 on Z with depth 2: step -4.
        let o = LinearOptions {
            offset: v(1.0, 0.0, -2.0),
            offset_is: OffsetIs::Gap,
            count: 2,
            group: true,
        };
        let plan = linear(&objs(&w, &[a]), &o).unwrap();
        assert_vec(plan.copies[1].parts[0].position, v(10.0, 0.0, -8.0));
        assert_eq!(plan.group.as_ref().unwrap().name, "Array");
    }

    #[test]
    fn linear_copies_a_model_whole() {
        let mut w = world();
        let (m, kids) = model(
            &mut w,
            &[
                (v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0)),
                (v(0.0, 2.0, 0.0), v(1.0, 1.0, 1.0)),
            ],
        );
        let o = LinearOptions {
            count: 2,
            offset: v(0.0, 0.0, 3.0),
            offset_is: OffsetIs::Distance,
            group: false,
        };
        let plan = linear(&objs(&w, &[m]), &o).unwrap();
        assert_eq!(plan.copies.len(), 2);
        assert_eq!(plan.copies[0].source, m);
        assert_eq!(plan.copies[0].parts[1].source_part, kids[1]);
        assert_vec(plan.copies[1].parts[1].position, v(0.0, 2.0, 6.0));
    }

    #[test]
    fn linear_rejects_bad_counts_and_zero_offsets() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let mut o = LinearOptions {
            count: 0,
            offset: v(1.0, 0.0, 0.0),
            offset_is: OffsetIs::Distance,
            group: false,
        };
        assert!(linear(&objs(&w, &[a]), &o).is_err());
        o.count = MAX_COPIES + 1;
        assert!(linear(&objs(&w, &[a]), &o).is_err());
        o.count = 2;
        o.offset = v(0.0, 0.0, 0.0);
        assert!(linear(&objs(&w, &[a]), &o).is_err());
    }

    #[test]
    fn radial_divide_by_seven_angles() {
        let AngleField::Divide(n) = parse_angle("/7").unwrap() else {
            panic!("expected /7 to divide");
        };
        assert_eq!(n, 7);
        let angles = radial_angles(n, 360.0);
        assert_eq!(angles.len(), 6);
        for (k, a) in angles.iter().enumerate() {
            assert!((a - 360.0 / 7.0 * (k + 1) as f64).abs() < 1e-9);
        }
        // Partial arc: ends included. 4 items over 90 degrees = 30-degree steps.
        assert_eq!(radial_angles(4, 90.0), vec![30.0, 60.0, 90.0]);
        assert_eq!(radial_angles(8, 360.0)[0], 45.0);
    }

    #[test]
    fn radial_places_and_turns_copies_about_the_centre() {
        let mut w = world();
        let a = part(
            &mut w,
            v(10.0, 0.0, 0.0),
            v(2.0, 1.0, 1.0),
            v(0.0, 0.0, 0.0),
        );
        let o = RadialOptions {
            count: 7,
            angle: 360.0,
            axis: Axis::Y,
            centre: RadialCentre::Point(v(0.0, 0.0, 0.0)),
            rotate_copies: true,
            group: true,
        };
        let plan = radial(&objs(&w, &[a]), &o).unwrap();
        assert_eq!(plan.copies.len(), 6);
        for (k, c) in plan.copies.iter().enumerate() {
            let theta = (360.0 / 7.0 * (k + 1) as f64).to_radians();
            // Turning +theta about +Y takes +X towards -Z.
            assert_vec(
                c.parts[0].position,
                v(10.0 * theta.cos(), 0.0, -10.0 * theta.sin()),
            );
            let rot = rotation_from_orientation(c.parts[0].orientation);
            let local_x = rot.col(0);
            assert_vec(local_x, v(theta.cos(), 0.0, -theta.sin()));
        }
        let o = RadialOptions {
            rotate_copies: false,
            count: 4,
            angle: 90.0,
            ..o
        };
        let plan = radial(&objs(&w, &[a]), &o).unwrap();
        assert_vec(plan.copies[2].parts[0].position, v(0.0, 0.0, -10.0));
        assert_vec(plan.copies[2].parts[0].orientation, v(0.0, 0.0, 0.0));
    }

    #[test]
    fn radial_about_the_active_object() {
        let mut w = world();
        let blade = part(&mut w, v(0.0, 0.0, 5.0), v(1.0, 1.0, 4.0), v(0.0, 0.0, 0.0));
        let hub = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let o = RadialOptions {
            count: 2,
            angle: 360.0,
            axis: Axis::Y,
            centre: RadialCentre::Active,
            rotate_copies: true,
            group: false,
        };
        // Both selected: the hub is copied too (onto itself at 180 degrees), the blade flips over.
        let plan = radial(&objs(&w, &[blade, hub]), &o).unwrap();
        assert_vec(plan.copies[0].parts[0].position, v(0.0, 0.0, -5.0));
        assert_vec(plan.copies[1].parts[0].position, v(0.0, 0.0, 0.0));
        assert!(radial(&objs(&w, &[blade]), &RadialOptions { count: 1, ..o }).is_err());
    }

    #[test]
    fn grid_counts_include_the_original() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let o = GridOptions {
            counts: [3, 1, 3],
            spacing: v(2.0, 0.0, 5.0),
            group: true,
        };
        let plan = grid(&objs(&w, &[a]), &o).unwrap();
        assert_eq!(plan.copies.len(), 8);
        assert_vec(
            plan.copies.last().unwrap().parts[0].position,
            v(4.0, 0.0, 10.0),
        );
        assert!(
            grid(
                &objs(&w, &[a]),
                &GridOptions {
                    counts: [0, 1, 1],
                    ..o
                }
            )
            .is_err()
        );
    }

    /// Arc length between two curve locations by fine, independent integration.
    fn arc_between(c: &PathCurve, a: (usize, f64), b: (usize, f64)) -> f64 {
        let steps = 20_000;
        let ta = a.0 as f64 + a.1;
        let tb = b.0 as f64 + b.1;
        let at = |t: f64| {
            let seg = (t.floor() as usize).min(c.segments() - 1);
            c.eval(seg, t - seg as f64)
        };
        let mut len = 0.0;
        let mut prev = at(ta);
        for i in 1..=steps {
            let p = at(ta + (tb - ta) * i as f64 / steps as f64);
            len += (p - prev).length();
            prev = p;
        }
        len
    }

    #[test]
    fn path_copies_sit_at_equal_arc_length_within_one_percent() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let points = vec![
            v(0.0, 0.0, 0.0),
            v(12.0, 0.0, 2.0),
            v(18.0, 4.0, 14.0),
            v(6.0, 4.0, 22.0),
            v(-4.0, 0.0, 30.0),
        ];
        let curve = PathCurve::new(&points).unwrap();
        for fill in [PathFill::Count(9), PathFill::Spacing(3.5)] {
            let o = PathOptions {
                points: points.clone(),
                fill,
                align: false,
                up: Axis::Y,
                group: false,
            };
            let plan = along_path(&objs(&w, &[a]), &o).unwrap();
            let stations = path_stations(curve.length(), fill).unwrap();
            assert_eq!(plan.copies.len(), stations.len());
            let spacing = match fill {
                PathFill::Count(n) => curve.length() / (n - 1) as f64,
                PathFill::Spacing(s) => s,
            };
            for k in 1..stations.len() {
                let pa = plan.copies[k - 1].parts[0].position;
                let pb = plan.copies[k].parts[0].position;
                // Copies lie on the curve.
                assert_vec(curve.point_at(stations[k]), pb);
                let arc = arc_between(
                    &curve,
                    curve.locate(stations[k - 1]),
                    curve.locate(stations[k]),
                );
                assert!(
                    (arc - spacing).abs() / spacing < 0.01,
                    "arc {arc} vs spacing {spacing} between {pa:?} and {pb:?}"
                );
            }
        }
    }

    #[test]
    fn path_alignment_follows_the_tangent() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 5.0, 0.0), v(2.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        // An L: along +X, then along +Z. With few points the corner rounds, but the end runs +Z.
        let o = PathOptions {
            points: vec![v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0), v(10.0, 0.0, 10.0)],
            fill: PathFill::Count(3),
            align: true,
            up: Axis::Y,
            group: false,
        };
        let plan = along_path(&objs(&w, &[a]), &o).unwrap();
        let first = rotation_from_orientation(plan.copies[0].parts[0].orientation);
        assert!(max_diff(&first, &DMat3::IDENTITY) < 1e-6);
        assert_vec(plan.copies[0].parts[0].position, v(0.0, 0.0, 0.0));
        let last = &plan.copies[2].parts[0];
        assert_vec(last.position, v(10.0, 0.0, 10.0));
        let curve = PathCurve::new(&o.points).unwrap();
        let t = curve.tangent_at(curve.length());
        let rot = rotation_from_orientation(last.orientation);
        assert!((rot.col(0).dot(t) - 1.0).abs() < 1e-6);
        assert!(rot.col(1).y > 0.99, "up stays up");
    }

    #[test]
    fn default_offsets() {
        let size = v(4.0, 1.0, 2.0);
        assert_vec(
            default_linear_offset(None, size, v(0.1, 0.0, -0.9)),
            v(0.0, 0.0, -2.0),
        );
        assert_vec(
            default_linear_offset(None, size, v(1.0, 0.0, 0.0)),
            v(4.0, 0.0, 0.0),
        );
        assert_vec(
            default_linear_offset(Some(v(1.0, 2.0, 3.0)), size, v(1.0, 0.0, 0.0)),
            v(1.0, 2.0, 3.0),
        );
        assert!((default_path_spacing(size, v(0.0, 0.0, 1.0)) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn drag_handle_and_wheel_change_the_count() {
        let step = v(5.0, 0.0, 0.0);
        let pivot = v(1.0, 0.0, 0.0);
        assert_vec(handle_position(pivot, step, 3), v(21.0, 0.0, 0.0));
        let along = drag_along(v(21.0, 0.0, 0.0), v(33.0, 7.0, 0.0), step);
        assert!((along - 12.0).abs() < 1e-12);
        assert_eq!(count_from_drag(3, along, step), 5); // 12 / 5 rounds to 2
        assert_eq!(count_from_drag(3, -100.0, step), 1);
        assert_eq!(count_from_wheel(3, 1), 4);
        assert_eq!(count_from_wheel(1, -3), 1);
        assert_eq!(count_from_wheel(MAX_COPIES, 2), MAX_COPIES);
    }

    #[test]
    fn short_forms() {
        assert_eq!(
            parse_count("5x"),
            Some(CountField {
                count: 5,
                times: true
            })
        );
        assert_eq!(
            parse_count(" 12 "),
            Some(CountField {
                count: 12,
                times: false
            })
        );
        assert_eq!(
            parse_count("x3"),
            Some(CountField {
                count: 3,
                times: true
            })
        );
        assert_eq!(parse_count("0"), None);
        assert_eq!(parse_count("abc"), None);
        assert_eq!(parse_angle("90°"), Some(AngleField::Degrees(90.0)));
        assert_eq!(parse_angle("-45"), Some(AngleField::Degrees(-45.0)));
        assert_eq!(parse_angle("/1"), None);
        assert_eq!(parse_angle("0"), None);
    }

    #[test]
    fn copies_keep_part_sizes_and_record_their_source() {
        let mut w = world();
        let a = part(
            &mut w,
            v(0.0, 0.0, 0.0),
            v(3.0, 2.0, 1.0),
            v(0.0, 45.0, 0.0),
        );
        let o = LinearOptions {
            count: 1,
            offset: v(0.0, 3.0, 0.0),
            offset_is: OffsetIs::Distance,
            group: false,
        };
        let plan = linear(&objs(&w, &[a]), &o).unwrap();
        let c = plan.copies[0].parts[0];
        assert_eq!(c.source_part, a);
        assert_vec(c.size, v(3.0, 2.0, 1.0));
        let src = objs(&w, &[a])[0].parts[0];
        assert!(max_diff(&rotation_from_orientation(c.orientation), &src.pose().rot) < 1e-12);
    }

    const BAD: [f64; 3] = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY];

    #[test]
    fn non_finite_and_huge_inputs_are_rejected() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let lin = LinearOptions {
            count: 3,
            offset: v(1.0, 0.0, 0.0),
            offset_is: OffsetIs::Distance,
            group: false,
        };
        let rad = RadialOptions {
            count: 4,
            angle: 360.0,
            axis: Axis::Y,
            centre: RadialCentre::Point(v(0.0, 0.0, 0.0)),
            rotate_copies: true,
            group: false,
        };
        let grd = GridOptions {
            counts: [2, 1, 1],
            spacing: v(1.0, 0.0, 0.0),
            group: false,
        };
        for x in BAD {
            for offset_is in [OffsetIs::Distance, OffsetIs::Gap] {
                let o = LinearOptions {
                    offset: v(x, 0.0, 0.0),
                    offset_is,
                    ..lin
                };
                assert!(linear(&objs(&w, &[a]), &o).is_err(), "linear offset {x}");
            }
            let o = RadialOptions {
                centre: RadialCentre::Point(v(0.0, x, 0.0)),
                ..rad
            };
            assert!(radial(&objs(&w, &[a]), &o).is_err(), "radial centre {x}");
            assert!(
                radial(&objs(&w, &[a]), &RadialOptions { angle: x, ..rad }).is_err(),
                "angle {x}"
            );
            let o = GridOptions {
                spacing: v(0.0, 0.0, x),
                ..grd
            };
            assert!(grid(&objs(&w, &[a]), &o).is_err(), "grid spacing {x}");
            let p = PathOptions {
                points: vec![v(0.0, 0.0, 0.0), v(x, 0.0, 0.0)],
                fill: PathFill::Count(2),
                align: true,
                up: Axis::Y,
                group: false,
            };
            assert!(along_path(&objs(&w, &[a]), &p).is_err(), "path point {x}");
            let p = PathOptions {
                points: vec![v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0)],
                fill: PathFill::Spacing(x),
                ..p
            };
            assert!(along_path(&objs(&w, &[a]), &p).is_err(), "path spacing {x}");
        }
        // Huge but finite values overflow to infinity in the result and are rejected.
        let o = LinearOptions {
            offset: v(f64::MAX, 0.0, 0.0),
            ..lin
        };
        assert!(linear(&objs(&w, &[a]), &o).is_err());
        let o = GridOptions {
            spacing: v(f64::MAX, 0.0, 0.0),
            counts: [3, 1, 1],
            ..grd
        };
        assert!(grid(&objs(&w, &[a]), &o).is_err());
        let p = PathOptions {
            points: vec![
                v(-1e307, 0.0, 0.0),
                v(1e307, 0.0, 0.0),
                v(1e307, 1e307, 0.0),
            ],
            fill: PathFill::Count(3),
            align: false,
            up: Axis::Y,
            group: false,
        };
        assert!(along_path(&objs(&w, &[a]), &p).is_err());
        let o = GridOptions {
            counts: [usize::MAX, usize::MAX, 2],
            ..grd
        };
        assert!(grid(&objs(&w, &[a]), &o).is_err());
    }

    #[test]
    fn path_stations_never_overflow() {
        for len in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(
                path_stations(len, PathFill::Spacing(1.0)).is_err(),
                "length {len}"
            );
            assert!(
                path_stations(len, PathFill::Count(3)).is_err(),
                "length {len}"
            );
        }
        assert!(path_stations(1e300, PathFill::Spacing(1e-3)).is_err());
        assert!(path_stations(f64::MAX, PathFill::Spacing(2e-6)).is_err());
        assert_eq!(
            path_stations(10.0, PathFill::Spacing(f64::MAX)).unwrap(),
            vec![0.0]
        );
        assert!(path_stations(10.0, PathFill::Count(usize::MAX)).is_err());
        assert_eq!(
            path_stations(10.0, PathFill::Spacing(5.0)).unwrap(),
            vec![0.0, 5.0, 10.0]
        );
    }

    #[test]
    fn drag_and_wheel_counts_saturate() {
        let step = v(2.0, 0.0, 0.0);
        assert_eq!(count_from_drag(3, f64::MAX, step), MAX_COPIES);
        assert_eq!(count_from_drag(3, -f64::MAX, step), 1);
        assert_eq!(count_from_drag(usize::MAX, 1e9, step), MAX_COPIES);
        assert_eq!(count_from_drag(usize::MAX, -1e9, step), 1);
        assert_eq!(count_from_drag(0, 0.0, step), 1);
        for x in BAD {
            assert_eq!(count_from_drag(3, x, step), 3, "along {x}");
            assert_eq!(count_from_drag(3, 4.0, v(x, 0.0, 0.0)), 3, "step {x}");
            assert_eq!(drag_along(v(0.0, 0.0, 0.0), v(x, 0.0, 0.0), step), 0.0);
            assert_eq!(
                drag_along(v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(x, 0.0, 0.0)),
                0.0
            );
        }
        // Tiny step: the ratio is huge but finite and clamps.
        assert_eq!(count_from_drag(3, 1e300, v(1e-8, 0.0, 0.0)), MAX_COPIES);
        assert_eq!(count_from_wheel(usize::MAX, i32::MAX), MAX_COPIES);
        assert_eq!(count_from_wheel(0, i32::MIN), 1);
        assert_eq!(count_from_wheel(usize::MAX, -1), MAX_COPIES - 1);
    }

    #[test]
    fn created_parts_are_capped_across_copies_and_parts() {
        let mut w = world();
        let parts: Vec<(DVec3, DVec3)> = (0..20)
            .map(|i| (v(i as f64, 0.0, 0.0), v(1.0, 1.0, 1.0)))
            .collect();
        let (m, _) = model(&mut w, &parts);
        let o = LinearOptions {
            count: 500,
            offset: v(0.0, 0.0, 2.0),
            offset_is: OffsetIs::Distance,
            group: false,
        };
        // 500 copies x 20 parts = 10,000: at the cap.
        let plan = linear(&objs(&w, &[m]), &o).unwrap();
        assert_eq!(plan.copies.len(), 500);
        // 501 x 20 = 10,020: over it, although 501 copies alone would be allowed.
        assert!(linear(&objs(&w, &[m]), &LinearOptions { count: 501, ..o }).is_err());
        let r = RadialOptions {
            count: 600,
            angle: 360.0,
            axis: Axis::Y,
            centre: RadialCentre::Point(v(0.0, 0.0, 50.0)),
            rotate_copies: true,
            group: false,
        };
        assert!(radial(&objs(&w, &[m]), &r).is_err());
        let g = GridOptions {
            counts: [10, 10, 10],
            spacing: v(30.0, 2.0, 2.0),
            group: false,
        };
        assert!(grid(&objs(&w, &[m]), &g).is_err());
        let p = PathOptions {
            points: vec![v(0.0, 0.0, 0.0), v(0.0, 0.0, 1000.0)],
            fill: PathFill::Count(600),
            align: false,
            up: Axis::Y,
            group: false,
        };
        assert!(along_path(&objs(&w, &[m]), &p).is_err());
    }

    #[test]
    fn handle_position_clamps_huge_counts() {
        let step = d(2.0, 0.0, 0.0);
        let at_max = handle_position(DVec3::ZERO, step, MAX_COPIES);
        assert_dvec(at_max, d(2.0 * (MAX_COPIES + 1) as f64, 0.0, 0.0));
        assert_dvec(handle_position(DVec3::ZERO, step, usize::MAX), at_max);
        assert_dvec(handle_position(DVec3::ZERO, step, MAX_COPIES + 1), at_max);
    }

    #[test]
    fn paths_are_capped_at_max_points() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let path = |n: usize| PathOptions {
            points: (0..n).map(|i| d(i as f64, 0.0, (i % 2) as f64)).collect(),
            fill: PathFill::Count(4),
            align: false,
            up: Axis::Y,
            group: false,
        };
        assert!(along_path(&objs(&w, &[a]), &path(MAX_PATH_POINTS)).is_ok());
        match along_path(&objs(&w, &[a]), &path(MAX_PATH_POINTS + 1)) {
            Err(ArrangeError::Invalid(msg)) => {
                assert!(msg.contains("1024"), "message names the limit: {msg}");
            }
            other => panic!("expected the point cap, got {other:?}"),
        }
    }
}
