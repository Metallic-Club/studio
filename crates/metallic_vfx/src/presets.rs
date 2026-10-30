//! The built-in effect library.
//!
//! Each preset is authored once, for its native style, with the small builder below; the editor (and
//! [`crate::Effect::restyle`]) converts it to any other style. The same presets are exported as
//! `.vfx.ron` files under `crates/metallic_vfx/presets/` (a test keeps them in sync), so they double as
//! format examples.

use crate::VfxError;
use crate::color::Rgb;
use crate::curve::{Curve, Gradient, Range};
use crate::effect::{
    ArtStyle, Beam, Direction, Effect, Emitter, Facing, Flipbook, FlipbookMode, HitAction, InOut,
    InitModule, PreviewMotion, Render, RenderKind, Shape, SimSpace, SortMode, SpawnModule, Texture,
    TextureMode, Trail, UpdateModule,
};
use crate::math::Vec3;

/// Builder for one emitter (keeps preset code short).
struct B(Emitter);

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn em(name: &str, capacity: u32, texture: Texture) -> B {
    B(Emitter {
        name: name.into(),
        enabled: true,
        capacity,
        space: SimSpace::World,
        time_scale: 1.0,
        delay: 0.0,
        fixed_fps: 0.0,
        spawn: Vec::new(),
        init: Vec::new(),
        update: Vec::new(),
        render: Render {
            texture,
            ..Render::default()
        },
    })
}

impl B {
    fn rate(mut self, per_second: f32) -> Self {
        self.0.spawn.push(SpawnModule::Rate {
            per_second,
            disabled: false,
        });
        self
    }
    fn burst(mut self, time: f32, count: u32) -> Self {
        self.0.spawn.push(SpawnModule::Burst {
            time,
            count,
            cycles: 1,
            interval: 0.0,
            disabled: false,
        });
        self
    }
    fn distance(mut self, per_stud: f32) -> Self {
        self.0.spawn.push(SpawnModule::Distance {
            per_stud,
            disabled: false,
        });
        self
    }
    fn shape(self, shape: Shape) -> Self {
        self.shape_full(shape, false, Vec3::ZERO, Vec3::ZERO)
    }
    fn shape_at(self, shape: Shape, offset: Vec3) -> Self {
        self.shape_full(shape, false, Vec3::ZERO, offset)
    }
    fn shape_full(mut self, shape: Shape, surface: bool, rotation: Vec3, offset: Vec3) -> Self {
        self.0.init.push(InitModule::Shape {
            shape,
            surface,
            rotation,
            offset,
            disabled: false,
        });
        self
    }
    fn life(mut self, a: f32, b: f32) -> Self {
        self.0.init.push(InitModule::Lifetime {
            seconds: Range::new(a, b),
            disabled: false,
        });
        self
    }
    fn vel(mut self, dir: Vec3, a: f32, b: f32, spread: f32) -> Self {
        self.0.init.push(InitModule::Velocity {
            speed: Range::new(a, b),
            direction: Direction::Axis(dir),
            spread: (spread, spread),
            disabled: false,
        });
        self
    }
    fn vel_out(mut self, a: f32, b: f32, spread: f32) -> Self {
        self.0.init.push(InitModule::Velocity {
            speed: Range::new(a, b),
            direction: Direction::FromShape(InOut::Outward),
            spread: (spread, spread),
            disabled: false,
        });
        self
    }
    fn size(mut self, a: f32, b: f32) -> Self {
        self.0.init.push(InitModule::Size {
            size: Range::new(a, b),
            disabled: false,
        });
        self
    }
    fn colors(mut self, hex: &[u32], mix: bool) -> Self {
        self.0.init.push(InitModule::Color {
            choices: hex.iter().map(|&h| Rgb::hex(h)).collect(),
            mix,
            disabled: false,
        });
        self
    }
    fn rot(mut self, a: f32, b: f32, s0: f32, s1: f32) -> Self {
        self.0.init.push(InitModule::Rotation {
            angle: Range::new(a, b),
            spin: Range::new(s0, s1),
            disabled: false,
        });
        self
    }
    fn inherit(mut self, amount: f32) -> Self {
        self.0.init.push(InitModule::InheritVelocity {
            amount,
            disabled: false,
        });
        self
    }
    fn accel(mut self, x: f32, y: f32, z: f32) -> Self {
        self.0.update.push(UpdateModule::Acceleration {
            accel: v(x, y, z),
            disabled: false,
        });
        self
    }
    fn drag(mut self, drag: f32) -> Self {
        self.0.update.push(UpdateModule::Drag {
            drag,
            disabled: false,
        });
        self
    }
    fn curl(mut self, strength: f32, scale: f32, scroll: Vec3) -> Self {
        self.0.update.push(UpdateModule::CurlNoise {
            strength,
            scale,
            scroll,
            octaves: 2,
            disabled: false,
        });
        self
    }
    fn vortex(mut self, axis: Vec3, swirl: f32, pull: f32) -> Self {
        self.0.update.push(UpdateModule::Vortex {
            axis,
            center: Vec3::ZERO,
            swirl,
            pull,
            disabled: false,
        });
        self
    }
    fn attract(mut self, position: Vec3, strength: f32, radius: f32, kill: f32) -> Self {
        self.0.update.push(UpdateModule::Attractor {
            position,
            strength,
            radius,
            kill_radius: kill,
            disabled: false,
        });
        self
    }
    fn ground(mut self, bounce: f32, friction: f32, on_hit: HitAction) -> Self {
        self.0.update.push(UpdateModule::CollisionPlane {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            bounce,
            friction,
            on_hit,
            disabled: false,
        });
        self
    }
    fn size_ol(mut self, points: &[(f32, f32)]) -> Self {
        self.0.update.push(UpdateModule::SizeOverLife {
            curve: Curve::smooth(points),
            disabled: false,
        });
        self
    }
    fn color_ol(mut self, points: &[(f32, u32)]) -> Self {
        self.0.update.push(UpdateModule::ColorOverLife {
            gradient: Gradient::hex(points),
            disabled: false,
        });
        self
    }
    fn color_steps(mut self, points: &[(f32, u32)]) -> Self {
        self.0.update.push(UpdateModule::ColorOverLife {
            gradient: Gradient::hex(points).stepped(),
            disabled: false,
        });
        self
    }
    fn fade(mut self, points: &[(f32, f32)]) -> Self {
        self.0.update.push(UpdateModule::TransparencyOverLife {
            curve: Curve::smooth(points),
            disabled: false,
        });
        self
    }
    fn squash(mut self, points: &[(f32, f32)]) -> Self {
        self.0.update.push(UpdateModule::SquashOverLife {
            curve: Curve::smooth(points),
            disabled: false,
        });
        self
    }
    fn erode(mut self, points: &[(f32, f32)]) -> Self {
        self.0.update.push(UpdateModule::ErosionOverLife {
            curve: Curve::smooth(points),
            disabled: false,
        });
        self
    }
    fn glow(mut self, light_emission: f32, brightness: f32) -> Self {
        self.0.render.light_emission = light_emission;
        self.0.render.brightness = brightness;
        self
    }
    fn facing(mut self, f: Facing) -> Self {
        self.0.render.facing = f;
        self
    }
    fn stretched(mut self, speed_scale: f32) -> Self {
        self.0.render.kind = RenderKind::Stretched {
            speed_scale,
            min_length: 1.0,
        };
        self
    }
    fn ribbon(mut self) -> Self {
        self.0.render.kind = RenderKind::Ribbon {
            texture_mode: TextureMode::Stretch,
            texture_length: 1.0,
        };
        self
    }
    fn flipbook(mut self, mode: FlipbookMode, fps: (f32, f32), start_random: bool) -> Self {
        self.0.render.flipbook = Some(Flipbook {
            columns: 4,
            rows: 4,
            mode,
            fps: Range::new(fps.0, fps.1),
            start_random,
            blend: true,
        });
        self
    }
    fn sort(mut self, s: SortMode) -> Self {
        self.0.render.sort = s;
        self
    }
    fn pixel(mut self) -> Self {
        self.0.render.alpha_steps = 2;
        self.0.render.pixel_snap = 3.0;
        self.0.fixed_fps = 12.0;
        self
    }
    fn toon(mut self) -> Self {
        self.0.render.alpha_steps = 1;
        self
    }
    fn done(self) -> Emitter {
        self.0
    }
}

