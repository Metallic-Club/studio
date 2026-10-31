//! Physics debug draw (#23): what the simulation sees, as plain data for a renderer to draw.
//!
//! [`PhysicsWorld::debug_draw`] fills a [`DebugDraw`] with one [`DebugShape`] per collider (its
//! shape, pose and body state), the contact points with their normals, and the velocities of
//! moving bodies. [`crate::CharacterController::debug_touch_volumes`] adds the characters'
//! bone-attached touch capsules. Nothing is computed unless asked for; the buffers are reused.
use crate::{PhysicsWorld, from_vector};
use metallic_core::{InstanceId, Vec3};

/// What to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DebugDrawOptions {
    /// Collider shapes, coloured by body state.
    pub shapes: bool,
    /// Contact points and normals.
    pub contacts: bool,
    /// Velocities of moving bodies.
    pub velocities: bool,
    /// The characters' touch volumes.
    pub touch_volumes: bool,
}

impl DebugDrawOptions {
    /// Everything on.
    pub const ALL: DebugDrawOptions = DebugDrawOptions {
        shapes: true,
        contacts: true,
        velocities: true,
        touch_volumes: true,
    };

    /// True when anything is on.
    pub fn any(self) -> bool {
        self.shapes || self.contacts || self.velocities || self.touch_volumes
    }
}

/// The shape of a collider, in a part's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugShapeKind {
    /// A box of `size`.
    Block,
    /// A sphere; `size` is its diameter on every axis.
    Ball,
    /// A cylinder along local X: `size.x` long, `size.y` (= `size.z`) across.
    Cylinder,
    /// A wedge (its tall edge at +Z) filling `size`.
    Wedge,
}

/// How the simulation treats a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugBody {
    /// Anchored and not moving.
    Fixed,
    /// Anchored and moved this step (it carries and pushes what it touches).
    Kinematic,
    /// Simulated and awake.
    Dynamic,
    /// Simulated, at rest and asleep.
    Sleeping,
}

/// One collider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugShape {
    /// The part it belongs to.
    pub part: InstanceId,
    /// Its shape.
    pub kind: DebugShapeKind,
    /// Centre in world space.
    pub position: Vec3,
    /// Rotation as a unit quaternion `[x, y, z, w]`.
    pub rotation: [f64; 4],
    /// Size in studs (see [`DebugShapeKind`]).
    pub size: Vec3,
    /// Body state.
    pub body: DebugBody,
    /// CanCollide off: it reports touches but never blocks.
    pub sensor: bool,
}

/// One contact point between two colliders.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugContact {
    /// The point in world space.
    pub point: Vec3,
    /// Unit normal from the first collider toward the second.
    pub normal: Vec3,
}

/// A moving body's velocity, from its centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugVelocity {
    /// The body's centre.
    pub from: Vec3,
    /// Velocity in studs per second.
    pub velocity: Vec3,
}

/// A character's touch volume: a capsule from `a` to `b`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugCapsule {
    /// One end of the segment.
    pub a: Vec3,
    /// The other end.
    pub b: Vec3,
    /// Radius in studs, touch margin included.
    pub radius: f64,
    /// It touches a part now.
    pub touching: bool,
}

/// The debug draw of one frame; reuse it, [`DebugDraw::clear`] keeps the capacity.
#[derive(Debug, Clone, Default)]
pub struct DebugDraw {
    /// One per collider (with [`DebugDrawOptions::shapes`]).
    pub shapes: Vec<DebugShape>,
    /// Active contact points (with [`DebugDrawOptions::contacts`]).
    pub contacts: Vec<DebugContact>,
    /// Bodies moving faster than [`MIN_SPEED`] (with [`DebugDrawOptions::velocities`]).
    pub velocities: Vec<DebugVelocity>,
    /// Touch volumes (with [`DebugDrawOptions::touch_volumes`]).
    pub touch_volumes: Vec<DebugCapsule>,
}

impl DebugDraw {
    /// Empties every list.
    pub fn clear(&mut self) {
        self.shapes.clear();
        self.contacts.clear();
        self.velocities.clear();
        self.touch_volumes.clear();
    }
}

/// Bodies slower than this (studs per second) get no velocity arrow.
pub const MIN_SPEED: f64 = 0.05;

fn kind_of(shape: &str) -> DebugShapeKind {
    match shape {
        "Ball" => DebugShapeKind::Ball,
        "Cylinder" => DebugShapeKind::Cylinder,
        "Wedge" => DebugShapeKind::Wedge,
        _ => DebugShapeKind::Block,
    }
}

/// The collider's extent for a part of `size` (as `sync::build_collider` builds it).
fn collider_size(kind: DebugShapeKind, size: Vec3) -> Vec3 {
    match kind {
        DebugShapeKind::Ball => {
            let d = size.x.min(size.y).min(size.z);
            Vec3::new(d, d, d)
        }
        DebugShapeKind::Cylinder => {
            let d = size.y.min(size.z);
            Vec3::new(size.x, d, d)
        }
        DebugShapeKind::Block | DebugShapeKind::Wedge => size,
    }
}

