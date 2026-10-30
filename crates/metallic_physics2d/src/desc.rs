//! Plain data descriptions of a 2D scene: bodies, colliders, shapes and joints.
//!
//! These structs are the whole boundary between game code and the physics backend. They do not
//! depend on the Instance tree, so the M3 decision on 2D classes (see
//! `docs/agents/126-physics2d.md`) only changes the code that fills them in.
use crate::error::Physics2dError;
use crate::math::Vec2;

/// Caller-chosen stable id of a body (for example an Instance id turned into a number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BodyId(pub u64);

/// Caller-chosen stable id of a joint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JointId(pub u64);

/// How a body moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// Never moves by itself (ground, walls). Moving it in the scene makes it a mover for that
    /// step, like an anchored Part in 3D: things standing on it are carried.
    Static,
    /// Simulated: gravity, contacts, joints and impulses move it.
    Dynamic,
    /// Moved only by the caller (moving platforms, doors); pushes dynamic bodies and carries
    /// characters, is never pushed back.
    Kinematic,
}

/// The geometry of one collider, in the collider's local frame (studs).
#[derive(Debug, Clone, PartialEq)]
pub enum Shape2d {
    /// Axis-aligned box (before rotation) with the given half width and half height.
    Box {
        /// Half width and half height.
        half_extents: Vec2,
    },
    /// Circle around the local origin.
    Circle {
        /// Radius.
        radius: f64,
    },
    /// Vertical capsule: a segment from `(0, -half_height)` to `(0, half_height)` thickened by
    /// `radius`. Total height is `2 * (half_height + radius)`.
    Capsule {
        /// Half length of the inner segment.
        half_height: f64,
        /// Radius of the rounded ends.
        radius: f64,
    },
    /// Convex polygon: the convex hull of the points (3 to 64 points, not all on one line).
    ConvexPolygon {
        /// Corner points; order does not matter.
        points: Vec<Vec2>,
    },
    /// A chain of edges (terrain, outlines). Hollow: it only collides along its edges.
    EdgeChain {
        /// Vertices in order (at least 2).
        points: Vec<Vec2>,
        /// Connects the last vertex back to the first.
        closed: bool,
    },
    /// A grid of solid tiles merged into as few boxes as possible (fewer seams to catch on,
    /// far fewer colliders than one box per tile). Tile `(col, row)` covers
    /// `[col * w, (col + 1) * w] x [row * h, (row + 1) * h]`; row 0 is at the bottom.
    Tilemap {
        /// Size of one tile.
        tile_size: Vec2,
        /// Number of columns.
        columns: u32,
        /// Row-major solid flags, `columns` per row, bottom row first.
        solid: Vec<bool>,
    },
}

/// Surface and mass properties of a collider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material2d {
    /// Mass per square stud (dynamic bodies only).
    pub density: f64,
    /// Coulomb friction coefficient (0 = ice).
    pub friction: f64,
    /// Bounciness from 0 (none) to 1 (perfectly elastic).
    pub restitution: f64,
}

impl Default for Material2d {
    fn default() -> Self {
        Material2d {
            density: 1.0,
            friction: 0.6,
            restitution: 0.0,
        }
    }
}

/// Collision filtering: two colliders interact only when each one's `memberships` intersects
/// the other's `filter`. Up to 32 groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionGroups {
    /// Groups this collider belongs to (bit mask).
    pub memberships: u32,
    /// Groups this collider interacts with (bit mask).
    pub filter: u32,
}

impl CollisionGroups {
    /// In every group, interacts with every group (the default).
    pub const ALL: CollisionGroups = CollisionGroups {
        memberships: u32::MAX,
        filter: u32::MAX,
    };

    /// True when colliders with these two settings interact.
    pub fn interacts_with(self, other: CollisionGroups) -> bool {
        self.memberships & other.filter != 0 && other.memberships & self.filter != 0
    }
}

impl Default for CollisionGroups {
    fn default() -> Self {
        CollisionGroups::ALL
    }
}