fn effect(name: &str, description: &str, tags: &[&str], emitters: Vec<Emitter>) -> Effect {
    Effect {
        name: name.into(),
        description: description.into(),
        tags: tags.iter().map(|t| (*t).to_string()).collect(),
        emitters,
        ..Effect::default()
    }
}

fn one_shot(mut e: Effect, duration: f32) -> Effect {
    e.duration = duration;
    e.looping = true;
    e
}

fn fire() -> Effect {
    effect(
        "Fire",
        "Campfire flames with embers and a smoke column.",
        &["fire", "loop", "environment"],
        vec![
            em("Smoke", 128, Texture::Smoke)
                .rate(5.0)
                .shape_at(
                    Shape::Sphere {
                        radius: 0.4,
                        partial: 1.0,
                    },
                    v(0.0, 1.8, 0.0),
                )
                .life(2.2, 3.2)
                .vel(Vec3::Y, 1.0, 1.6, 20.0)
                .size(1.0, 1.5)
                .rot(0.0, 360.0, -25.0, 25.0)
                .curl(1.5, 3.0, v(0.0, 0.6, 0.0))
                .size_ol(&[(0.0, 0.6), (1.0, 2.4)])
                .color_ol(&[(0.0, 0x3A3330), (1.0, 0x5E5853)])
                .fade(&[(0.0, 1.0), (0.2, 0.72), (1.0, 1.0)])
                .done(),
            em("Flames", 128, Texture::FlameSheet)
                .rate(42.0)
                .shape(Shape::Cone {
                    radius: 0.55,
                    angle: 12.0,
                })
                .life(0.55, 0.95)
                .vel_out(1.8, 3.2, 0.0)
                .size(1.3, 1.9)
                .rot(-12.0, 12.0, -20.0, 20.0)
                .accel(0.0, 1.6, 0.0)
                .curl(3.0, 2.0, v(0.0, 1.5, 0.0))
                .size_ol(&[(0.0, 0.55), (0.25, 1.0), (1.0, 0.25)])
                .color_ol(&[
                    (0.0, 0xFFF3C4),
                    (0.2, 0xFFB238),
                    (0.55, 0xF25C19),
                    (1.0, 0x7A1E0C),
                ])
                .fade(&[(0.0, 1.0), (0.12, 0.05), (0.65, 0.3), (1.0, 1.0)])
                .flipbook(FlipbookMode::Loop, (14.0, 20.0), true)
                .glow(0.85, 1.6)
                .done(),
            em("Embers", 64, Texture::SoftDot)
                .rate(9.0)
                .shape_at(
                    Shape::Sphere {
                        radius: 0.45,
                        partial: 1.0,
                    },
                    v(0.0, 0.3, 0.0),
                )
                .life(1.0, 2.0)
                .vel(Vec3::Y, 2.0, 4.0, 40.0)
                .size(0.08, 0.15)
                .drag(0.5)
                .curl(6.0, 1.5, v(0.0, 1.0, 0.0))
                .color_ol(&[(0.0, 0xFFD27A), (1.0, 0xFF5A1F)])
                .fade(&[(0.0, 0.0), (0.7, 0.2), (1.0, 1.0)])
                .glow(1.0, 2.2)
                .done(),
        ],
    )
}

fn smoke() -> Effect {
    effect(
        "Smoke",
        "A tall, drifting smoke plume.",
        &["smoke", "loop", "environment"],
        vec![
            em("Plume", 256, Texture::Smoke)
                .rate(9.0)
                .shape(Shape::Disc {
                    radius: 0.7,
                    inner: 0.0,
                })
                .life(3.5, 5.0)
                .vel(Vec3::Y, 1.2, 1.9, 14.0)
                .size(1.4, 2.2)
                .rot(0.0, 360.0, -18.0, 18.0)
                .accel(0.45, 0.25, 0.0)
                .drag(0.25)
                .curl(1.8, 4.0, v(0.0, 0.5, 0.0))
                .size_ol(&[(0.0, 0.5), (1.0, 2.6)])
                .color_ol(&[(0.0, 0x77716B), (0.5, 0x9E9891), (1.0, 0xC8C3BC)])
                .fade(&[(0.0, 1.0), (0.12, 0.45), (0.6, 0.65), (1.0, 1.0)])
                .done(),
        ],
    )
}

fn sparks() -> Effect {
    one_shot(
        effect(
            "Sparks",
            "Grinder sparks that bounce on the ground.",
            &["sparks", "impact", "metal"],
            vec![
                em("Flash", 4, Texture::SoftDot)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(0.0, 1.2, 0.0))
                    .life(0.14, 0.14)
                    .size(2.6, 2.6)
                    .size_ol(&[(0.0, 0.5), (1.0, 1.3)])
                    .color_ol(&[(0.0, 0xFFE2A0), (1.0, 0xFF9A3C)])
                    .fade(&[(0.0, 0.0), (1.0, 1.0)])
                    .glow(1.0, 1.6)
                    .done(),
                em("Sparks", 128, Texture::Spark)
                    .burst(0.0, 48)
                    .shape_at(Shape::Point, v(0.0, 1.2, 0.0))
                    .life(0.5, 1.1)
                    .vel(v(0.4, 1.0, 0.0), 6.0, 12.0, 120.0)
                    .size(0.09, 0.15)
                    .accel(0.0, -16.0, 0.0)
                    .drag(0.8)
                    .ground(0.35, 0.25, HitAction::Bounce)
                    .color_ol(&[(0.0, 0xFFF4C2), (0.35, 0xFFB347), (1.0, 0xD9480F)])
                    .fade(&[(0.0, 0.0), (0.75, 0.0), (1.0, 1.0)])
                    .stretched(0.055)
                    .glow(1.0, 2.2)
                    .done(),
            ],
        ),
        1.3,
    )
}