impl PhysicsWorld {
    /// Adds what `options` asks for to `out` (cleared first is up to the caller).
    pub fn debug_draw(&self, options: DebugDrawOptions, out: &mut DebugDraw) {
        if options.shapes || options.velocities {
            for (&part, entry) in &self.entries {
                let Some(body) = self.rapier.bodies.get(entry.body) else {
                    continue;
                };
                let pose = body.position();
                let position = from_vector(pose.translation);
                if options.shapes {
                    let kind = kind_of(&entry.signature.shape);
                    let state = if body.is_fixed() {
                        DebugBody::Fixed
                    } else if body.is_kinematic() {
                        DebugBody::Kinematic
                    } else if body.is_sleeping() {
                        DebugBody::Sleeping
                    } else {
                        DebugBody::Dynamic
                    };
                    let q = pose.rotation;
                    out.shapes.push(DebugShape {
                        part,
                        kind,
                        position,
                        rotation: [q.x, q.y, q.z, q.w].map(f64::from),
                        size: collider_size(kind, entry.signature.size),
                        body: state,
                        sensor: !entry.signature.can_collide,
                    });
                }
                if options.velocities && !body.is_fixed() {
                    let velocity = if body.is_kinematic() {
                        entry.derived_velocity
                    } else {
                        from_vector(body.linvel())
                    };
                    if velocity.magnitude() > MIN_SPEED {
                        out.velocities.push(DebugVelocity {
                            from: position,
                            velocity,
                        });
                    }
                }
            }
        }
        if options.contacts {
            for pair in self.rapier.narrow_phase.contact_pairs() {
                if !pair.has_any_active_contact() {
                    continue;
                }
                let Some(collider) = self.rapier.colliders.get(pair.collider1) else {
                    continue;
                };
                let pose = collider.position();
                for manifold in pair.manifolds() {
                    let normal = from_vector(manifold.data.normal);
                    for p in &manifold.points {
                        let point = from_vector(pose.translation + pose.rotation * p.local_p1);
                        out.contacts.push(DebugContact { point, normal });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::{Value, World};

    fn part(world: &mut World, name: &str, shape: &str, pos: Vec3, size: Vec3, anchored: bool) {
        let ws = world.root("Workspace").unwrap();
        let p = world.create("Part", Some(ws)).unwrap();
        world.set(p, "Name", Value::String(name.into())).unwrap();
        world.set(p, "Shape", Value::String(shape.into())).unwrap();
        world.set(p, "Position", Value::Vector3(pos)).unwrap();
        world.set(p, "Size", Value::Vector3(size)).unwrap();
        world.set(p, "Anchored", Value::Bool(anchored)).unwrap();
    }

    #[test]
    fn debug_draw_emits_one_box_per_collider() {
        let mut world = World::new_with_services();
        let size = Vec3::new(4.0, 1.0, 2.0);
        part(
            &mut world,
            "A",
            "Block",
            Vec3::new(0.0, 0.0, 0.0),
            size,
            true,
        );
        part(
            &mut world,
            "B",
            "Block",
            Vec3::new(10.0, 5.0, 0.0),
            size,
            false,
        );
        part(
            &mut world,
            "C",
            "Ball",
            Vec3::new(-10.0, 5.0, 0.0),
            size,
            false,
        );
        let mut physics = PhysicsWorld::new();
        physics.sync_from(&world);
        let mut out = DebugDraw::default();
        physics.debug_draw(DebugDrawOptions::ALL, &mut out);
        assert_eq!(out.shapes.len(), 3);
        let blocks: Vec<_> = out
            .shapes
            .iter()
            .filter(|s| s.kind == DebugShapeKind::Block)
            .collect();
        assert_eq!(blocks.len(), 2);
        for b in &blocks {
            assert_eq!(b.size, size);
        }
        let fixed = blocks.iter().find(|s| s.body == DebugBody::Fixed).unwrap();
        assert!(fixed.position.magnitude() < 1e-6);
        let ball = out
            .shapes
            .iter()
            .find(|s| s.kind == DebugShapeKind::Ball)
            .unwrap();
        assert_eq!(ball.size, Vec3::new(1.0, 1.0, 1.0));
        assert_eq!(ball.body, DebugBody::Dynamic);

        // Nothing asked, nothing drawn.
        out.clear();
        physics.debug_draw(DebugDrawOptions::default(), &mut out);
        assert!(out.shapes.is_empty() && out.contacts.is_empty() && out.velocities.is_empty());
    }

    #[test]
    fn a_resting_body_shows_contacts_and_a_falling_one_its_velocity() {
        let mut world = World::new_with_services();
        part(
            &mut world,
            "Floor",
            "Block",
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(40.0, 1.0, 40.0),
            true,
        );
        part(
            &mut world,
            "Box",
            "Block",
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(2.0, 2.0, 2.0),
            false,
        );
        part(
            &mut world,
            "Falling",
            "Block",
            Vec3::new(12.0, 30.0, 0.0),
            Vec3::new(2.0, 2.0, 2.0),
            false,
        );
        let mut physics = PhysicsWorld::new();
        physics.sync_from(&world);
        for _ in 0..10 {
            physics.step(&mut world, 1.0 / 60.0);
        }
        let mut out = DebugDraw::default();
        physics.debug_draw(DebugDrawOptions::ALL, &mut out);
        assert!(!out.contacts.is_empty(), "the box rests on the floor");
        for c in &out.contacts {
            assert!(
                c.point.y.abs() < 0.2,
                "contact at the floor top: {:?}",
                c.point
            );
            assert!((c.normal.magnitude() - 1.0).abs() < 1e-3);
        }
        let falling = out
            .velocities
            .iter()
            .find(|v| v.from.x > 10.0)
            .expect("the falling box moves");
        assert!(falling.velocity.y < -10.0);
    }
}
