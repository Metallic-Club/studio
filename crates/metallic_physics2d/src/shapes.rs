//! Turns [`Shape2d`] descriptions into backend colliders and into world-space outlines for the
//! debug view. Also merges tilemaps into boxes.
use crate::desc::{BodyId, ColliderDesc, Shape2d};
use crate::error::Physics2dError;
use crate::math::{Pose2, Vec2};
use rapier2d::prelude::{ColliderBuilder, Pose, SharedShape, Vector};

pub(crate) fn v(p: Vec2) -> Vector {
    Vector::new(p.x as f32, p.y as f32)
}

pub(crate) fn from_v(p: Vector) -> Vec2 {
    Vec2::new(f64::from(p.x), f64::from(p.y))
}

pub(crate) fn pose(p: Pose2) -> Pose {
    Pose::new(v(p.position), p.angle as f32)
}

pub(crate) fn from_pose(p: &Pose) -> Pose2 {
    Pose2::new(from_v(p.translation), f64::from(p.rotation.angle()))
}

/// A rectangle of merged tiles: column, row, width and height in tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TileRect {
    pub col: u32,
    pub row: u32,
    pub cols: u32,
    pub rows: u32,
}

/// Greedy merge: horizontal runs per row, then identical runs on consecutive rows are stacked.
/// Deterministic and linear in the number of tiles.
pub(crate) fn merge_tiles(columns: u32, solid: &[bool]) -> Vec<TileRect> {
    let width = columns as usize;
    if width == 0 {
        return Vec::new();
    }
    let rows = solid.len() / width;
    let mut done: Vec<TileRect> = Vec::new();
    // Rectangles still growing upward, keyed by their (col, cols) run.
    let mut open: Vec<TileRect> = Vec::new();
    for row in 0..rows {
        let line = &solid[row * width..(row + 1) * width];
        let mut runs: Vec<(u32, u32)> = Vec::new();
        let mut c = 0;
        while c < width {
            if line[c] {
                let start = c;
                while c < width && line[c] {
                    c += 1;
                }
                runs.push((start as u32, (c - start) as u32));
            } else {
                c += 1;
            }
        }
        let mut next_open = Vec::with_capacity(runs.len());
        for (col, cols) in runs {
            if let Some(i) = open.iter().position(|r| r.col == col && r.cols == cols) {
                let mut r = open.remove(i);
                r.rows += 1;
                next_open.push(r);
            } else {
                next_open.push(TileRect {
                    col,
                    row: row as u32,
                    cols,
                    rows: 1,
                });
            }
        }
        done.append(&mut open);
        open = next_open;
    }
    done.append(&mut open);
    done.sort_by_key(|r| (r.row, r.col));
    done
}