fn magic() -> Effect {
    effect(
        "Magic",
        "Violet motes spiralling around a glowing rune ring.",
        &["magic", "loop", "aura"],
        vec![
            em("Rune ring", 8, Texture::Ring)
                .rate(1.6)
                .shape_at(Shape::Point, v(0.0, 0.05, 0.0))
                .life(1.6, 1.6)
                .size(3.4, 3.4)
                .size_ol(&[(0.0, 0.7), (1.0, 1.05)])
                .color_ol(&[(0.0, 0xB59BFF), (1.0, 0x6B4FD8)])
                .fade(&[(0.0, 1.0), (0.3, 0.35), (1.0, 1.0)])
                .facing(Facing::Ground)
                .glow(1.0, 1.4)
                .done(),
            em("Motes", 256, Texture::Twinkle)
                .rate(46.0)
                .shape_full(
                    Shape::Cylinder {
                        radius: 1.3,
                        height: 0.2,
                        top: 1.0,
                    },
                    true,
                    Vec3::ZERO,
                    v(0.0, 0.2, 0.0),
                )
                .life(1.3, 2.1)
                .vel(Vec3::Y, 0.8, 1.8, 10.0)
                .size(0.12, 0.3)
                .colors(&[0xC9B3FF, 0x8B6CF0, 0xE8DDFF], true)
                .rot(0.0, 360.0, -90.0, 90.0)
                .vortex(Vec3::Y, 5.0, 1.6)
                .curl(1.5, 2.0, Vec3::ZERO)
                .size_ol(&[(0.0, 0.0), (0.2, 1.0), (1.0, 0.0)])
                .fade(&[(0.0, 0.0), (0.8, 0.1), (1.0, 1.0)])
                .glow(1.0, 1.8)
                .done(),
            em("Core glow", 16, Texture::SoftDot)
                .rate(5.0)
                .shape_at(Shape::Point, v(0.0, 0.9, 0.0))
                .life(0.9, 1.2)
                .size(2.2, 2.8)
                .size_ol(&[(0.0, 0.6), (1.0, 1.1)])
                .color_ol(&[(0.0, 0x8B6CF0), (1.0, 0x4B2FA8)])
                .fade(&[(0.0, 1.0), (0.4, 0.75), (1.0, 1.0)])
                .glow(1.0, 1.0)
                .done(),
        ],
    )
}

fn explosion() -> Effect {
    one_shot(
        effect(
            "Explosion",
            "Fireball, debris sparks, smoke and a ground shockwave.",
            &["explosion", "impact", "one-shot"],
            vec![
                em("Flash", 4, Texture::SoftDot)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(0.0, 1.2, 0.0))
                    .life(0.16, 0.16)
                    .size(7.0, 7.0)
                    .color_ol(&[(0.0, 0xFFF1C9), (1.0, 0xFF8C3A)])
                    .fade(&[(0.0, 0.0), (1.0, 1.0)])
                    .glow(1.0, 1.6)
                    .done(),
                em("Smoke", 64, Texture::Smoke)
                    .burst(0.12, 22)
                    .shape_at(
                        Shape::Sphere {
                            radius: 1.0,
                            partial: 1.0,
                        },
                        v(0.0, 1.3, 0.0),
                    )
                    .life(2.0, 3.0)
                    .vel_out(1.0, 3.0, 0.0)
                    .size(2.0, 3.0)
                    .rot(0.0, 360.0, -30.0, 30.0)
                    .accel(0.0, 0.9, 0.0)
                    .drag(1.5)
                    .curl(1.2, 3.0, Vec3::ZERO)
                    .size_ol(&[(0.0, 0.6), (1.0, 1.8)])
                    .color_ol(&[(0.0, 0x2E2926), (1.0, 0x6B6560)])
                    .fade(&[(0.0, 1.0), (0.1, 0.4), (1.0, 1.0)])
                    .done(),
                em("Fireball", 64, Texture::SmokeSheet)
                    .burst(0.0, 28)
                    .shape_at(
                        Shape::Sphere {
                            radius: 0.6,
                            partial: 1.0,
                        },
                        v(0.0, 1.2, 0.0),
                    )
                    .life(0.8, 1.3)
                    .vel_out(3.0, 7.0, 0.0)
                    .size(1.6, 2.6)
                    .rot(0.0, 360.0, -60.0, 60.0)
                    .accel(0.0, 2.0, 0.0)
                    .drag(3.0)
                    .size_ol(&[(0.0, 0.4), (0.15, 1.0), (1.0, 1.4)])
                    .color_ol(&[
                        (0.0, 0xFFF1C1),
                        (0.15, 0xFFB341),
                        (0.4, 0xE2531A),
                        (0.7, 0x4A2A22),
                        (1.0, 0x2A2523),
                    ])
                    .fade(&[(0.0, 0.0), (0.6, 0.1), (1.0, 1.0)])
                    .flipbook(FlipbookMode::OneShot, (12.0, 12.0), false)
                    .glow(0.6, 1.5)
                    .done(),
                em("Debris", 96, Texture::Spark)
                    .burst(0.0, 40)
                    .shape_at(
                        Shape::Sphere {
                            radius: 0.4,
                            partial: 1.0,
                        },
                        v(0.0, 1.2, 0.0),
                    )
                    .life(0.6, 1.2)
                    .vel_out(10.0, 18.0, 0.0)
                    .size(0.12, 0.2)
                    .accel(0.0, -20.0, 0.0)
                    .drag(0.8)
                    .ground(0.3, 0.3, HitAction::Bounce)
                    .color_ol(&[(0.0, 0xFFF0B0), (1.0, 0xE8572A)])
                    .fade(&[(0.0, 0.0), (0.8, 0.1), (1.0, 1.0)])
                    .stretched(0.04)
                    .glow(1.0, 2.0)
                    .done(),
                em("Shockwave", 4, Texture::Ring)
                    .burst(0.02, 1)
                    .shape_at(Shape::Point, v(0.0, 0.05, 0.0))
                    .life(0.5, 0.5)
                    .size(10.0, 10.0)
                    .size_ol(&[(0.0, 0.08), (0.4, 0.75), (1.0, 1.0)])
                    .color_ol(&[(0.0, 0xFFE2B0), (1.0, 0xFFB070)])
                    .fade(&[(0.0, 0.1), (1.0, 1.0)])
                    .facing(Facing::Ground)
                    .glow(1.0, 1.2)
                    .done(),
            ],
        ),
        3.2,
    )
}