/// One collider attached to a body.
#[derive(Debug, Clone, PartialEq)]
pub struct ColliderDesc {
    /// The geometry.
    pub shape: Shape2d,
    /// Offset of the shape from the body origin, in body space.
    pub offset: Vec2,
    /// Rotation of the shape relative to the body, radians counter-clockwise.
    pub angle: f64,
    /// Density, friction, restitution.
    pub material: Material2d,
    /// A sensor (trigger) reports enter/exit but never blocks anything.
    pub sensor: bool,
    /// One-way platform: `Some(n)` makes the collider solid only for things arriving from the
    /// side `n` points to (body space; `Vec2::UP` is the usual "jump up through, land on top").
    pub one_way: Option<Vec2>,
    /// Collision filtering.
    pub groups: CollisionGroups,
}

impl ColliderDesc {
    /// A solid collider with the default material, no offset and default groups.
    pub fn new(shape: Shape2d) -> Self {
        ColliderDesc {
            shape,
            offset: Vec2::ZERO,
            angle: 0.0,
            material: Material2d::default(),
            sensor: false,
            one_way: None,
            groups: CollisionGroups::ALL,
        }
    }

    /// Shorthand for a box collider with the given full width and height.
    pub fn rect(width: f64, height: f64) -> Self {
        ColliderDesc::new(Shape2d::Box {
            half_extents: Vec2::new(width * 0.5, height * 0.5),
        })
    }

    /// Shorthand for a circle collider.
    pub fn circle(radius: f64) -> Self {
        ColliderDesc::new(Shape2d::Circle { radius })
    }

    /// Makes this collider a sensor (trigger).
    pub fn sensor(mut self) -> Self {
        self.sensor = true;
        self
    }

    /// Makes this collider a one-way platform that is solid from above (body space).
    pub fn one_way_up(mut self) -> Self {
        self.one_way = Some(Vec2::UP);
        self
    }

    /// Sets the material.
    pub fn with_material(mut self, material: Material2d) -> Self {
        self.material = material;
        self
    }

    /// Sets the offset from the body origin.
    pub fn with_offset(mut self, offset: Vec2) -> Self {
        self.offset = offset;
        self
    }

    /// Sets the collision groups.
    pub fn with_groups(mut self, groups: CollisionGroups) -> Self {
        self.groups = groups;
        self
    }
}

/// One body in a [`Scene2d`].
#[derive(Debug, Clone, PartialEq)]
pub struct BodyDesc {
    /// Stable id chosen by the caller; unique within the scene.
    pub id: BodyId,
    /// Static, dynamic or kinematic.
    pub kind: BodyKind,
    /// World position of the body origin (written back for dynamic bodies by `step`).
    pub position: Vec2,
    /// Rotation, radians counter-clockwise (written back for dynamic bodies by `step`).
    pub angle: f64,
    /// Colliders attached to the body. A dynamic body needs at least one with density > 0
    /// to have mass.
    pub colliders: Vec<ColliderDesc>,
    /// Prevents rotation (crates that must stay upright).
    pub fixed_rotation: bool,
    /// Multiplier on world gravity (0 = floats).
    pub gravity_scale: f64,
    /// Linear velocity damping per second.
    pub linear_damping: f64,
    /// Angular velocity damping per second.
    pub angular_damping: f64,
    /// Continuous collision detection: fast bodies cannot tunnel through thin walls.
    pub bullet: bool,
}

impl BodyDesc {
    /// A body of `kind` at `position` with no colliders and default settings.
    pub fn new(id: BodyId, kind: BodyKind, position: Vec2) -> Self {
        BodyDesc {
            id,
            kind,
            position,
            angle: 0.0,
            colliders: Vec::new(),
            fixed_rotation: false,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            bullet: false,
        }
    }

    /// Adds a collider.
    pub fn with_collider(mut self, collider: ColliderDesc) -> Self {
        self.colliders.push(collider);
        self
    }

    /// Sets the rotation.
    pub fn with_angle(mut self, angle: f64) -> Self {
        self.angle = angle;
        self
    }

    /// Enables continuous collision detection.
    pub fn bullet(mut self) -> Self {
        self.bullet = true;
        self
    }

