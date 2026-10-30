//! Pure cores of the building tools (design spec `docs/design/building-tools.md`).
//!
//! Nothing here draws or reads input. Only [`geom::snapshot`] reads the `metallic_core` world;
//! every core takes plain snapshots (positions, normals, screen rectangles, part traits) and returns plain
//! results. The viewport, Explorer and `History` from Task 10 (#10) wire them up later:
//!
//! - [`scatter`]: the brush scatter tool (B, spec section 9, issue #219). Poisson-disk
//!   placement on surfaces through a [`scatter::SurfaceProbe`], slope and surface filters,
//!   random yaw and scale, alignment to the surface normal, erase by brush tag, and one
//!   [`scatter::StrokeCommit`] per stroke for the undo history.
//! - [`select`]: selection modes (box, lasso, paint; replace, add, subtract) and Select
//!   similar (spec 10.1, issue #220).
//! - [`measure`]: point-to-point measurements, vertex snapping and gap readouts between
//!   bounding boxes (spec 10.2, issue #220).
//! - [`geom`]: the shared [`geom::PartSnapshot`], [`geom::Aabb`], box corners and gaps. The
//!   arrange ops (#307) move onto it next.
//!
//! [`rng`] is the small seeded generator the scatter tool uses so a seed always gives the
//! same placements.

// pub mod arrange;
pub mod geom;
pub mod measure;
pub mod rng;
pub mod scatter;
pub mod select;