fn dust_puff() -> Effect {
    one_shot(
        effect(
            "Dust puff",
            "A soft puff of dust kicked up from the ground (landing, dash).",
            &["dust", "impact", "movement"],
            vec![
                em("Puff", 32, Texture::Smoke)
                    .burst(0.0, 14)
                    .shape(Shape::Cone {
                        radius: 0.3,
                        angle: 78.0,
                    })
                    .life(0.8, 1.3)
                    .vel_out(2.0, 4.2, 0.0)
                    .size(0.5, 0.9)
                    .rot(0.0, 360.0, -60.0, 60.0)
                    .accel(0.0, 0.4, 0.0)
                    .drag(2.5)
                    .size_ol(&[(0.0, 0.5), (1.0, 2.2)])
                    .colors(&[0xC8B59A, 0xB5A285, 0xD6C7AE], true)
                    .fade(&[(0.0, 1.0), (0.08, 0.3), (1.0, 1.0)])
                    .done(),
            ],
        ),
        1.6,
    )
}

fn footstep() -> Effect {
    one_shot(
        effect(
            "Footstep",
            "Small dust and pebbles for each step; also spawns by distance while walking.",
            &["dust", "movement", "footstep"],
            vec![
                em("Dust", 32, Texture::Smoke)
                    .burst(0.0, 7)
                    .distance(0.6)
                    .shape(Shape::Cone {
                        radius: 0.15,
                        angle: 82.0,
                    })
                    .life(0.5, 0.8)
                    .vel_out(1.2, 2.4, 0.0)
                    .size(0.28, 0.45)
                    .rot(0.0, 360.0, -40.0, 40.0)
                    .drag(3.0)
                    .size_ol(&[(0.0, 0.6), (1.0, 1.9)])
                    .colors(&[0xD2C3A6, 0xBFAE8E], true)
                    .fade(&[(0.0, 1.0), (0.1, 0.35), (1.0, 1.0)])
                    .done(),
                em("Pebbles", 16, Texture::Dot)
                    .burst(0.0, 5)
                    .shape(Shape::Point)
                    .life(0.5, 0.7)
                    .vel(Vec3::Y, 2.0, 3.2, 70.0)
                    .size(0.06, 0.1)
                    .colors(&[0x8C7B62, 0x6E604B], false)
                    .accel(0.0, -18.0, 0.0)
                    .ground(0.3, 0.5, HitAction::Bounce)
                    .done(),
            ],
        ),
        0.9,
    )
}

fn splash() -> Effect {
    one_shot(
        effect(
            "Splash",
            "Water splash with droplets, mist and ripples.",
            &["water", "impact"],
            vec![
                em("Ripples", 8, Texture::Ring)
                    .burst(0.0, 1)
                    .burst(0.22, 1)
                    .shape_at(Shape::Point, v(0.0, 0.03, 0.0))
                    .life(0.9, 0.9)
                    .size(4.0, 4.0)
                    .size_ol(&[(0.0, 0.1), (1.0, 1.0)])
                    .colors(&[0xD6F1FF], false)
                    .fade(&[(0.0, 0.2), (1.0, 1.0)])
                    .facing(Facing::Ground)
                    .glow(0.4, 1.0)
                    .done(),
                em("Mist", 16, Texture::SoftDot)
                    .burst(0.02, 10)
                    .shape(Shape::Disc {
                        radius: 0.4,
                        inner: 0.0,
                    })
                    .life(0.6, 1.0)
                    .vel(Vec3::Y, 1.0, 2.5, 60.0)
                    .size(0.6, 1.0)
                    .drag(2.0)
                    .size_ol(&[(0.0, 0.6), (1.0, 1.8)])
                    .colors(&[0xEAF7FF], false)
                    .fade(&[(0.0, 0.55), (1.0, 1.0)])
                    .done(),
                em("Droplets", 96, Texture::Droplet)
                    .burst(0.0, 50)
                    .shape(Shape::Cone {
                        radius: 0.35,
                        angle: 28.0,
                    })
                    .life(0.7, 1.1)
                    .vel_out(5.0, 8.5, 0.0)
                    .size(0.12, 0.2)
                    .colors(&[0xDDF3FF, 0xA8DBF5], true)
                    .accel(0.0, -18.0, 0.0)
                    .ground(0.0, 0.0, HitAction::Kill)
                    .fade(&[(0.0, 0.0), (1.0, 0.3)])
                    .facing(Facing::VelocityParallel)
                    .glow(0.25, 1.1)
                    .done(),
            ],
        ),
        1.4,
    )
}

fn rain() -> Effect {
    effect(
        "Rain",
        "Steady rain with splashes on the ground.",
        &["weather", "rain", "loop"],
        vec![
            em("Drops", 1024, Texture::Streak)
                .rate(320.0)
                .shape_at(
                    Shape::Box {
                        size: v(14.0, 0.1, 8.0),
                    },
                    v(0.0, 9.0, 0.0),
                )
                .life(0.7, 0.7)
                .vel(v(0.12, -1.0, 0.0), 17.0, 21.0, 2.0)
                .size(0.05, 0.07)
                .colors(&[0xB7CDE0], false)
                .ground(0.0, 0.0, HitAction::Kill)
                .fade(&[(0.0, 0.45), (1.0, 0.45)])
                .stretched(0.045)
                .done(),
            em("Splashes", 256, Texture::Ring)
                .rate(70.0)
                .shape_at(
                    Shape::Box {
                        size: v(14.0, 0.0, 8.0),
                    },
                    v(0.0, 0.03, 0.0),
                )
                .life(0.28, 0.28)
                .size(0.55, 0.55)
                .size_ol(&[(0.0, 0.15), (1.0, 1.0)])
                .colors(&[0xC6D9EA], false)
                .fade(&[(0.0, 0.3), (1.0, 1.0)])
                .facing(Facing::Ground)
                .done(),
        ],
    )
}

fn snow() -> Effect {
    effect(
        "Snow",
        "Gentle snowfall that settles on the ground.",
        &["weather", "snow", "loop"],
        vec![
            em("Flakes", 1024, Texture::SoftDot)
                .rate(70.0)
                .shape_at(
                    Shape::Box {
                        size: v(14.0, 0.1, 8.0),
                    },
                    v(0.0, 8.0, 0.0),
                )
                .life(8.0, 10.0)
                .vel(-Vec3::Y, 0.8, 1.4, 10.0)
                .size(0.07, 0.17)
                .curl(1.2, 3.0, v(0.3, 0.0, 0.0))
                .ground(0.0, 1.0, HitAction::Stick)
                .fade(&[(0.0, 1.0), (0.08, 0.0), (0.9, 0.0), (1.0, 1.0)])
                .done(),
        ],
    )
}

