//! Align and distribute (building-tools spec 6.1 and 6.2).
//!
//! Both work on each object's bounding box along one axis of a frame: the world axes, or the
//! active object's axes in Local space. Objects only translate along that axis.

use metallic_core::{InstanceId, World};

use super::math::{Aabb, Axis, Mat3, Pose};
use super::plan::{ArrangeError, Plan, describe};
use super::scene::{Object, read_objects, selection_bounds_in};

/// Which edge of the bounding box lines up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The low side.
    Min,
    /// The centre.
    Centre,
    /// The high side.
    Max,
}

/// What the objects align to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RelativeTo {
    /// The active object (the last selected); it stays put. The default.
    Active,
    /// The bounding box of the whole selection.
    Selection,
    /// The nearest grid line, spaced this many studs apart, for each object on its own.
    Grid(f64),
}

/// Which axes "X", "Y" and "Z" mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    /// World axes.
    World,
    /// The active object's local axes (a Model's are the world axes).
    Local,
}

/// Align options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignOptions {
    /// Axis.
    pub axis: Axis,
    /// Edge.
    pub edge: Edge,
    /// Reference.
    pub relative: RelativeTo,
    /// Frame.
    pub space: Space,
}

/// How distribute spaces the objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributeMode {
    /// Equal distance between centres.
    Centres,
    /// Equal empty space between neighbouring bounding boxes.
    Gaps,
}

/// Distribute options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistributeOptions {
    /// Axis.
    pub axis: Axis,
    /// Mode.
    pub mode: DistributeMode,
    /// `None` keeps the current span (first and last objects stay). `Some(v)` sets that exact
    /// centre spacing (Centres) or gap (Gaps); the first object stays and the span changes.
    pub spacing: Option<f64>,
    /// Frame.
    pub space: Space,
}

fn edge_of(b: &Aabb, axis: Axis, edge: Edge) -> f64 {
    match edge {
        Edge::Min => axis.of(b.min),
        Edge::Centre => axis.of(b.centre()),
        Edge::Max => axis.of(b.max),
    }
}

fn frame_for(objects: &[Object], space: Space) -> Mat3 {
    match space {
        Space::World => Mat3::IDENTITY,
        Space::Local => objects.last().map_or(Mat3::IDENTITY, |o| o.pivot().rot),
    }
}

fn shift(plan: &mut Plan, object: &Object, frame: &Mat3, axis: Axis, delta: f64) {
    if delta.abs() <= 1e-12 {
        return;
    }
    let t = Pose::translation(frame.col(axis.index()) * delta);
    plan.push_object(object, &object.transformed(&t));
}

/// Plans an align of the objects `ids` (the last is the active object).
///
/// Errors: [`ArrangeError::NeedMore`] with fewer than 2 objects (Active, Selection) or none (Grid);
/// [`ArrangeError::Invalid`] for a grid step that is not positive.
pub fn align(world: &World, ids: &[InstanceId], opts: &AlignOptions) -> Result<Plan, ArrangeError> {
    let objects = read_objects(world, ids)?;
    let needed = if matches!(opts.relative, RelativeTo::Grid(_)) {
        1
    } else {
        2
    };
    if objects.len() < needed {
        return Err(ArrangeError::NeedMore {
            needed,
            got: objects.len(),
        });
    }
    let frame = frame_for(&objects, opts.space);
    let target = match opts.relative {
        RelativeTo::Active => Some(edge_of(
            &objects[objects.len() - 1].bounds_in(&frame),
            opts.axis,
            opts.edge,
        )),
        RelativeTo::Selection => Some(edge_of(
            &selection_bounds_in(&objects, &frame),
            opts.axis,
            opts.edge,
        )),
        RelativeTo::Grid(step) if step > 0.0 && step.is_finite() => None,
        RelativeTo::Grid(_) => {
            return Err(ArrangeError::Invalid(
                "the grid step must be positive".into(),
            ));
        }
    };
    let mut plan = Plan {
        label: format!("Align {}", describe(&objects)),
        ..Plan::default()
    };
    let movers = match opts.relative {
        RelativeTo::Active => &objects[..objects.len() - 1],
        _ => &objects[..],
    };
    for o in movers {
        let value = edge_of(&o.bounds_in(&frame), opts.axis, opts.edge);
        let goal = match (target, opts.relative) {
            (Some(t), _) => t,
            (None, RelativeTo::Grid(step)) => (value / step).round() * step,
            (None, _) => value,
        };
        shift(&mut plan, o, &frame, opts.axis, goal - value);
    }
    Ok(plan)
}