    /// The part of the description that requires rebuilding the backend body when changed
    /// (everything except the pose).
    pub(crate) fn structure_eq(&self, other: &BodyDesc) -> bool {
        self.kind == other.kind
            && self.colliders == other.colliders
            && self.fixed_rotation == other.fixed_rotation
            && self.gravity_scale == other.gravity_scale
            && self.linear_damping == other.linear_damping
            && self.angular_damping == other.angular_damping
            && self.bullet == other.bullet
    }
}

/// A motor driving a joint axis toward a target speed with a force/torque limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointMotor {
    /// Target speed (radians/s for revolute, studs/s for prismatic).
    pub target_velocity: f64,
    /// Largest force or torque the motor may apply.
    pub max_force: f64,
}

/// Spring settings for a distance joint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    /// Length the spring pulls toward.
    pub rest_length: f64,
    /// Stiffness (force per stud of stretch).
    pub stiffness: f64,
    /// Damping (force per stud/s).
    pub damping: f64,
}

/// What a joint allows. Anchors are in each body's local space.
#[derive(Debug, Clone, PartialEq)]
pub enum JointKind {
    /// A pin: the anchors coincide, the bodies rotate freely (wheels, pendulums, doors).
    Revolute {
        /// Anchor on body A.
        anchor_a: Vec2,
        /// Anchor on body B.
        anchor_b: Vec2,
        /// Optional `(min, max)` relative angle in radians.
        limits: Option<(f64, f64)>,
        /// Optional motor.
        motor: Option<JointMotor>,
    },
    /// A slider along `axis` (in body A space): no relative rotation.
    Prismatic {
        /// Anchor on body A.
        anchor_a: Vec2,
        /// Anchor on body B.
        anchor_b: Vec2,
        /// Slide direction in body A space.
        axis: Vec2,
        /// Optional `(min, max)` travel in studs.
        limits: Option<(f64, f64)>,
        /// Optional motor.
        motor: Option<JointMotor>,
    },
    /// Keeps the anchor distance between `min_length` and `max_length` (rope: `0..len`,
    /// rod: `len..len`), optionally with a spring.
    Distance {
        /// Anchor on body A.
        anchor_a: Vec2,
        /// Anchor on body B.
        anchor_b: Vec2,
        /// Shortest allowed distance.
        min_length: f64,
        /// Longest allowed distance.
        max_length: f64,
        /// Optional spring pulling toward a rest length.
        spring: Option<Spring>,
    },
    /// Glues two bodies together at the anchors (breakable structures later).
    Weld {
        /// Anchor on body A.
        anchor_a: Vec2,
        /// Anchor on body B.
        anchor_b: Vec2,
    },
}

/// A joint between two bodies of a [`Scene2d`].
#[derive(Debug, Clone, PartialEq)]
pub struct JointDesc {
    /// Stable id chosen by the caller.
    pub id: JointId,
    /// First body.
    pub body_a: BodyId,
    /// Second body.
    pub body_b: BodyId,
    /// What the joint allows.
    pub kind: JointKind,
    /// Whether the two joined bodies still collide with each other.
    pub collide_connected: bool,
}

/// The whole 2D scene as plain data: what [`crate::PhysicsWorld2d::sync_from`] mirrors and
/// [`crate::PhysicsWorld2d::step`] writes dynamic poses back into.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene2d {
    /// All bodies.
    pub bodies: Vec<BodyDesc>,
    /// All joints.
    pub joints: Vec<JointDesc>,
}

impl Scene2d {
    /// The body with this id, if any.
    pub fn body(&self, id: BodyId) -> Option<&BodyDesc> {
        self.bodies.iter().find(|b| b.id == id)
    }

    /// The body with this id, mutably, if any.
    pub fn body_mut(&mut self, id: BodyId) -> Option<&mut BodyDesc> {
        self.bodies.iter_mut().find(|b| b.id == id)
    }
}

fn finite(body: BodyId, field: &'static str, v: f64) -> Result<(), Physics2dError> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(Physics2dError::NonFinite { body, field })
    }
}

fn finite2(body: BodyId, field: &'static str, v: Vec2) -> Result<(), Physics2dError> {
    finite(body, field, v.x)?;
    finite(body, field, v.y)
}