fn leaves() -> Effect {
    effect(
        "Falling leaves",
        "Autumn leaves tumbling in the wind.",
        &["nature", "leaves", "loop"],
        vec![
            em("Leaves", 128, Texture::Leaf)
                .rate(6.0)
                .shape_at(
                    Shape::Box {
                        size: v(10.0, 0.1, 6.0),
                    },
                    v(-1.5, 7.0, 0.0),
                )
                .life(7.0, 9.0)
                .vel(-Vec3::Y, 0.4, 0.8, 30.0)
                .size(0.35, 0.5)
                .colors(&[0xD9822B, 0xC0562A, 0xE8B04A, 0x9C6B2E], false)
                .rot(0.0, 360.0, -200.0, 200.0)
                .accel(0.7, -0.7, 0.0)
                .drag(0.6)
                .curl(2.5, 2.5, v(0.4, 0.0, 0.0))
                .ground(0.0, 1.0, HitAction::Stick)
                .squash(&[
                    (0.0, 0.0),
                    (0.25, 0.9),
                    (0.5, 0.0),
                    (0.75, -0.9),
                    (1.0, 0.0),
                ])
                .fade(&[(0.0, 1.0), (0.05, 0.0), (0.92, 0.0), (1.0, 1.0)])
                .done(),
        ],
    )
}

fn confetti() -> Effect {
    one_shot(
        effect(
            "Confetti",
            "A celebratory confetti cannon.",
            &["celebration", "ui", "one-shot"],
            vec![
                em("Confetti", 256, Texture::Confetti)
                    .burst(0.0, 140)
                    .shape(Shape::Cone {
                        radius: 0.2,
                        angle: 26.0,
                    })
                    .life(3.0, 4.0)
                    .vel_out(10.0, 15.0, 0.0)
                    .size(0.18, 0.26)
                    .colors(
                        &[0xFF4D6D, 0xFFD23F, 0x3EC1D3, 0x7C5CFF, 0x6BE07B, 0xFF9F1C],
                        false,
                    )
                    .rot(0.0, 360.0, -400.0, 400.0)
                    .accel(0.0, -9.0, 0.0)
                    .drag(1.6)
                    .curl(3.0, 2.0, Vec3::ZERO)
                    .squash(&[
                        (0.0, 0.0),
                        (0.2, 1.0),
                        (0.4, -0.6),
                        (0.6, 0.9),
                        (0.8, -0.8),
                        (1.0, 0.0),
                    ])
                    .fade(&[(0.0, 0.0), (0.85, 0.0), (1.0, 1.0)])
                    .done(),
            ],
        ),
        4.2,
    )
}

fn hit_impact() -> Effect {
    one_shot(
        effect(
            "Hit impact",
            "A punchy melee hit: flash, star, sparks and a ring.",
            &["impact", "combat", "one-shot"],
            vec![
                em("Flash", 2, Texture::SoftDot)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(0.0, 1.5, 0.0))
                    .life(0.12, 0.12)
                    .size(3.0, 3.0)
                    .colors(&[0xFFF4D6], false)
                    .fade(&[(0.0, 0.0), (1.0, 1.0)])
                    .glow(1.0, 1.5)
                    .done(),
                em("Star", 2, Texture::Twinkle)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(0.0, 1.5, 0.0))
                    .life(0.2, 0.2)
                    .size(2.6, 2.6)
                    .rot(0.0, 90.0, 0.0, 0.0)
                    .size_ol(&[(0.0, 1.0), (1.0, 0.2)])
                    .colors(&[0xFFFFFF], false)
                    .glow(1.0, 1.8)
                    .done(),
                em("Ring", 2, Texture::Ring)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(0.0, 1.5, 0.0))
                    .life(0.24, 0.24)
                    .size(3.0, 3.0)
                    .size_ol(&[(0.0, 0.15), (1.0, 1.0)])
                    .colors(&[0xFFD58A], false)
                    .fade(&[(0.0, 0.0), (1.0, 1.0)])
                    .glow(1.0, 1.4)
                    .done(),
                em("Sparks", 32, Texture::Spark)
                    .burst(0.0, 16)
                    .shape_at(Shape::Point, v(0.0, 1.5, 0.0))
                    .life(0.2, 0.35)
                    .vel_out(9.0, 15.0, 0.0)
                    .size(0.12, 0.16)
                    .drag(6.0)
                    .color_ol(&[(0.0, 0xFFF3C4), (1.0, 0xFF8A2B)])
                    .fade(&[(0.0, 0.0), (0.6, 0.0), (1.0, 1.0)])
                    .stretched(0.05)
                    .glow(1.0, 2.0)
                    .done(),
            ],
        ),
        0.7,
    )
}

fn muzzle_flash() -> Effect {
    one_shot(
        effect(
            "Muzzle flash",
            "Rapid-fire muzzle flash pointing along +X, with a smoke wisp.",
            &["weapon", "combat", "fps"],
            vec![
                em("Wisp", 32, Texture::Smoke)
                    .burst(0.02, 3)
                    .shape_at(Shape::Point, v(-1.5, 1.5, 0.0))
                    .life(0.6, 0.9)
                    .vel(Vec3::X, 1.0, 2.0, 30.0)
                    .size(0.3, 0.45)
                    .rot(0.0, 360.0, -40.0, 40.0)
                    .drag(2.0)
                    .accel(0.0, 0.8, 0.0)
                    .curl(1.5, 1.0, Vec3::ZERO)
                    .size_ol(&[(0.0, 0.6), (1.0, 2.2)])
                    .colors(&[0x8F8A84], false)
                    .fade(&[(0.0, 0.55), (1.0, 1.0)])
                    .done(),
                em("Core", 4, Texture::SoftDot)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(-1.5, 1.5, 0.0))
                    .life(0.06, 0.06)
                    .size(1.4, 1.4)
                    .colors(&[0xFFE9B0], false)
                    .fade(&[(0.0, 0.0), (1.0, 1.0)])
                    .glow(1.0, 2.0)
                    .done(),
                em("Flame", 16, Texture::Spark)
                    .burst(0.0, 6)
                    .shape_at(Shape::Point, v(-1.5, 1.5, 0.0))
                    .life(0.06, 0.09)
                    .vel(Vec3::X, 7.0, 12.0, 26.0)
                    .size(0.35, 0.5)
                    .color_ol(&[(0.0, 0xFFF0B8), (1.0, 0xFF8A2B)])
                    .fade(&[(0.0, 0.0), (1.0, 1.0)])
                    .stretched(0.11)
                    .glow(1.0, 2.4)
                    .done(),
            ],
        ),
        0.28,
    )
}

