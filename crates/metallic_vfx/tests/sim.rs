//! Behaviour tests for the CPU reference simulator.

use metallic_vfx::draw::{beam_point, build_draw_list, flipbook_frames};
use metallic_vfx::roblox::{FlipbookLayout, ParticleEmitter};
use metallic_vfx::{
    Beam, Camera, Curve, Direction, Effect, Emitter, Flipbook, FlipbookMode, HitAction,
    InitModule, Range, Shape, Simulation, SpawnModule, Transform, UpdateModule, Vec3, VfxError,
};

const DT: f32 = 1.0 / 60.0;

fn emitter(spawn: Vec<SpawnModule>, life: f32) -> Emitter {
    Emitter {
        name: "test".into(),
        capacity: 10_000,
        spawn,
        init: vec![
            InitModule::Lifetime {
                seconds: Range::constant(life),
                disabled: false,
            },
            InitModule::Velocity {
                speed: Range::new(1.0, 3.0),
                direction: Direction::Axis(Vec3::Y),
                spread: (90.0, 90.0),
                disabled: false,
            },
        ],
        update: vec![],
        ..Emitter::default()
    }
}

fn single(em: Emitter) -> Effect {
    Effect {
        emitters: vec![em],
        ..Effect::default()
    }
}

fn rate(per_second: f32) -> SpawnModule {
    SpawnModule::Rate {
        per_second,
        disabled: false,
    }
}

fn run(sim: &mut Simulation, seconds: f32) {
    let steps = (seconds / DT).round() as usize;
    for _ in 0..steps {
        sim.step(DT);
    }
}