fn shape_err(body: BodyId, reason: &'static str) -> Physics2dError {
    Physics2dError::InvalidShape { body, reason }
}

/// Largest number of points accepted for a convex polygon.
pub const MAX_POLYGON_POINTS: usize = 64;

impl Shape2d {
    pub(crate) fn validate(&self, body: BodyId) -> Result<(), Physics2dError> {
        match self {
            Shape2d::Box { half_extents } => {
                finite2(body, "box half_extents", *half_extents)?;
                if half_extents.x <= 0.0 || half_extents.y <= 0.0 {
                    return Err(shape_err(body, "box half extents must be positive"));
                }
            }
            Shape2d::Circle { radius } => {
                finite(body, "circle radius", *radius)?;
                if *radius <= 0.0 {
                    return Err(shape_err(body, "circle radius must be positive"));
                }
            }
            Shape2d::Capsule {
                half_height,
                radius,
            } => {
                finite(body, "capsule half_height", *half_height)?;
                finite(body, "capsule radius", *radius)?;
                if *radius <= 0.0 || *half_height < 0.0 {
                    return Err(shape_err(body, "capsule radius must be positive"));
                }
            }
            Shape2d::ConvexPolygon { points } => {
                if points.len() < 3 || points.len() > MAX_POLYGON_POINTS {
                    return Err(shape_err(body, "convex polygon needs 3 to 64 points"));
                }
                for p in points {
                    finite2(body, "polygon point", *p)?;
                }
            }
            Shape2d::EdgeChain { points, .. } => {
                if points.len() < 2 {
                    return Err(shape_err(body, "edge chain needs at least 2 points"));
                }
                for p in points {
                    finite2(body, "edge chain point", *p)?;
                }
            }
            Shape2d::Tilemap {
                tile_size,
                columns,
                solid,
            } => {
                finite2(body, "tile_size", *tile_size)?;
                if tile_size.x <= 0.0 || tile_size.y <= 0.0 {
                    return Err(shape_err(body, "tile size must be positive"));
                }
                if *columns == 0 || solid.len() % (*columns as usize) != 0 {
                    return Err(shape_err(
                        body,
                        "tilemap needs columns > 0 and a whole number of rows",
                    ));
                }
                if !solid.iter().any(|s| *s) {
                    return Err(shape_err(body, "tilemap has no solid tile"));
                }
            }
        }
        Ok(())
    }
}

impl BodyDesc {
    /// Checks every number and shape; returns the first problem found.
    pub fn validate(&self) -> Result<(), Physics2dError> {
        let id = self.id;
        finite2(id, "position", self.position)?;
        finite(id, "angle", self.angle)?;
        finite(id, "gravity_scale", self.gravity_scale)?;
        finite(id, "linear_damping", self.linear_damping)?;
        finite(id, "angular_damping", self.angular_damping)?;
        if self.linear_damping < 0.0 || self.angular_damping < 0.0 {
            return Err(Physics2dError::InvalidConfig("damping must not be negative"));
        }
        for c in &self.colliders {
            finite2(id, "collider offset", c.offset)?;
            finite(id, "collider angle", c.angle)?;
            finite(id, "density", c.material.density)?;
            finite(id, "friction", c.material.friction)?;
            finite(id, "restitution", c.material.restitution)?;
            if c.material.density < 0.0 || c.material.friction < 0.0 {
                return Err(shape_err(id, "density and friction must not be negative"));
            }
            if let Some(n) = c.one_way {
                if n.normalized().is_none() {
                    return Err(shape_err(id, "one-way direction must be a non-zero vector"));
                }
                if c.sensor {
                    return Err(shape_err(id, "a sensor cannot be a one-way platform"));
                }
            }
            c.shape.validate(id)?;
        }
        Ok(())
    }
}