fn heal() -> Effect {
    effect(
        "Heal",
        "Green healing aura rising around a character.",
        &["magic", "buff", "loop"],
        vec![
            em("Ground ring", 8, Texture::Ring)
                .rate(1.2)
                .shape_at(Shape::Point, v(0.0, 0.04, 0.0))
                .life(1.7, 1.7)
                .size(2.8, 2.8)
                .size_ol(&[(0.0, 0.85), (1.0, 1.0)])
                .colors(&[0x5BD67E], false)
                .fade(&[(0.0, 1.0), (0.3, 0.35), (1.0, 1.0)])
                .facing(Facing::Ground)
                .glow(1.0, 1.3)
                .done(),
            em("Glow", 64, Texture::SoftDot)
                .rate(12.0)
                .shape(Shape::Disc {
                    radius: 1.2,
                    inner: 0.7,
                })
                .life(1.2, 1.8)
                .vel(Vec3::Y, 0.8, 1.6, 0.0)
                .size(0.6, 0.9)
                .colors(&[0x47C56C], false)
                .fade(&[(0.0, 1.0), (0.3, 0.7), (1.0, 1.0)])
                .glow(1.0, 1.0)
                .done(),
            em("Plus", 64, Texture::Twinkle)
                .rate(16.0)
                .shape(Shape::Disc {
                    radius: 1.2,
                    inner: 0.6,
                })
                .life(1.2, 1.8)
                .vel(Vec3::Y, 1.5, 2.5, 0.0)
                .size(0.18, 0.32)
                .colors(&[0x9BF7B2, 0xD6FFE0], true)
                .vortex(Vec3::Y, 1.5, 0.0)
                .size_ol(&[(0.0, 0.0), (0.2, 1.0), (1.0, 0.0)])
                .glow(1.0, 1.7)
                .done(),
        ],
    )
}

fn portal() -> Effect {
    effect(
        "Portal",
        "A swirling violet portal standing upright.",
        &["magic", "portal", "loop"],
        vec![
            em("Inner swirl", 128, Texture::Smoke)
                .rate(36.0)
                .shape_full(
                    Shape::Disc {
                        radius: 1.6,
                        inner: 0.0,
                    },
                    false,
                    v(90.0, 0.0, 0.0),
                    v(0.0, 2.2, 0.0),
                )
                .life(0.9, 1.3)
                .size(0.9, 1.5)
                .rot(0.0, 360.0, 60.0, 140.0)
                .vortex(Vec3::Z, 4.0, 1.0)
                .color_ol(&[(0.0, 0x2B1B55), (1.0, 0x6B4FD8)])
                .fade(&[(0.0, 1.0), (0.3, 0.45), (1.0, 1.0)])
                .glow(0.3, 1.0)
                .done(),
            em("Rim", 512, Texture::SoftDot)
                .rate(140.0)
                .shape_full(
                    Shape::Disc {
                        radius: 2.0,
                        inner: 0.9,
                    },
                    false,
                    v(90.0, 0.0, 0.0),
                    v(0.0, 2.2, 0.0),
                )
                .life(0.8, 1.3)
                .size(0.1, 0.24)
                .colors(&[0x7C5CFF, 0xB497FF, 0x5C3FD1, 0xE3D8FF], true)
                .attract(v(0.0, 2.2, 0.0), 6.0, 2.0, 0.25)
                .vortex(Vec3::Z, 12.0, 0.0)
                .fade(&[(0.0, 0.2), (1.0, 1.0)])
                .glow(1.0, 1.8)
                .done(),
        ],
    )
}

fn shockwave() -> Effect {
    one_shot(
        effect(
            "Shockwave",
            "A ground slam: expanding ring, dust wall and flash.",
            &["impact", "ground", "one-shot"],
            vec![
                em("Dust wall", 64, Texture::Smoke)
                    .burst(0.0, 40)
                    .shape_full(
                        Shape::Cone {
                            radius: 0.6,
                            angle: 86.0,
                        },
                        true,
                        Vec3::ZERO,
                        v(0.0, 0.2, 0.0),
                    )
                    .life(0.9, 1.4)
                    .vel_out(6.0, 9.0, 0.0)
                    .size(0.6, 0.9)
                    .rot(0.0, 360.0, -50.0, 50.0)
                    .drag(3.0)
                    .accel(0.0, 0.6, 0.0)
                    .size_ol(&[(0.0, 0.6), (1.0, 2.0)])
                    .colors(&[0xB9AE9C, 0xA39784], true)
                    .fade(&[(0.0, 1.0), (0.06, 0.25), (1.0, 1.0)])
                    .done(),
                em("Ring", 4, Texture::Ring)
                    .burst(0.0, 1)
                    .burst(0.08, 1)
                    .shape_at(Shape::Point, v(0.0, 0.05, 0.0))
                    .life(0.6, 0.6)
                    .size(10.0, 10.0)
                    .size_ol(&[(0.0, 0.05), (0.35, 0.7), (1.0, 1.0)])
                    .colors(&[0xE8EEFF], false)
                    .fade(&[(0.0, 0.0), (1.0, 1.0)])
                    .facing(Facing::Ground)
                    .glow(0.7, 1.2)
                    .done(),
                em("Flash", 2, Texture::SoftDot)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(0.0, 0.4, 0.0))
                    .life(0.15, 0.15)
                    .size(4.0, 4.0)
                    .colors(&[0xF1F4FF], false)
                    .fade(&[(0.0, 0.2), (1.0, 1.0)])
                    .glow(1.0, 1.2)
                    .done(),
            ],
        ),
        1.6,
    )
}

fn toon_smear() -> Effect {
    let mut e = one_shot(
        effect(
            "Toon smear",
            "Anime sword slash: a hard-edged smear arc with sparkles.",
            &["toon", "combat", "slash"],
            vec![
                em("Smear", 2, Texture::Smear)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(0.0, 1.6, 0.0))
                    .life(0.28, 0.28)
                    .size(4.6, 4.6)
                    .rot(-25.0, -25.0, 0.0, 0.0)
                    .size_ol(&[(0.0, 0.85), (1.0, 1.05)])
                    .color_steps(&[(0.0, 0xFFFFFF), (0.35, 0xA6E9FF), (0.7, 0x3D8BFF)])
                    .erode(&[(0.0, 0.0), (0.45, 0.15), (1.0, 1.0)])
                    .toon()
                    .glow(0.2, 1.0)
                    .done(),
                em("Sparkles", 16, Texture::Twinkle)
                    .burst(0.04, 7)
                    .shape_at(
                        Shape::Sphere {
                            radius: 1.2,
                            partial: 0.5,
                        },
                        v(0.4, 1.6, 0.0),
                    )
                    .life(0.3, 0.5)
                    .vel_out(3.0, 5.0, 0.0)
                    .size(0.35, 0.55)
                    .drag(5.0)
                    .colors(&[0xFFFFFF, 0xBDF0FF], false)
                    .erode(&[(0.0, 0.0), (1.0, 1.0)])
                    .toon()
                    .glow(0.5, 1.2)
                    .done(),
            ],
        ),
        0.7,
    );
    e.style = ArtStyle::Toon;
    e
}