/// Convex hull (monotone chain), counter-clockwise, without collinear points.
pub(crate) fn convex_hull(points: &[Vec2]) -> Vec<Vec2> {
    let mut pts: Vec<Vec2> = points.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut hull: Vec<Vec2> = Vec::with_capacity(pts.len() * 2);
    for &p in &pts {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 1e-12 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower_len = hull.len() + 1;
    for &p in pts.iter().rev().skip(1) {
        while hull.len() >= lower_len
            && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 1e-12
        {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    hull
}

fn polygon_area(points: &[Vec2]) -> f64 {
    let n = points.len();
    let mut a = 0.0;
    for i in 0..n {
        let p = points[i];
        let q = points[(i + 1) % n];
        a += p.x * q.y - q.x * p.y;
    }
    a * 0.5
}

/// Builds the backend collider for a description (without material, flags or groups).
pub(crate) fn build_shape(body: BodyId, c: &ColliderDesc) -> Result<ColliderBuilder, Physics2dError> {
    let bad = |reason| Physics2dError::InvalidShape { body, reason };
    let builder = match &c.shape {
        Shape2d::Box { half_extents } => {
            ColliderBuilder::cuboid(half_extents.x as f32, half_extents.y as f32)
        }
        Shape2d::Circle { radius } => ColliderBuilder::ball(*radius as f32),
        Shape2d::Capsule {
            half_height,
            radius,
        } => ColliderBuilder::capsule_y(*half_height as f32, *radius as f32),
        Shape2d::ConvexPolygon { points } => {
            let hull = convex_hull(points);
            if hull.len() < 3 || polygon_area(&hull) < 1e-9 {
                return Err(bad("convex polygon is degenerate (points on one line)"));
            }
            ColliderBuilder::convex_polyline(hull.into_iter().map(v).collect())
                .ok_or(bad("convex polygon is degenerate"))?
        }
        Shape2d::EdgeChain { points, closed } => {
            let n = points.len() as u32;
            let mut indices: Vec<[u32; 2]> = (0..n.saturating_sub(1)).map(|i| [i, i + 1]).collect();
            if *closed && n >= 3 {
                indices.push([n - 1, 0]);
            }
            ColliderBuilder::polyline(points.iter().map(|p| v(*p)).collect(), Some(indices))
        }
        Shape2d::Tilemap {
            tile_size,
            columns,
            solid,
        } => {
            let parts: Vec<(Pose, SharedShape)> = merge_tiles(*columns, solid)
                .into_iter()
                .map(|r| {
                    let (min, max) = tile_rect_bounds(r, *tile_size);
                    let center = (min + max) * 0.5;
                    let half = (max - min) * 0.5;
                    (
                        Pose::from_translation(v(center)),
                        SharedShape::cuboid(half.x as f32, half.y as f32),
                    )
                })
                .collect();
            if parts.is_empty() {
                return Err(bad("tilemap has no solid tile"));
            }
            ColliderBuilder::compound(parts)
        }
    };
    Ok(builder.position(pose(Pose2::new(c.offset, c.angle))))
}

fn tile_rect_bounds(r: TileRect, tile: Vec2) -> (Vec2, Vec2) {
    let min = Vec2::new(f64::from(r.col) * tile.x, f64::from(r.row) * tile.y);
    let max = Vec2::new(
        f64::from(r.col + r.cols) * tile.x,
        f64::from(r.row + r.rows) * tile.y,
    );
    (min, max)
}

/// World-space geometry of one collider, for drawing.
#[derive(Debug, Clone, PartialEq)]
pub enum DebugGeometry {
    /// A filled circle; `angle` lets the view draw a spoke so rotation is visible.
    Circle {
        /// Center.
        center: Vec2,
        /// Radius.
        radius: f64,
        /// Body rotation.
        angle: f64,
    },
    /// A closed convex or rectangular outline.
    Polygon {
        /// Corners in order.
        points: Vec<Vec2>,
    },
    /// A capsule: segment `a`..`b` thickened by `radius`.
    Capsule {
        /// One end of the inner segment.
        a: Vec2,
        /// Other end of the inner segment.
        b: Vec2,
        /// Radius.
        radius: f64,
    },
    /// A chain of edges.
    Polyline {
        /// Vertices in order.
        points: Vec<Vec2>,
        /// Whether the last vertex connects back to the first.
        closed: bool,
    },
}

fn rect_points(min: Vec2, max: Vec2) -> [Vec2; 4] {
    [
        min,
        Vec2::new(max.x, min.y),
        max,
        Vec2::new(min.x, max.y),
    ]
}

/// World-space outlines of a collider attached to a body at `body_pose`.
pub(crate) fn debug_geometry(c: &ColliderDesc, body_pose: Pose2) -> Vec<DebugGeometry> {
    let local = Pose2::new(c.offset, c.angle);
    let to_world = |p: Vec2| body_pose.transform_point(local.transform_point(p));
    let angle = body_pose.angle + c.angle;
    match &c.shape {
        Shape2d::Box { half_extents } => vec![DebugGeometry::Polygon {
            points: rect_points(-*half_extents, *half_extents)
                .iter()
                .map(|p| to_world(*p))
                .collect(),
        }],
        Shape2d::Circle { radius } => vec![DebugGeometry::Circle {
            center: to_world(Vec2::ZERO),
            radius: *radius,
            angle,
        }],
        Shape2d::Capsule {
            half_height,
            radius,
        } => vec![DebugGeometry::Capsule {
            a: to_world(Vec2::new(0.0, -half_height)),
            b: to_world(Vec2::new(0.0, *half_height)),
            radius: *radius,
        }],
        Shape2d::ConvexPolygon { points } => vec![DebugGeometry::Polygon {
            points: convex_hull(points).into_iter().map(to_world).collect(),
        }],
        Shape2d::EdgeChain { points, closed } => vec![DebugGeometry::Polyline {
            points: points.iter().map(|p| to_world(*p)).collect(),
            closed: *closed,
        }],
        Shape2d::Tilemap {
            tile_size,
            columns,
            solid,
        } => merge_tiles(*columns, solid)
            .into_iter()
            .map(|r| {
                let (min, max) = tile_rect_bounds(r, *tile_size);
                DebugGeometry::Polygon {
                    points: rect_points(min, max).iter().map(|p| to_world(*p)).collect(),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_merge_into_few_rects() {
        // 4 columns, 3 rows:
        // row 2: . X X .
        // row 1: X X X X
        // row 0: X X X X
        let solid = [
            true, true, true, true, //
            true, true, true, true, //
            false, true, true, false,
        ];
        let rects = merge_tiles(4, &solid);
        assert_eq!(
            rects,
            vec![
                TileRect {
                    col: 0,
                    row: 0,
                    cols: 4,
                    rows: 2
                },
                TileRect {
                    col: 1,
                    row: 2,
                    cols: 2,
                    rows: 1
                },
            ]
        );
    }

    #[test]
    fn merged_tiles_cover_exactly_the_solid_tiles() {
        // A pseudo-random pattern; every solid tile is covered once, no empty tile is covered.
        let columns = 13u32;
        let solid: Vec<bool> = (0..13 * 9).map(|i| (i * 7919 + i / 5) % 3 != 0).collect();
        let mut covered = vec![0u8; solid.len()];
        for r in merge_tiles(columns, &solid) {
            for row in r.row..r.row + r.rows {
                for col in r.col..r.col + r.cols {
                    covered[(row * columns + col) as usize] += 1;
                }
            }
        }
        for (i, s) in solid.iter().enumerate() {
            assert_eq!(covered[i], u8::from(*s), "tile {i}");
        }
    }

    #[test]
    fn hull_drops_inner_and_collinear_points() {
        let pts = [
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(1.0, 1.0),
        ];
        let h = convex_hull(&pts);
        assert_eq!(h.len(), 4);
        assert!(polygon_area(&h) > 3.99);
    }
}