impl JointDesc {
    /// Checks every number; returns the first problem found.
    pub fn validate(&self) -> Result<(), Physics2dError> {
        let b = self.body_a;
        let bad = |reason| Physics2dError::InvalidJoint {
            joint: self.id,
            reason,
        };
        if self.body_a == self.body_b {
            return Err(bad("a joint needs two different bodies"));
        }
        let check_limits = |l: Option<(f64, f64)>| -> Result<(), Physics2dError> {
            if let Some((lo, hi)) = l {
                finite(b, "joint limit", lo)?;
                finite(b, "joint limit", hi)?;
                if lo > hi {
                    return Err(bad("joint limits must have min <= max"));
                }
            }
            Ok(())
        };
        let check_motor = |m: Option<JointMotor>| -> Result<(), Physics2dError> {
            if let Some(m) = m {
                finite(b, "motor target_velocity", m.target_velocity)?;
                finite(b, "motor max_force", m.max_force)?;
                if m.max_force < 0.0 {
                    return Err(bad("motor max_force must not be negative"));
                }
            }
            Ok(())
        };
        match &self.kind {
            JointKind::Revolute {
                anchor_a,
                anchor_b,
                limits,
                motor,
            } => {
                finite2(b, "joint anchor", *anchor_a)?;
                finite2(b, "joint anchor", *anchor_b)?;
                check_limits(*limits)?;
                check_motor(*motor)?;
            }
            JointKind::Prismatic {
                anchor_a,
                anchor_b,
                axis,
                limits,
                motor,
            } => {
                finite2(b, "joint anchor", *anchor_a)?;
                finite2(b, "joint anchor", *anchor_b)?;
                if axis.normalized().is_none() {
                    return Err(bad("prismatic axis must be a non-zero vector"));
                }
                check_limits(*limits)?;
                check_motor(*motor)?;
            }
            JointKind::Distance {
                anchor_a,
                anchor_b,
                min_length,
                max_length,
                spring,
            } => {
                finite2(b, "joint anchor", *anchor_a)?;
                finite2(b, "joint anchor", *anchor_b)?;
                finite(b, "min_length", *min_length)?;
                finite(b, "max_length", *max_length)?;
                if *min_length < 0.0 || *max_length <= 0.0 || min_length > max_length {
                    return Err(bad("distance joint needs 0 <= min_length <= max_length, max > 0"));
                }
                if let Some(s) = spring {
                    finite(b, "spring rest_length", s.rest_length)?;
                    finite(b, "spring stiffness", s.stiffness)?;
                    finite(b, "spring damping", s.damping)?;
                    if s.stiffness < 0.0 || s.damping < 0.0 || s.rest_length < 0.0 {
                        return Err(bad("spring values must not be negative"));
                    }
                }
            }
            JointKind::Weld { anchor_a, anchor_b } => {
                finite2(b, "joint anchor", *anchor_a)?;
                finite2(b, "joint anchor", *anchor_b)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_interact_both_ways() {
        let player = CollisionGroups {
            memberships: 0b01,
            filter: 0b10,
        };
        let wall = CollisionGroups {
            memberships: 0b10,
            filter: 0b01,
        };
        let ghost = CollisionGroups {
            memberships: 0b100,
            filter: 0,
        };
        assert!(player.interacts_with(wall));
        assert!(!player.interacts_with(ghost));
        assert!(CollisionGroups::ALL.interacts_with(CollisionGroups::ALL));
    }

    #[test]
    fn validation_catches_bad_numbers() {
        let b = BodyDesc::new(BodyId(1), BodyKind::Dynamic, Vec2::new(f64::NAN, 0.0))
            .with_collider(ColliderDesc::rect(1.0, 1.0));
        assert!(matches!(
            b.validate(),
            Err(Physics2dError::NonFinite { field: "position", .. })
        ));
        let b = BodyDesc::new(BodyId(1), BodyKind::Dynamic, Vec2::ZERO)
            .with_collider(ColliderDesc::rect(0.0, 1.0));
        assert!(matches!(b.validate(), Err(Physics2dError::InvalidShape { .. })));
        let b = BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::ZERO).with_collider(
            ColliderDesc::new(Shape2d::Tilemap {
                tile_size: Vec2::new(1.0, 1.0),
                columns: 3,
                solid: vec![true; 4],
            }),
        );
        assert!(b.validate().is_err());
    }
}
