//! Physics debug draw for a running game (#23): turns `metallic_physics`'s debug data into
//! renderer wireframes ([`DebugShape`]).
//!
//! Colours (restrained, on the graphite UI's palette): fixed bodies silver, moved anchored
//! parts amber, awake simulated bodies violet, sleeping ones slate, sensors teal; contacts red
//! with their normals, velocities pale yellow, touch volumes green (orange while touching).
use metallic_core::{Vec3, World};
use metallic_physics::{
    CharacterController, DebugBody, DebugDraw, DebugDrawOptions, DebugShapeKind, PhysicsWorld,
};
use metallic_render::DebugShape;
use metallic_render::glam::{Quat, Vec3 as GVec3};
use metallic_render::meshes::MeshKind;

const FIXED: [f32; 3] = [0.55, 0.57, 0.62];
const KINEMATIC: [f32; 3] = [0.86, 0.68, 0.38];
const DYNAMIC: [f32; 3] = [0.62, 0.5, 0.95];
const SLEEPING: [f32; 3] = [0.3, 0.34, 0.44];
const SENSOR: [f32; 3] = [0.4, 0.72, 0.7];
const CONTACT: [f32; 3] = [0.95, 0.35, 0.3];
const VELOCITY: [f32; 3] = [0.95, 0.88, 0.5];
const TOUCH: [f32; 3] = [0.4, 0.62, 0.48];
const TOUCHING: [f32; 3] = [0.98, 0.6, 0.3];

/// Contacts drawn at most (a pile of parts can have thousands).
pub const MAX_CONTACTS: usize = 512;
/// Seconds of motion a velocity line shows.
const VELOCITY_SECONDS: f64 = 0.15;

fn g(v: Vec3) -> GVec3 {
    GVec3::new(v.x as f32, v.y as f32, v.z as f32)
}

fn mesh(kind: DebugShapeKind) -> MeshKind {
    match kind {
        DebugShapeKind::Block => MeshKind::Box,
        DebugShapeKind::Ball => MeshKind::Sphere,
        DebugShapeKind::Cylinder => MeshKind::Cylinder,
        DebugShapeKind::Wedge => MeshKind::Wedge,
    }
}

/// The debug draw options of a game and its reused buffers.
#[derive(Default)]
pub(crate) struct DebugOverlay {
    pub options: DebugDrawOptions,
    draw: DebugDraw,
}

impl DebugOverlay {
    /// Fills `out` (cleared first) with what the options ask for.
    pub fn shapes(
        &mut self,
        physics: &PhysicsWorld,
        characters: &CharacterController,
        world: &World,
        out: &mut Vec<DebugShape>,
    ) {
        out.clear();
        self.draw.clear();
        if !self.options.any() {
            return;
        }
        physics.debug_draw(self.options, &mut self.draw);
        if self.options.touch_volumes {
            characters.debug_touch_volumes(world, &mut self.draw.touch_volumes);
        }
        for s in &self.draw.shapes {
            let [x, y, z, w] = s.rotation.map(|c| c as f32);
            let colour = if s.sensor {
                SENSOR
            } else {
                match s.body {
                    DebugBody::Fixed => FIXED,
                    DebugBody::Kinematic => KINEMATIC,
                    DebugBody::Dynamic => DYNAMIC,
                    DebugBody::Sleeping => SLEEPING,
                }
            };
            out.push(DebugShape::oriented(
                mesh(s.kind),
                g(s.position),
                Quat::from_xyzw(x, y, z, w).normalize(),
                g(s.size),
                colour,
            ));
        }
        for c in self.draw.contacts.iter().take(MAX_CONTACTS) {
            let p = g(c.point);
            out.push(DebugShape::oriented(
                MeshKind::Box,
                p,
                Quat::IDENTITY,
                GVec3::splat(0.25),
                CONTACT,
            ));
            out.extend(DebugShape::segment(p, p + g(c.normal) * 1.5, 0.05, CONTACT));
        }
        for v in &self.draw.velocities {
            let from = g(v.from);
            let to = from + g(v.velocity * VELOCITY_SECONDS);
            out.extend(DebugShape::segment(from, to, 0.08, VELOCITY));
        }
        for t in &self.draw.touch_volumes {
            let colour = if t.touching { TOUCHING } else { TOUCH };
            DebugShape::capsule(g(t.a), g(t.b), t.radius as f32, colour, out);
        }
    }
}