fn pixel_spark() -> Effect {
    let mut e = one_shot(
        effect(
            "Pixel spark",
            "8-bit spark burst, snapped to pixels and stepped at 12 fps.",
            &["pixel", "impact", "retro"],
            vec![
                em("Core", 2, Texture::PixelDot)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(0.0, 1.5, 0.0))
                    .life(0.33, 0.33)
                    .size(1.1, 1.1)
                    .colors(&[0xFFF6A8], false)
                    .erode(&[(0.0, 0.0), (1.0, 1.0)])
                    .pixel()
                    .glow(0.6, 1.0)
                    .done(),
                em("Sparks", 32, Texture::PixelSpark)
                    .burst(0.0, 14)
                    .shape_at(Shape::Point, v(0.0, 1.5, 0.0))
                    .life(0.5, 0.8)
                    .vel(Vec3::Y, 4.0, 8.0, 300.0)
                    .size(0.4, 0.4)
                    .accel(0.0, -12.0, 0.0)
                    .drag(1.0)
                    .color_steps(&[(0.0, 0xFFF6A8), (0.4, 0xFFB22E), (0.75, 0xE8572A)])
                    .pixel()
                    .done(),
            ],
        ),
        1.0,
    );
    e.style = ArtStyle::Pixel;
    e
}

fn lava_bubbles() -> Effect {
    effect(
        "Lava bubbles",
        "Bubbles swelling and popping on a lava pool, with splatter.",
        &["lava", "environment", "loop"],
        vec![
            em("Heat glow", 8, Texture::SoftDot)
                .rate(2.0)
                .shape_at(Shape::Point, v(0.0, 0.05, 0.0))
                .life(2.0, 2.0)
                .size(6.0, 6.0)
                .colors(&[0xFF5A1F], false)
                .fade(&[(0.0, 1.0), (0.5, 0.75), (1.0, 1.0)])
                .facing(Facing::Ground)
                .glow(1.0, 1.0)
                .done(),
            em("Bubbles", 32, Texture::Bubble)
                .rate(3.5)
                .shape(Shape::Disc {
                    radius: 2.0,
                    inner: 0.0,
                })
                .life(1.2, 1.8)
                .vel(Vec3::Y, 0.15, 0.3, 0.0)
                .size(0.35, 0.7)
                .size_ol(&[(0.0, 0.2), (0.82, 1.0), (0.9, 1.25), (1.0, 0.0)])
                .colors(&[0xFF7A1A, 0xFFB03A], true)
                .glow(0.5, 1.5)
                .sort(SortMode::OldestOnTop)
                .done(),
            em("Splatter", 64, Texture::Dot)
                .rate(7.0)
                .shape(Shape::Disc {
                    radius: 1.8,
                    inner: 0.0,
                })
                .life(0.5, 0.9)
                .vel(Vec3::Y, 2.0, 4.0, 50.0)
                .size(0.08, 0.15)
                .accel(0.0, -14.0, 0.0)
                .ground(0.0, 0.0, HitAction::Kill)
                .color_ol(&[(0.0, 0xFFC24A), (1.0, 0xB3260E)])
                .glow(0.5, 1.4)
                .done(),
        ],
    )
}

fn lightning() -> Effect {
    let bolt = |name: &str, width: f32, jitter: f32, color: u32| Beam {
        name: name.into(),
        attachment0: v(-3.5, 4.5, 0.0),
        attachment1: v(3.0, 0.2, 0.0),
        direction0: v(1.0, -1.0, 0.0),
        direction1: v(1.0, -1.0, 0.0),
        curve_size0: 1.0,
        curve_size1: 1.0,
        segments: 28,
        width0: width,
        width1: width * 0.6,
        color: Gradient::hex(&[(0.0, 0xE3EBFF), (1.0, color)]),
        transparency: Curve::constant(0.0),
        texture: Texture::Electric,
        texture_speed: 3.0,
        light_emission: 1.0,
        brightness: 2.0,
        jitter,
        jitter_fps: 18.0,
        ..Beam::default()
    };
    let mut e = effect(
        "Lightning",
        "A crackling lightning beam with impact sparks.",
        &["beam", "electric", "loop"],
        vec![
            em("Impact sparks", 64, Texture::Spark)
                .rate(30.0)
                .shape_at(Shape::Point, v(3.0, 0.2, 0.0))
                .life(0.2, 0.4)
                .vel(Vec3::Y, 4.0, 8.0, 160.0)
                .size(0.08, 0.12)
                .accel(0.0, -14.0, 0.0)
                .colors(&[0xD6E2FF, 0x9DB6FF], true)
                .fade(&[(0.0, 0.0), (1.0, 1.0)])
                .stretched(0.04)
                .glow(1.0, 2.0)
                .done(),
        ],
    );
    e.beams = vec![
        bolt("Glow", 1.6, 0.5, 0x6E7BFF),
        bolt("Bolt", 0.45, 0.5, 0xA9B8FF),
    ];
    // Same seed for both bolts would overlap exactly; offset the glow's jitter by curve sizes.
    if let Some(b) = e.beams.first_mut() {
        b.curve_size0 = 1.2;
        b.texture = Texture::Band;
        b.light_emission = 1.0;
        b.brightness = 0.9;
    }
    e
}

fn sword_trail() -> Effect {
    let mut e = effect(
        "Sword trail",
        "A blade trail with sparkles; preview swings it like a sword.",
        &["trail", "combat", "melee"],
        vec![
            em("Sparkles", 128, Texture::Twinkle)
                .distance(3.0)
                .shape(Shape::Edge {
                    a: v(1.2, 1.4, 0.0),
                    b: v(3.6, 1.4, 0.0),
                })
                .life(0.3, 0.5)
                .size(0.12, 0.22)
                .colors(&[0xE6F4FF, 0x9CCBFF], true)
                .size_ol(&[(0.0, 1.0), (1.0, 0.0)])
                .glow(1.0, 1.6)
                .done(),
        ],
    );
    e.trails = vec![Trail {
        name: "Blade".into(),
        attachment0: v(1.0, 1.4, 0.0),
        attachment1: v(3.8, 1.4, 0.0),
        lifetime: 0.32,
        min_length: 0.05,
        width_scale: Curve::linear(&[(0.0, 1.0), (1.0, 0.35)]),
        color: Gradient::hex(&[(0.0, 0xFFFFFF), (0.3, 0xB6DCFF), (1.0, 0x3E6DFF)]),
        transparency: Curve::smooth(&[(0.0, 0.0), (1.0, 1.0)]),
        face_camera: false,
        light_emission: 0.85,
        brightness: 1.3,
        ..Trail::default()
    }];
    e.preview = PreviewMotion::Swing;
    e
}