fn sim(effect: Effect, seed: u32) -> Simulation {
    match Simulation::new(effect, seed) {
        Ok(s) => s,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn same_seed_same_steps_gives_identical_particles() {
    for preset in metallic_vfx::presets::all() {
        let mut a = sim(preset.clone(), 42);
        let mut b = sim(preset.clone(), 42);
        for i in 0..120 {
            let t = preset.preview.transform_at(i as f32 * DT);
            a.set_transform(t);
            b.set_transform(t);
            a.step(DT);
            b.step(DT);
        }
        for (ea, eb) in a.emitters().iter().zip(b.emitters()) {
            assert_eq!(ea.pool, eb.pool, "{} not deterministic", preset.name);
        }
    }
}

#[test]
fn different_seeds_differ() {
    let e = single(emitter(vec![rate(50.0)], 2.0));
    let mut a = sim(e.clone(), 1);
    let mut b = sim(e, 2);
    run(&mut a, 0.5);
    run(&mut b, 0.5);
    assert_eq!(a.particle_count(), b.particle_count());
    assert_ne!(a.emitters()[0].pool.position_age, b.emitters()[0].pool.position_age);
}

#[test]
fn restart_replays_identically() {
    let e = metallic_vfx::presets::all().remove(0);
    let mut s = sim(e, 9);
    run(&mut s, 1.0);
    let first = s.emitters()[1].pool.clone();
    s.restart();
    run(&mut s, 1.0);
    assert_eq!(first, s.emitters()[1].pool);
}

#[test]
fn rate_spawns_exact_counts() {
    // 10/s for 1 s with 60 steps must give exactly 10 particles, despite float accumulation.
    let mut s = sim(single(emitter(vec![rate(10.0)], 5.0)), 1);
    run(&mut s, 1.0);
    assert_eq!(s.particle_count(), 10);
    run(&mut s, 2.0);
    assert_eq!(s.particle_count(), 30);
}

#[test]
fn rate_count_is_independent_of_step_size() {
    for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 144.0, 0.1] {
        let mut s = sim(single(emitter(vec![rate(37.0)], 10.0)), 1);
        let steps = (2.0 / dt as f64).round() as usize;
        for _ in 0..steps {
            s.step(dt);
        }
        assert!(
            (s.particle_count() as i64 - 74).abs() <= 1,
            "dt {dt}: {}",
            s.particle_count()
        );
    }
}

#[test]
fn steady_state_count_is_rate_times_lifetime() {
    let mut s = sim(single(emitter(vec![rate(100.0)], 1.5)), 3);
    run(&mut s, 5.0);
    let n = s.particle_count() as i64;
    assert!((n - 150).abs() <= 2, "count {n}");
}

#[test]
fn bursts_fire_once_and_repeat_on_cycles() {
    let burst = SpawnModule::Burst {
        time: 0.5,
        count: 25,
        cycles: 3,
        interval: 0.25,
        disabled: false,
    };
    let mut s = sim(single(emitter(vec![burst], 10.0)), 1);
    run(&mut s, 0.49);
    assert_eq!(s.particle_count(), 0);
    run(&mut s, 0.05);
    assert_eq!(s.particle_count(), 25);
    run(&mut s, 1.0);
    assert_eq!(s.particle_count(), 75);
}

#[test]
fn looping_effects_repeat_bursts_each_cycle() {
    let mut e = single(emitter(
        vec![SpawnModule::Burst {
            time: 0.0,
            count: 10,
            cycles: 1,
            interval: 0.0,
            disabled: false,
        }],
        0.1,
    ));
    e.duration = 1.0;
    e.looping = true;
    let mut s = sim(e.clone(), 1);
    let mut spawned_cycles = 0;
    let mut prev = 0;
    for _ in 0..(60 * 3 - 6) {
        s.step(DT);
        let n = s.particle_count();
        if n > prev {
            spawned_cycles += 1;
        }
        prev = n;
    }
    assert_eq!(spawned_cycles, 3);

    e.looping = false;
    let mut once = sim(e, 1);
    run(&mut once, 3.0);
    assert!(once.is_finished());
}

#[test]
fn particles_die_at_their_lifetime() {
    let mut s = sim(single(emitter(vec![SpawnModule::Burst {
        time: 0.0,
        count: 50,
        cycles: 1,
        interval: 0.0,
        disabled: false,
    }], 0.5)), 1);
    s.step(DT);
    assert_eq!(s.particle_count(), 50);
    for p in &s.emitters()[0].pool.velocity_life {
        assert_eq!(p[3], 0.5);
    }
    run(&mut s, 0.45);
    assert_eq!(s.particle_count(), 50);
    run(&mut s, 0.1);
    assert_eq!(s.particle_count(), 0);
}

#[test]
fn zero_lifetime_spawns_nothing_and_lifetime_is_capped() {
    let mut s = sim(single(emitter(vec![rate(100.0)], 0.0)), 1);
    run(&mut s, 1.0);
    assert_eq!(s.particle_count(), 0);
    let mut long = sim(single(emitter(vec![rate(10.0)], 60.0)), 1);
    run(&mut long, 0.5);
    assert!(long.emitters()[0].pool.velocity_life.iter().all(|v| v[3] == 20.0));
}

#[test]
fn capacity_is_respected() {
    let mut em = emitter(vec![rate(1000.0)], 10.0);
    em.capacity = 64;
    let mut s = sim(single(em), 1);
    run(&mut s, 2.0);
    assert_eq!(s.particle_count(), 64);
}

#[test]
fn bounds_contain_every_particle() {
    for preset in metallic_vfx::presets::all() {
        let mut s = sim(preset.clone(), 5);
        for i in 0..90 {
            s.set_transform(preset.preview.transform_at(i as f32 * DT));
            s.step(DT);
        }
        if s.particle_count() == 0 {
            continue;
        }
        let Some(b) = s.bounds() else {
            panic!("{} has particles but no bounds", preset.name);
        };
        for (state, em) in s.emitters().iter().zip(&preset.emitters) {
            for i in 0..state.pool.len() {
                let p = state.pool.position(i);
                let p = if em.space == metallic_vfx::SimSpace::Local {
                    s.transform().point(p)
                } else {
                    p
                };
                assert!(b.contains(p), "{}: {p:?} outside {b:?}", preset.name);
            }
        }
    }
}

#[test]
fn roblox_drag_halves_speed_per_inverse_drag_seconds() {
    let mut em = emitter(
        vec![SpawnModule::Burst {
            time: 0.0,
            count: 1,
            cycles: 1,
            interval: 0.0,
            disabled: false,
        }],
        10.0,
    );
    em.init[1] = InitModule::Velocity {
        speed: Range::constant(8.0),
        direction: Direction::Axis(Vec3::X),
        spread: (0.0, 0.0),
        disabled: false,
    };
    em.update = vec![UpdateModule::Drag {
        drag: 2.0,
        disabled: false,
    }];
    let mut s = sim(single(em), 1);
    s.step(DT);
    run(&mut s, 0.5);
    let speed = s.emitters()[0].pool.velocity(0).length();
    // Born during the first step, aged ~0.5 s of drag: speed 8 * 2^(-2 * 0.5) = 4.
    assert!((speed - 4.0).abs() < 0.15, "speed {speed}");
}

#[test]
fn acceleration_and_collision_plane() {
    let mut em = emitter(
        vec![SpawnModule::Burst {
            time: 0.0,
            count: 20,
            cycles: 1,
            interval: 0.0,
            disabled: false,
        }],
        5.0,
    );
    em.init.push(InitModule::Shape {
        shape: Shape::Point,
        surface: false,
        rotation: Vec3::ZERO,
        offset: Vec3::new(0.0, 3.0, 0.0),
        disabled: false,
    });
    em.update = vec![
        UpdateModule::Acceleration {
            accel: Vec3::new(0.0, -20.0, 0.0),
            disabled: false,
        },
        UpdateModule::CollisionPlane {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            bounce: 0.5,
            friction: 0.0,
            on_hit: HitAction::Bounce,
            disabled: false,
        },
    ];
    let mut s = sim(single(em), 1);
    for _ in 0..240 {
        s.step(DT);
        for i in 0..s.emitters()[0].pool.len() {
            assert!(s.emitters()[0].pool.position(i).y >= -1e-4);
        }
    }
}

#[test]
fn attractor_kill_radius_removes_particles() {
    let mut em = emitter(vec![rate(50.0)], 10.0);
    em.update = vec![UpdateModule::Attractor {
        position: Vec3::ZERO,
        strength: 0.0,
        radius: 1.0,
        kill_radius: 100.0,
        disabled: false,
    }];
    let mut s = sim(single(em), 1);
    run(&mut s, 1.0);
    // Only particles born during the last step (not yet updated) can be alive.
    assert!(s.particle_count() <= 1, "{}", s.particle_count());
}

#[test]
fn time_scale_zero_freezes_the_emitter() {
    let mut em = emitter(vec![rate(50.0)], 10.0);
    em.time_scale = 0.0;
    let mut s = sim(single(em), 1);
    run(&mut s, 1.0);
    assert_eq!(s.particle_count(), 0);
}

#[test]
fn fixed_fps_steps_in_chunks() {
    let mut em = emitter(vec![rate(12.0)], 10.0);
    em.fixed_fps = 12.0;
    let mut s = sim(single(em), 1);
    // 4 frames of 1/60 s are under one 1/12 s step: nothing happens yet.
    run(&mut s, 4.0 / 60.0);
    assert_eq!(s.particle_count(), 0);
    run(&mut s, 1.0 / 60.0);
    assert_eq!(s.particle_count(), 1);
}

#[test]
fn emit_and_clear_match_roblox() {
    let mut s = sim(single(emitter(vec![], 5.0)), 1);
    assert_eq!(s.emit(0, 16), Ok(()));
    assert_eq!(s.emit(3, 1), Err(VfxError::NoSuchEmitter(3)));
    s.step(DT);
    assert_eq!(s.particle_count(), 16);
    s.clear();
    assert_eq!(s.particle_count(), 0);
}

#[test]
fn moving_emitter_spawns_along_its_path() {
    let mut s = sim(single(emitter(vec![rate(600.0)], 5.0)), 1);
    s.teleport(Transform::from_position(Vec3::ZERO));
    s.set_transform(Transform::from_position(Vec3::new(10.0, 0.0, 0.0)));
    s.step(DT);
    let xs: Vec<f32> = (0..s.particle_count())
        .map(|i| s.emitters()[0].pool.position(i).x)
        .collect();
    assert_eq!(xs.len(), 10);
    let min = xs.iter().copied().fold(f32::INFINITY, f32::min);
    let max = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(min < 2.0 && max > 8.0, "spread {min}..{max}");
}

#[test]
fn distance_spawning_counts_studs() {
    let mut em = emitter(
        vec![SpawnModule::Distance {
            per_stud: 2.0,
            disabled: false,
        }],
        5.0,
    );
    em.capacity = 1000;
    let mut s = sim(single(em), 1);
    for i in 1..=60 {
        s.set_transform(Transform::from_position(Vec3::new(i as f32 * 0.1, 0.0, 0.0)));
        s.step(DT);
    }
    assert_eq!(s.particle_count(), 12);
}

#[test]
fn bad_dt_is_ignored() {
    let mut s = sim(single(emitter(vec![rate(10.0)], 5.0)), 1);
    s.step(f32::NAN);
    s.step(-1.0);
    s.step(f32::INFINITY);
    assert_eq!(s.time(), 0.0);
    s.step(100.0);
    assert!(s.time() <= metallic_vfx::sim::MAX_STEP);
}

#[test]
fn invalid_effects_are_rejected() {
    let mut e = single(emitter(vec![rate(f32::NAN)], 1.0));
    assert!(matches!(Simulation::new(e.clone(), 1), Err(VfxError::Invalid { .. })));
    e.emitters[0].spawn = vec![rate(1.0)];
    e.emitters[0].capacity = 0;
    assert!(matches!(e.validate(), Err(VfxError::Invalid { .. })));
}

#[test]
fn roblox_particle_emitter_maps_to_same_behaviour() {
    let pe = ParticleEmitter {
        rate: 1000.0,
        lifetime: Range::constant(1.0),
        transparency: Curve::linear(&[(0.0, 0.0), (1.0, 1.0)]),
        light_emission: 1.0,
        flipbook_layout: FlipbookLayout::Grid4x4,
        flipbook_framerate: Range::constant(90.0),
        ..ParticleEmitter::default()
    };
    let em = pe.to_emitter("Roblox");
    // Rate is capped at Roblox's 400/s.
    assert!(matches!(em.spawn[0], SpawnModule::Rate { per_second, .. } if per_second == 400.0));
    let fb = em.render.flipbook.clone();
    assert_eq!(fb.map(|f| (f.frames(), f.fps.max)), Some((16, 30.0)));
    let mut s = sim(single(em), 1);
    run(&mut s, 0.5);
    let n = s.particle_count() as i64;
    assert!((n - 200).abs() <= 1, "count {n}");
    // Transparency 0 -> 1 over life: a half-life particle is about half transparent.
    let pool = &s.emitters()[0].pool;
    for i in 0..pool.len() {
        let t = pool.position_age[i][3] / pool.velocity_life[i][3];
        assert!((pool.tint[i][3] - (1.0 - t)).abs() < 0.02);
    }
    // LightEmission 1 = additive: premultiplied alpha is 0.
    let cam = Camera::perspective(Vec3::new(0.0, 2.0, 12.0), Vec3::ZERO, 50.0);
    let list = build_draw_list(&s, &cam);
    assert!(!list.quads.is_empty());
    assert!(list.quads.iter().all(|q| q.colors[0][3] == 0.0));
}

#[test]
fn flipbook_modes() {
    let fb = |mode| Flipbook {
        columns: 2,
        rows: 2,
        mode,
        fps: Range::constant(4.0),
        start_random: false,
        blend: false,
    };
    // OneShot spans the lifetime regardless of fps.
    assert_eq!(flipbook_frames(&fb(FlipbookMode::OneShot), 0.0, 2.0, 0.5, 1).0, 0);
    assert_eq!(flipbook_frames(&fb(FlipbookMode::OneShot), 1.99, 2.0, 0.5, 1).0, 3);
    // Loop at 4 fps: frame 1 at 0.25 s, wraps after 1 s.
    assert_eq!(flipbook_frames(&fb(FlipbookMode::Loop), 0.3, 9.0, 0.5, 1).0, 1);
    assert_eq!(flipbook_frames(&fb(FlipbookMode::Loop), 1.05, 9.0, 0.5, 1).0, 0);
    // PingPong goes 0 1 2 3 2 1 0.
    assert_eq!(flipbook_frames(&fb(FlipbookMode::PingPong), 1.3, 9.0, 0.5, 1).0, 1);
    let (a, _, _) = flipbook_frames(&fb(FlipbookMode::Random), 0.6, 9.0, 0.5, 7);
    assert!(a < 4);
}

#[test]
fn beam_control_points_follow_roblox() {
    let b = Beam {
        attachment0: Vec3::ZERO,
        attachment1: Vec3::new(10.0, 0.0, 0.0),
        direction0: Vec3::Y,
        direction1: Vec3::Y,
        curve_size0: 3.0,
        curve_size1: 3.0,
        ..Beam::default()
    };
    // P1 = A0 + up*3, P2 = A1 - up*3: the midpoint is (5, 0, 0) and the quarter point is above.
    let mid = beam_point(&b, 0.5);
    assert!((mid - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4);
    assert!(beam_point(&b, 0.25).y > 0.5);
    assert!((beam_point(&b, 0.0) - b.attachment0).length() < 1e-5);
    assert!((beam_point(&b, 1.0) - b.attachment1).length() < 1e-5);
}

#[test]
fn draw_list_is_sorted_back_to_front() {
    let e = match metallic_vfx::presets::by_name("Explosion") {
        Ok(e) => e,
        Err(e) => panic!("{e}"),
    };
    let mut s = sim(e, 1);
    run(&mut s, 0.3);
    let cam = Camera::perspective(Vec3::new(0.0, 3.0, 14.0), Vec3::new(0.0, 1.5, 0.0), 45.0);
    let list = build_draw_list(&s, &cam);
    assert!(list.quads.len() > 50);
    for w in list.quads.windows(2) {
        assert!(w[0].depth >= w[1].depth);
    }
    for q in &list.quads {
        assert!((q.texture as usize) < list.textures.len());
    }
}

#[test]
fn trails_follow_motion_and_expire() {
    let e = match metallic_vfx::presets::by_name("Sword trail") {
        Ok(e) => e,
        Err(e) => panic!("{e}"),
    };
    let mut s = sim(e.clone(), 1);
    for i in 0..60 {
        s.set_transform(e.preview.transform_at(i as f32 * DT));
        s.step(DT);
    }
    let n = s.trails()[0].points.len();
    assert!(n > 5, "trail has {n} points");
    assert!(s.trails()[0].points.iter().all(|p| p.age <= e.trails[0].lifetime + DT));
}