/// Plans a distribute of the objects `ids` along one axis, ordered by their centres.
///
/// Errors: [`ArrangeError::NeedMore`] with fewer than 3 objects; [`ArrangeError::Invalid`] for a
/// non-finite spacing or a negative centre spacing.
pub fn distribute(
    world: &World,
    ids: &[InstanceId],
    opts: &DistributeOptions,
) -> Result<Plan, ArrangeError> {
    let objects = read_objects(world, ids)?;
    if objects.len() < 3 {
        return Err(ArrangeError::NeedMore {
            needed: 3,
            got: objects.len(),
        });
    }
    if let Some(s) = opts.spacing
        && (!s.is_finite() || (opts.mode == DistributeMode::Centres && s < 0.0))
    {
        return Err(ArrangeError::Invalid(
            "the spacing must be a finite number (centres: not negative)".into(),
        ));
    }
    let frame = frame_for(&objects, opts.space);
    let axis = opts.axis;
    let mut items: Vec<(&Object, Aabb)> =
        objects.iter().map(|o| (o, o.bounds_in(&frame))).collect();
    // Stable sort keeps selection order for equal centres.
    items.sort_by(|a, b| axis.of(a.1.centre()).total_cmp(&axis.of(b.1.centre())));
    let n = items.len();
    let mut plan = Plan {
        label: format!("Distribute {}", describe(&objects)),
        ..Plan::default()
    };
    match opts.mode {
        DistributeMode::Centres => {
            let first = axis.of(items[0].1.centre());
            let last = axis.of(items[n - 1].1.centre());
            let step = opts.spacing.unwrap_or((last - first) / (n - 1) as f64);
            for (i, (o, b)) in items.iter().enumerate() {
                let goal = first + step * i as f64;
                shift(&mut plan, o, &frame, axis, goal - axis.of(b.centre()));
            }
        }
        DistributeMode::Gaps => {
            let start = axis.of(items[0].1.min);
            let end = axis.of(items[n - 1].1.max);
            let total: f64 = items.iter().map(|(_, b)| axis.of(b.size())).sum();
            let gap = opts
                .spacing
                .unwrap_or((end - start - total) / (n - 1) as f64);
            let mut cursor = start;
            for (o, b) in &items {
                shift(&mut plan, o, &frame, axis, cursor - axis.of(b.min));
                cursor += axis.of(b.size()) + gap;
            }
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::arrange::testing::*;

    fn opts(axis: Axis, edge: Edge, relative: RelativeTo) -> AlignOptions {
        AlignOptions {
            axis,
            edge,
            relative,
            space: Space::World,
        }
    }

    #[test]
    fn align_min_to_active_object() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(2.0, 2.0, 2.0), v(0.0, 0.0, 0.0));
        let b = part(&mut w, v(5.0, 3.0, 0.0), v(4.0, 1.0, 2.0), v(0.0, 0.0, 0.0));
        let active = part(
            &mut w,
            v(10.0, 0.0, 0.0),
            v(6.0, 1.0, 1.0),
            v(0.0, 0.0, 0.0),
        );
        let plan = align(
            &w,
            &[a, b, active],
            &opts(Axis::X, Edge::Min, RelativeTo::Active),
        )
        .unwrap();
        // Active min X = 10 - 3 = 7. a: min -1 -> 7, centre 8. b: min 3 -> 7, centre 9.
        assert_vec(pos_after(&plan, a), v(8.0, 0.0, 0.0));
        assert_vec(pos_after(&plan, b), v(9.0, 3.0, 0.0));
        assert!(plan.after(active, "Position").is_none());
        assert_eq!(plan.label, "Align 3 parts");
        // Only Position changes.
        assert_eq!(plan.changes.len(), 2);
    }

    #[test]
    fn align_uses_rotated_bounding_boxes() {
        let mut w = world();
        // 4 x 1 x 2 rotated 90 degrees about Y: X extent becomes 2 (half 1).
        let r = part(
            &mut w,
            v(0.0, 0.0, 0.0),
            v(4.0, 1.0, 2.0),
            v(0.0, 90.0, 0.0),
        );
        let active = part(
            &mut w,
            v(20.0, 0.0, 0.0),
            v(2.0, 2.0, 2.0),
            v(0.0, 0.0, 0.0),
        );
        let plan = align(
            &w,
            &[r, active],
            &opts(Axis::X, Edge::Max, RelativeTo::Active),
        )
        .unwrap();
        // Active max X = 21; rotated part's max X = centre + 1 -> centre 20.
        assert_vec(pos_after(&plan, r), v(20.0, 0.0, 0.0));
        // 45 degrees: X half-extent = (4 cos45 + 2 sin45) / 2 = 3/sqrt2.
        let mut w = world();
        let r = part(
            &mut w,
            v(0.0, 0.0, 0.0),
            v(4.0, 1.0, 2.0),
            v(0.0, 45.0, 0.0),
        );
        let active = part(
            &mut w,
            v(20.0, 0.0, 0.0),
            v(2.0, 2.0, 2.0),
            v(0.0, 0.0, 0.0),
        );
        let plan = align(
            &w,
            &[r, active],
            &opts(Axis::X, Edge::Min, RelativeTo::Active),
        )
        .unwrap();
        let half = 3.0 / 2f64.sqrt();
        assert_vec(pos_after(&plan, r), v(19.0 + half, 0.0, 0.0));
    }

    #[test]
    fn align_centre_and_max_to_selection_bounds() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(2.0, 2.0, 2.0), v(0.0, 0.0, 0.0));
        let b = part(
            &mut w,
            v(0.0, 10.0, 0.0),
            v(2.0, 4.0, 2.0),
            v(0.0, 0.0, 0.0),
        );
        // Selection Y: -1 .. 12, centre 5.5.
        let plan = align(
            &w,
            &[a, b],
            &opts(Axis::Y, Edge::Centre, RelativeTo::Selection),
        )
        .unwrap();
        assert_vec(pos_after(&plan, a), v(0.0, 5.5, 0.0));
        assert_vec(pos_after(&plan, b), v(0.0, 5.5, 0.0));
        let plan = align(
            &w,
            &[a, b],
            &opts(Axis::Y, Edge::Max, RelativeTo::Selection),
        )
        .unwrap();
        assert_vec(pos_after(&plan, a), v(0.0, 11.0, 0.0));
        assert!(plan.after(b, "Position").is_none());
    }

    #[test]
    fn align_to_grid_snaps_each_edge_to_its_nearest_line() {
        let mut w = world();
        let a = part(&mut w, v(0.3, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let b = part(&mut w, v(3.9, 0.0, 0.0), v(3.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let plan = align(
            &w,
            &[a, b],
            &opts(Axis::X, Edge::Min, RelativeTo::Grid(1.0)),
        )
        .unwrap();
        // a min -0.2 -> 0, centre 0.5. b min 2.4 -> 2, centre 3.5.
        assert_vec(pos_after(&plan, a), v(0.5, 0.0, 0.0));
        assert_vec(pos_after(&plan, b), v(3.5, 0.0, 0.0));
    }

    #[test]
    fn align_in_local_space_moves_along_the_active_axes() {
        let mut w = world();
        let a = part(
            &mut w,
            v(0.0, 0.0, 0.0),
            v(1.0, 1.0, 1.0),
            v(0.0, 90.0, 0.0),
        );
        let active = part(
            &mut w,
            v(0.0, 0.0, 10.0),
            v(1.0, 1.0, 1.0),
            v(0.0, 90.0, 0.0),
        );
        // Active local X = world (0, 0, -1). Centre along local X: a at 0, active at -10.
        let o = AlignOptions {
            axis: Axis::X,
            edge: Edge::Centre,
            relative: RelativeTo::Active,
            space: Space::Local,
        };
        let plan = align(&w, &[a, active], &o).unwrap();
        assert_vec(pos_after(&plan, a), v(0.0, 0.0, 10.0));
    }

    #[test]
    fn align_moves_a_model_as_one_unit() {
        let mut w = world();
        let (m, kids) = model(
            &mut w,
            &[
                (v(0.0, 0.0, 0.0), v(2.0, 2.0, 2.0)),
                (v(4.0, 0.0, 0.0), v(2.0, 2.0, 2.0)),
            ],
        );
        let active = part(
            &mut w,
            v(0.0, 0.0, 0.0),
            v(2.0, 20.0, 2.0),
            v(0.0, 0.0, 0.0),
        );
        // Selecting a child too must not move it twice.
        let plan = align(
            &w,
            &[m, kids[0], active],
            &opts(Axis::Y, Edge::Max, RelativeTo::Active),
        )
        .unwrap();
        assert_vec(pos_after(&plan, kids[0]), v(0.0, 9.0, 0.0));
        assert_vec(pos_after(&plan, kids[1]), v(4.0, 9.0, 0.0));
        assert_eq!(plan.changes.len(), 2);
        assert_eq!(plan.label, "Align 2 objects");
    }

    #[test]
    fn align_needs_two_objects_and_a_positive_grid() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        assert!(matches!(
            align(&w, &[a], &opts(Axis::X, Edge::Min, RelativeTo::Active)),
            Err(ArrangeError::NeedMore { needed: 2, got: 1 })
        ));
        assert!(align(&w, &[a], &opts(Axis::X, Edge::Min, RelativeTo::Grid(0.0))).is_err());
    }

    fn dist(axis: Axis, mode: DistributeMode, spacing: Option<f64>) -> DistributeOptions {
        DistributeOptions {
            axis,
            mode,
            spacing,
            space: Space::World,
        }
    }

    #[test]
    fn distribute_equal_centres_keeps_the_ends() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let c = part(
            &mut w,
            v(12.0, 0.0, 0.0),
            v(1.0, 1.0, 1.0),
            v(0.0, 0.0, 0.0),
        );
        let b = part(&mut w, v(1.0, 0.0, 0.0), v(5.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let d = part(&mut w, v(2.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let plan = distribute(
            &w,
            &[a, c, b, d],
            &dist(Axis::X, DistributeMode::Centres, None),
        )
        .unwrap();
        // Sorted by centre: a 0, b 1, d 2, c 12 -> 0, 4, 8, 12.
        assert!(plan.after(a, "Position").is_none());
        assert_vec(pos_after(&plan, b), v(4.0, 0.0, 0.0));
        assert_vec(pos_after(&plan, d), v(8.0, 0.0, 0.0));
        assert!(plan.after(c, "Position").is_none());
    }

    #[test]
    fn distribute_equal_gaps_and_a_fixed_gap() {
        let mut w = world();
        // Bounds X: a [-1, 1], b [2, 6], c [19, 21]. Span 22, sizes 2 + 4 + 2 = 8, gap 7.
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(2.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let b = part(&mut w, v(4.0, 0.0, 0.0), v(4.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let c = part(
            &mut w,
            v(20.0, 0.0, 0.0),
            v(2.0, 1.0, 1.0),
            v(0.0, 0.0, 0.0),
        );
        let plan = distribute(&w, &[a, b, c], &dist(Axis::X, DistributeMode::Gaps, None)).unwrap();
        // b min = 1 + 7 = 8 -> centre 10.
        assert_vec(pos_after(&plan, b), v(10.0, 0.0, 0.0));
        assert!(plan.after(c, "Position").is_none());
        let plan = distribute(
            &w,
            &[a, b, c],
            &dist(Axis::X, DistributeMode::Gaps, Some(0.5)),
        )
        .unwrap();
        // b min 1.5 -> centre 3.5; c min 1.5 + 4 + 0.5 = 6 -> centre 7.
        assert_vec(pos_after(&plan, b), v(3.5, 0.0, 0.0));
        assert_vec(pos_after(&plan, c), v(7.0, 0.0, 0.0));
        let plan = distribute(
            &w,
            &[a, b, c],
            &dist(Axis::X, DistributeMode::Centres, Some(3.0)),
        )
        .unwrap();
        assert_vec(pos_after(&plan, b), v(3.0, 0.0, 0.0));
        assert_vec(pos_after(&plan, c), v(6.0, 0.0, 0.0));
    }

    #[test]
    fn distribute_needs_three() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let b = part(&mut w, v(3.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        assert!(matches!(
            distribute(&w, &[a, b], &dist(Axis::X, DistributeMode::Gaps, None)),
            Err(ArrangeError::NeedMore { needed: 3, got: 2 })
        ));
    }

    #[test]
    fn planning_never_writes_the_world() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let b = part(&mut w, v(3.0, 2.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let cursor = w.change_cursor();
        let plan = align(&w, &[a, b], &opts(Axis::Y, Edge::Min, RelativeTo::Active)).unwrap();
        assert!(!plan.is_empty());
        let (changes, _) = w.changes_since(cursor);
        assert_eq!(changes, metallic_core::Changes::Ids(Vec::new()));
    }
}