fn magic_ribbon() -> Effect {
    let mut e = effect(
        "Magic ribbon",
        "A glowing ribbon that follows a moving object, with a sparkle tail.",
        &["trail", "magic", "ribbon"],
        vec![
            em("Ribbon", 128, Texture::Band)
                .rate(60.0)
                .shape(Shape::Point)
                .life(0.9, 0.9)
                .size(0.7, 0.7)
                .size_ol(&[(0.0, 1.0), (1.0, 0.0)])
                .color_ol(&[(0.0, 0xF2EAFF), (0.4, 0xA88BFF), (1.0, 0x4B2FA8)])
                .fade(&[(0.0, 0.0), (1.0, 1.0)])
                .ribbon()
                .glow(1.0, 1.5)
                .done(),
            em("Sparkle tail", 128, Texture::Twinkle)
                .rate(40.0)
                .shape(Shape::Sphere {
                    radius: 0.25,
                    partial: 1.0,
                })
                .life(0.5, 0.9)
                .vel(Vec3::Y, 0.2, 0.8, 360.0)
                .size(0.1, 0.2)
                .inherit(0.15)
                .colors(&[0xE3D8FF, 0xB497FF], true)
                .size_ol(&[(0.0, 1.0), (1.0, 0.0)])
                .glow(1.0, 1.8)
                .done(),
        ],
    );
    e.preview = PreviewMotion::Orbit;
    e
}

fn crystal_shatter() -> Effect {
    let mut e = one_shot(
        effect(
            "Crystal shatter",
            "Low-poly ice crystal shards bursting and bouncing.",
            &["low-poly", "impact", "ice"],
            vec![
                em("Shards", 64, Texture::Shard)
                    .burst(0.0, 30)
                    .shape_at(
                        Shape::Sphere {
                            radius: 0.4,
                            partial: 1.0,
                        },
                        v(0.0, 1.4, 0.0),
                    )
                    .life(1.2, 1.8)
                    .vel_out(4.0, 8.0, 0.0)
                    .size(0.25, 0.5)
                    .colors(&[0x8FD3FF, 0x5BA8E6, 0xC6ECFF], false)
                    .rot(0.0, 360.0, -360.0, 360.0)
                    .accel(0.0, -14.0, 0.0)
                    .ground(0.35, 0.4, HitAction::Bounce)
                    .erode(&[(0.0, 0.0), (0.7, 0.0), (1.0, 1.0)])
                    .toon()
                    .done(),
                em("Puff", 16, Texture::Hexagon)
                    .burst(0.0, 8)
                    .shape_at(
                        Shape::Sphere {
                            radius: 0.5,
                            partial: 1.0,
                        },
                        v(0.0, 1.4, 0.0),
                    )
                    .life(0.5, 0.8)
                    .vel_out(1.0, 2.0, 0.0)
                    .size(0.6, 0.9)
                    .rot(0.0, 360.0, -60.0, 60.0)
                    .drag(2.0)
                    .colors(&[0xE4F6FF], false)
                    .erode(&[(0.0, 0.0), (1.0, 1.0)])
                    .toon()
                    .done(),
            ],
        ),
        1.9,
    );
    e.style = ArtStyle::LowPoly;
    e
}

fn pickup_sparkle_2d() -> Effect {
    let mut e = one_shot(
        effect(
            "Pickup sparkle",
            "Coin pickup burst for 2D games: twinkles and a ring on the XY plane.",
            &["2d", "ui", "pickup"],
            vec![
                em("Ring", 2, Texture::Ring)
                    .burst(0.0, 1)
                    .shape_at(Shape::Point, v(0.0, 1.5, 0.0))
                    .life(0.35, 0.35)
                    .size(2.6, 2.6)
                    .size_ol(&[(0.0, 0.2), (1.0, 1.0)])
                    .colors(&[0xFFE27A], false)
                    .fade(&[(0.0, 0.0), (1.0, 1.0)])
                    .glow(1.0, 1.3)
                    .done(),
                em("Twinkles", 32, Texture::Twinkle)
                    .burst(0.0, 12)
                    .shape_at(Shape::Point, v(0.0, 1.5, 0.0))
                    .life(0.4, 0.7)
                    .vel(Vec3::Y, 3.0, 6.0, 360.0)
                    .size(0.25, 0.45)
                    .rot(0.0, 45.0, -180.0, 180.0)
                    .drag(4.0)
                    .colors(&[0xFFF3B0, 0xFFD23F], true)
                    .size_ol(&[(0.0, 1.0), (1.0, 0.0)])
                    .glow(1.0, 1.6)
                    .done(),
            ],
        ),
        0.9,
    );
    e.style = ArtStyle::Flat2D;
    e
}

fn waterfall_mist() -> Effect {
    effect(
        "Fountain",
        "A water fountain jet falling back into a pool.",
        &["water", "loop", "environment"],
        vec![
            em("Jet", 512, Texture::Droplet)
                .rate(160.0)
                .shape_at(
                    Shape::Disc {
                        radius: 0.15,
                        inner: 0.0,
                    },
                    v(0.0, 0.3, 0.0),
                )
                .life(1.4, 1.8)
                .vel(Vec3::Y, 8.0, 9.5, 12.0)
                .size(0.1, 0.18)
                .colors(&[0xDDF3FF, 0x9FD3F2], true)
                .accel(0.0, -12.0, 0.0)
                .ground(0.0, 0.0, HitAction::Kill)
                .fade(&[(0.0, 0.1), (1.0, 0.4)])
                .facing(Facing::VelocityParallel)
                .glow(0.3, 1.1)
                .done(),
            em("Spray", 256, Texture::SoftDot)
                .rate(30.0)
                .shape_at(
                    Shape::Disc {
                        radius: 1.6,
                        inner: 0.6,
                    },
                    v(0.0, 0.1, 0.0),
                )
                .life(0.8, 1.2)
                .vel(Vec3::Y, 0.5, 1.5, 60.0)
                .size(0.5, 0.9)
                .drag(1.0)
                .size_ol(&[(0.0, 0.5), (1.0, 1.6)])
                .colors(&[0xEAF7FF], false)
                .fade(&[(0.0, 0.7), (1.0, 1.0)])
                .done(),
        ],
    )
}

/// Every built-in preset, in library order.
#[must_use]
pub fn all() -> Vec<Effect> {
    vec![
        fire(),
        smoke(),
        sparks(),
        magic(),
        explosion(),
        dust_puff(),
        footstep(),
        splash(),
        rain(),
        snow(),
        leaves(),
        confetti(),
        hit_impact(),
        muzzle_flash(),
        heal(),
        portal(),
        shockwave(),
        toon_smear(),
        pixel_spark(),
        lava_bubbles(),
        lightning(),
        sword_trail(),
        magic_ribbon(),
        crystal_shatter(),
        pickup_sparkle_2d(),
        waterfall_mist(),
    ]
}

/// Names of every preset, in library order.
#[must_use]
pub fn names() -> Vec<String> {
    all().into_iter().map(|e| e.name).collect()
}

/// A preset by name (case-insensitive).
///
/// # Errors
/// [`VfxError::NoSuchPreset`] when nothing matches.
pub fn by_name(name: &str) -> Result<Effect, VfxError> {
    all()
        .into_iter()
        .find(|e| e.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| VfxError::NoSuchPreset(name.to_string()))
}

/// File name used for a preset under `presets/` (`"Hit impact"` → `"hit_impact.vfx.ron"`).
#[must_use]
pub fn file_name(effect_name: &str) -> String {
    let slug: String = effect_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{slug}.vfx.ron")
}
