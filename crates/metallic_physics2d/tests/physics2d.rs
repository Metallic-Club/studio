//! Behaviour tests for the 2D rigid-body world.
use metallic_physics2d::*;

const DT: f64 = 1.0 / 60.0;

fn world() -> PhysicsWorld2d {
    PhysicsWorld2d::new(WorldConfig::default()).expect("default config is valid")
}

/// A wide static floor whose top surface is at y = 0.
fn floor(id: u64) -> BodyDesc {
    BodyDesc::new(BodyId(id), BodyKind::Static, Vec2::new(0.0, -2.0))
        .with_collider(ColliderDesc::rect(400.0, 4.0))
}

fn crate_box(id: u64, x: f64, y: f64, size: f64) -> BodyDesc {
    BodyDesc::new(BodyId(id), BodyKind::Dynamic, Vec2::new(x, y))
        .with_collider(ColliderDesc::rect(size, size))
}

fn run(w: &mut PhysicsWorld2d, scene: &mut Scene2d, seconds: f64) {
    let frames = (seconds / DT).round() as usize;
    for _ in 0..frames {
        w.step(scene, DT).expect("valid dt");
    }
}

fn pos(scene: &Scene2d, id: u64) -> Vec2 {
    scene.body(BodyId(id)).expect("body exists").position
}

#[test]
fn falling_body_hits_ground_at_expected_time() {
    // Box bottom starts 50 studs above the floor: t = sqrt(2h/g).
    let h = 50.0;
    let mut scene = Scene2d {
        bodies: vec![floor(1), crate_box(2, 0.0, h + 1.0, 2.0)],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    let expected = (2.0 * h / GRAVITY).sqrt();
    let mut hit_time = None;
    for frame in 1..=120 {
        w.step(&mut scene, DT).expect("valid dt");
        if w
            .drain_touched()
            .iter()
            .any(|&(a, b)| (a, b) == (BodyId(1), BodyId(2)) || (a, b) == (BodyId(2), BodyId(1)))
        {
            hit_time = Some(frame as f64 * DT);
            break;
        }
    }
    let t = hit_time.expect("the box reached the floor");
    assert!(
        (t - expected).abs() <= 2.0 * DT,
        "touched at {t:.3} s, expected {expected:.3} s"
    );
    run(&mut w, &mut scene, 2.0);
    let rest = pos(&scene, 2);
    assert!((rest.y - 1.0).abs() < 0.05, "box rests on the floor: {rest:?}");
}

#[test]
fn stacking_ten_boxes_stays_stable() {
    let mut bodies = vec![floor(1)];
    for i in 0..10 {
        bodies.push(crate_box(10 + i, 0.0, 2.0 + 4.0 * i as f64 + 0.01 * i as f64, 4.0));
    }
    let mut scene = Scene2d {
        bodies,
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    run(&mut w, &mut scene, 10.0);
    for i in 0..10u64 {
        let p = pos(&scene, 10 + i);
        let expected_y = 2.0 + 4.0 * i as f64;
        assert!(p.x.abs() < 0.05, "box {i} drifted sideways: {p:?}");
        assert!((p.y - expected_y).abs() < 0.15, "box {i} sank or jumped: {p:?}");
        let s = w.body_state(BodyId(10 + i)).expect("body exists");
        assert!(s.pose.angle.abs() < 0.01, "box {i} tilted: {}", s.pose.angle);
        assert!(
            s.sleeping || s.linear_velocity.length() < 0.05,
            "box {i} still moving: {:?}",
            s.linear_velocity
        );
    }
}

#[test]
fn one_way_platform_lets_bodies_up_and_holds_them_from_above() {
    let platform = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(0.0, 20.0))
        .with_collider(ColliderDesc::rect(30.0, 1.0).one_way_up());
    let mut scene = Scene2d {
        bodies: vec![
            floor(1),
            platform,
            // Below the platform, will be launched up through it.
            crate_box(3, 0.0, 1.0, 2.0),
            // Above the platform, will fall onto it.
            crate_box(4, 8.0, 40.0, 2.0),
        ],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    w.set_velocity(BodyId(3), Vec2::new(0.0, 100.0), 0.0)
        .expect("dynamic body");
    run(&mut w, &mut scene, 3.0);
    let launched = pos(&scene, 3);
    let dropped = pos(&scene, 4);
    // Platform top is at 20.5; a 2-stud box resting on it has its center at 21.5.
    assert!(
        (launched.y - 21.5).abs() < 0.1,
        "launched box passed up through and landed on top: {launched:?}"
    );
    assert!(
        (dropped.y - 21.5).abs() < 0.1,
        "dropped box is held by the platform: {dropped:?}"
    );
}

#[test]
fn sensors_fire_enter_and_exit_and_never_block() {
    let trigger = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(0.0, 20.0))
        .with_collider(ColliderDesc::rect(10.0, 4.0).sensor());
    let mut scene = Scene2d {
        bodies: vec![floor(1), trigger, crate_box(3, 0.0, 40.0, 2.0)],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    let mut events = Vec::new();
    for _ in 0..180 {
        w.step(&mut scene, DT).expect("valid dt");
        events.extend(w.drain_sensor_events());
    }
    assert_eq!(
        events,
        vec![
            SensorEvent {
                sensor: BodyId(2),
                other: BodyId(3),
                kind: SensorEventKind::Enter
            },
            SensorEvent {
                sensor: BodyId(2),
                other: BodyId(3),
                kind: SensorEventKind::Exit
            },
        ]
    );
    assert!((pos(&scene, 3).y - 1.0).abs() < 0.05, "fell through the sensor");
}

#[test]
fn removing_a_body_inside_a_sensor_reports_exit() {
    let trigger = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(0.0, 1.0))
        .with_collider(ColliderDesc::rect(10.0, 4.0).sensor());
    let mut scene = Scene2d {
        bodies: vec![floor(1), trigger, crate_box(3, 0.0, 1.0, 2.0)],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    run(&mut w, &mut scene, 0.2);
    let enter = w.drain_sensor_events();
    assert_eq!(enter.len(), 1);
    scene.bodies.retain(|b| b.id != BodyId(3));
    w.sync_from(&scene).expect("valid scene");
    let exit = w.drain_sensor_events();
    assert_eq!(exit.len(), 1);
    assert_eq!(exit[0].kind, SensorEventKind::Exit);
}

#[test]
fn raycast_hits_nearest_and_respects_filters() {
    let sensor = BodyDesc::new(BodyId(5), BodyKind::Static, Vec2::new(0.0, 20.0))
        .with_collider(ColliderDesc::rect(10.0, 2.0).sensor());
    let shelf = BodyDesc::new(BodyId(6), BodyKind::Static, Vec2::new(0.0, 10.0))
        .with_collider(ColliderDesc::rect(10.0, 2.0));
    let scene = Scene2d {
        bodies: vec![floor(1), sensor, shelf],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    let down = Vec2::new(0.0, -100.0);
    let hit = w
        .raycast(Vec2::new(0.0, 30.0), down, &RayFilter::default())
        .expect("hits the shelf");
    assert_eq!(hit.body, BodyId(6));
    assert!((hit.distance - 19.0).abs() < 1e-3, "distance {}", hit.distance);
    assert!((hit.normal.y - 1.0).abs() < 1e-4);
    assert!((hit.point.y - 11.0).abs() < 1e-3);

    let hit = w
        .raycast(
            Vec2::new(0.0, 30.0),
            down,
            &RayFilter {
                ignore: &[BodyId(6)],
                ..RayFilter::default()
            },
        )
        .expect("hits the floor");
    assert_eq!(hit.body, BodyId(1));

    let hit = w
        .raycast(
            Vec2::new(0.0, 30.0),
            down,
            &RayFilter {
                include_sensors: true,
                ..RayFilter::default()
            },
        )
        .expect("hits the sensor");
    assert_eq!(hit.body, BodyId(5));

    // Too short, zero and NaN directions miss.
    assert!(w.raycast(Vec2::new(0.0, 30.0), Vec2::new(0.0, -5.0), &RayFilter::default()).is_none());
    assert!(w.raycast(Vec2::new(0.0, 30.0), Vec2::ZERO, &RayFilter::default()).is_none());
    assert!(
        w.raycast(Vec2::new(0.0, 30.0), Vec2::new(f64::NAN, -1.0), &RayFilter::default())
            .is_none()
    );
    assert_eq!(w.bodies_at_point(Vec2::new(0.0, 20.0)), vec![BodyId(5)]);
}

#[test]
fn moving_a_static_platform_carries_the_box_on_it() {
    let platform = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(0.0, 10.0))
        .with_collider(ColliderDesc::rect(20.0, 2.0));
    let mut scene = Scene2d {
        bodies: vec![platform, crate_box(3, 0.0, 12.0, 2.0)],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    run(&mut w, &mut scene, 0.5);
    let start = pos(&scene, 3);
    // Move the platform 10 studs/s to the right for 2 s, like a script writing Position.
    for frame in 1..=120 {
        if let Some(p) = scene.body_mut(BodyId(2)) {
            p.position.x = frame as f64 * DT * 10.0;
        }
        w.step(&mut scene, DT).expect("valid dt");
        if frame == 60 {
            let v = w.body_state(BodyId(2)).expect("platform").linear_velocity;
            assert!((v.x - 10.0).abs() < 1e-6, "mover velocity is derived: {v:?}");
        }
    }
    let end = pos(&scene, 3);
    assert!(
        (end.x - start.x - 20.0).abs() < 0.5,
        "box was carried ~20 studs: {start:?} -> {end:?}"
    );
    assert!((end.y - 12.0).abs() < 0.1, "box stayed on top: {end:?}");
    assert!((pos(&scene, 2).x - 20.0).abs() < 1e-9, "platform landed exactly");
}

#[test]
fn kinematic_body_pushes_dynamic_bodies() {
    let pusher = BodyDesc::new(BodyId(2), BodyKind::Kinematic, Vec2::new(-10.0, 2.0))
        .with_collider(ColliderDesc::rect(2.0, 4.0));
    let mut scene = Scene2d {
        bodies: vec![floor(1), pusher, crate_box(3, 0.0, 1.0, 2.0)],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    for frame in 1..=120 {
        if let Some(p) = scene.body_mut(BodyId(2)) {
            p.position.x = -10.0 + frame as f64 * DT * 10.0;
        }
        w.step(&mut scene, DT).expect("valid dt");
    }
    // Pusher ends at x = 10; its right face at 11; the box (half width 1) is ahead of it.
    let b = pos(&scene, 3);
    assert!(b.x > 11.5, "box was pushed: {b:?}");
}

/// Builds a busy scene: a pyramid, joints, a one-way platform and a sensor.
fn busy_scene() -> Scene2d {
    let mut bodies = vec![floor(1)];
    let mut id = 100;
    for row in 0..8 {
        for col in 0..(8 - row) {
            let x = (col as f64 - (8 - row) as f64 * 0.5) * 2.1;
            bodies.push(crate_box(id, x, 1.0 + row as f64 * 2.05, 2.0));
            id += 1;
        }
    }
    bodies.push(
        BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(30.0, 30.0))
            .with_collider(ColliderDesc::rect(1.0, 1.0)),
    );
    bodies.push(
        BodyDesc::new(BodyId(3), BodyKind::Dynamic, Vec2::new(38.0, 30.0))
            .with_collider(ColliderDesc::circle(1.0)),
    );
    bodies.push(
        BodyDesc::new(BodyId(4), BodyKind::Static, Vec2::new(-30.0, 10.0))
            .with_collider(ColliderDesc::rect(10.0, 1.0).one_way_up()),
    );
    bodies.push(
        BodyDesc::new(BodyId(5), BodyKind::Static, Vec2::new(0.0, 3.0))
            .with_collider(ColliderDesc::rect(40.0, 6.0).sensor()),
    );
    for i in 0..5 {
        bodies.push(
            BodyDesc::new(BodyId(200 + i), BodyKind::Dynamic, Vec2::new(-30.0 + i as f64, 20.0 + 3.0 * i as f64))
                .with_collider(ColliderDesc::circle(0.8)),
        );
    }
    Scene2d {
        bodies,
        joints: vec![JointDesc {
            id: JointId(1),
            body_a: BodyId(2),
            body_b: BodyId(3),
            kind: JointKind::Revolute {
                anchor_a: Vec2::ZERO,
                anchor_b: Vec2::new(-8.0, 0.0),
                limits: None,
                motor: None,
            },
            collide_connected: false,
        }],
    }
}

fn simulate(frames: usize) -> (Scene2d, Vec<(BodyId, BodyId)>, Vec<SensorEvent>, Vec2) {
    let mut scene = busy_scene();
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    w.add_character(BodyId(50), CharacterConfig::default(), Vec2::new(-20.0, 3.0))
        .expect("character");
    let mut touched = Vec::new();
    let mut sensor = Vec::new();
    for f in 0..frames {
        let input = CharacterInput {
            move_x: if (f / 40) % 2 == 0 { 1.0 } else { -0.5 },
            jump: f % 50 < 10,
            drop_through: false,
        };
        w.set_character_input(BodyId(50), input).expect("character");
        // Uneven frame times exercise the accumulator.
        let dt = if f % 3 == 0 { 0.021 } else { 0.0137 };
        w.step(&mut scene, dt).expect("valid dt");
        touched.extend(w.drain_touched());
        sensor.extend(w.drain_sensor_events());
    }
    let c = w.character(BodyId(50)).expect("character").position;
    (scene, touched, sensor, c)
}

#[test]
fn same_input_gives_bit_identical_results() {
    let a = simulate(400);
    let b = simulate(400);
    assert_eq!(a.0, b.0, "body poses differ");
    assert_eq!(a.1, b.1, "touch events differ");
    assert_eq!(a.2, b.2, "sensor events differ");
    assert_eq!(a.3, b.3, "character differs");
    assert!(!a.1.is_empty() && !a.2.is_empty(), "the scene produced events");
}

#[test]
fn non_finite_or_negative_dt_is_rejected_and_changes_nothing() {
    let mut scene = Scene2d {
        bodies: vec![floor(1), crate_box(2, 0.0, 100.0, 2.0)],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -DT] {
        assert_eq!(
            w.step(&mut scene, bad).map_err(|e| matches!(e, Physics2dError::InvalidDt(_))),
            Err(true)
        );
    }
    assert_eq!(w.tick_count(), 0);
    assert_eq!(pos(&scene, 2).y, 100.0);
    run(&mut w, &mut scene, 1.0);
    assert!(pos(&scene, 2).y < 100.0 - 50.0, "physics still runs after bad dt");
    // A huge frame is clamped to max_frame_time (0.25 s = 15 ticks).
    let ticks = w.step(&mut scene, 10.0).expect("valid dt");
    assert_eq!(ticks, 15);
}

#[test]
fn invalid_scenes_are_rejected_without_changes() {
    let mut w = world();
    let good = Scene2d {
        bodies: vec![floor(1)],
        joints: vec![],
    };
    w.sync_from(&good).expect("valid scene");
    let dup = Scene2d {
        bodies: vec![floor(1), floor(1)],
        joints: vec![],
    };
    assert_eq!(w.sync_from(&dup), Err(Physics2dError::DuplicateBody(BodyId(1))));
    let bad_poly = Scene2d {
        bodies: vec![
            floor(1),
            BodyDesc::new(BodyId(2), BodyKind::Dynamic, Vec2::ZERO).with_collider(ColliderDesc::new(
                Shape2d::ConvexPolygon {
                    points: vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(2.0, 2.0)],
                },
            )),
        ],
        joints: vec![],
    };
    assert!(matches!(
        w.sync_from(&bad_poly),
        Err(Physics2dError::InvalidShape { .. })
    ));
    let bad_joint = Scene2d {
        bodies: vec![floor(1)],
        joints: vec![JointDesc {
            id: JointId(1),
            body_a: BodyId(1),
            body_b: BodyId(9),
            kind: JointKind::Weld {
                anchor_a: Vec2::ZERO,
                anchor_b: Vec2::ZERO,
            },
            collide_connected: false,
        }],
    };
    assert!(matches!(
        w.sync_from(&bad_joint),
        Err(Physics2dError::InvalidJoint { .. })
    ));
    // The world still has exactly the floor.
    assert_eq!(w.debug_shapes().len(), 1);
    assert!(PhysicsWorld2d::new(WorldConfig {
        tick: 0.0,
        ..WorldConfig::default()
    })
    .is_err());
}

#[test]
fn collision_groups_filter_contacts() {
    let ghost_groups = CollisionGroups {
        memberships: 0b10,
        filter: 0b01, // only the floor's group
    };
    let floor_body = BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::new(0.0, -2.0)).with_collider(
        ColliderDesc::rect(400.0, 4.0).with_groups(CollisionGroups {
            memberships: 0b01,
            filter: u32::MAX,
        }),
    );
    let lower = BodyDesc::new(BodyId(2), BodyKind::Dynamic, Vec2::new(0.0, 1.0))
        .with_collider(ColliderDesc::rect(2.0, 2.0).with_groups(ghost_groups));
    let upper = BodyDesc::new(BodyId(3), BodyKind::Dynamic, Vec2::new(0.0, 6.0))
        .with_collider(ColliderDesc::rect(2.0, 2.0).with_groups(ghost_groups));
    let mut scene = Scene2d {
        bodies: vec![floor_body, lower, upper],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    run(&mut w, &mut scene, 2.0);
    // The two ghosts do not collide with each other: both end on the floor, overlapping.
    assert!((pos(&scene, 2).y - 1.0).abs() < 0.05);
    assert!((pos(&scene, 3).y - 1.0).abs() < 0.05);
}

#[test]
fn joints_hold_their_constraints() {
    let anchor = BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::new(0.0, 50.0))
        .with_collider(ColliderDesc::rect(1.0, 1.0));
    let bob = BodyDesc::new(BodyId(2), BodyKind::Dynamic, Vec2::new(10.0, 50.0))
        .with_collider(ColliderDesc::circle(1.0));
    let rope_end = BodyDesc::new(BodyId(3), BodyKind::Dynamic, Vec2::new(-5.0, 50.0))
        .with_collider(ColliderDesc::circle(0.5));
    let weld_a = BodyDesc::new(BodyId(4), BodyKind::Dynamic, Vec2::new(40.0, 10.0))
        .with_collider(ColliderDesc::rect(2.0, 2.0));
    let weld_b = BodyDesc::new(BodyId(5), BodyKind::Dynamic, Vec2::new(43.0, 10.0))
        .with_collider(ColliderDesc::rect(2.0, 2.0));
    let rail = BodyDesc::new(BodyId(6), BodyKind::Static, Vec2::new(-40.0, 40.0))
        .with_collider(ColliderDesc::rect(1.0, 1.0));
    let slider = BodyDesc::new(BodyId(7), BodyKind::Dynamic, Vec2::new(-40.0, 40.0))
        .with_collider(ColliderDesc::rect(1.0, 1.0));
    let mut scene = Scene2d {
        bodies: vec![floor(100), anchor, bob, rope_end, weld_a, weld_b, rail, slider],
        joints: vec![
            JointDesc {
                id: JointId(1),
                body_a: BodyId(1),
                body_b: BodyId(2),
                kind: JointKind::Revolute {
                    anchor_a: Vec2::ZERO,
                    anchor_b: Vec2::new(-10.0, 0.0),
                    limits: None,
                    motor: None,
                },
                collide_connected: false,
            },
            JointDesc {
                id: JointId(2),
                body_a: BodyId(1),
                body_b: BodyId(3),
                kind: JointKind::Distance {
                    anchor_a: Vec2::ZERO,
                    anchor_b: Vec2::ZERO,
                    min_length: 0.0,
                    max_length: 8.0,
                    spring: None,
                },
                collide_connected: false,
            },
            JointDesc {
                id: JointId(3),
                body_a: BodyId(4),
                body_b: BodyId(5),
                kind: JointKind::Weld {
                    anchor_a: Vec2::new(1.5, 0.0),
                    anchor_b: Vec2::new(-1.5, 0.0),
                },
                collide_connected: false,
            },
            JointDesc {
                id: JointId(4),
                body_a: BodyId(6),
                body_b: BodyId(7),
                kind: JointKind::Prismatic {
                    anchor_a: Vec2::ZERO,
                    anchor_b: Vec2::ZERO,
                    axis: Vec2::new(0.0, 1.0),
                    limits: Some((-10.0, 0.0)),
                    motor: None,
                },
                collide_connected: false,
            },
        ],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    let mut max_rope: f64 = 0.0;
    for _ in 0..180 {
        w.step(&mut scene, DT).expect("valid dt");
        let d = (pos(&scene, 2) - Vec2::new(0.0, 50.0)).length();
        assert!((d - 10.0).abs() < 0.1, "pendulum arm length kept: {d}");
        max_rope = max_rope.max((pos(&scene, 3) - Vec2::new(0.0, 50.0)).length());
    }
    assert!(max_rope < 8.15, "rope never stretched past 8: {max_rope}");
    assert!(max_rope > 7.5, "rope went taut: {max_rope}");
    let a = pos(&scene, 4);
    let b = pos(&scene, 5);
    assert!(((b - a).length() - 3.0).abs() < 0.05, "weld kept spacing");
    let sa = w.body_state(BodyId(4)).expect("body").pose.angle;
    let sb = w.body_state(BodyId(5)).expect("body").pose.angle;
    assert!((sa - sb).abs() < 0.01, "weld kept relative angle");
    let s = pos(&scene, 7);
    assert!((s.x + 40.0).abs() < 0.01, "slider stayed on its axis: {s:?}");
    assert!((s.y - 30.0).abs() < 0.1, "slider stopped at its lower limit: {s:?}");
}

#[test]
fn revolute_motor_spins_a_wheel() {
    let axle = BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::new(0.0, 20.0))
        .with_collider(ColliderDesc::rect(0.5, 0.5).sensor());
    let wheel = BodyDesc::new(BodyId(2), BodyKind::Dynamic, Vec2::new(0.0, 20.0))
        .with_collider(ColliderDesc::circle(2.0));
    let mut scene = Scene2d {
        bodies: vec![axle, wheel],
        joints: vec![JointDesc {
            id: JointId(1),
            body_a: BodyId(1),
            body_b: BodyId(2),
            kind: JointKind::Revolute {
                anchor_a: Vec2::ZERO,
                anchor_b: Vec2::ZERO,
                limits: None,
                motor: Some(JointMotor {
                    target_velocity: 3.0,
                    max_force: 1.0e5,
                }),
            },
            collide_connected: false,
        }],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    run(&mut w, &mut scene, 2.0);
    let s = w.body_state(BodyId(2)).expect("wheel");
    assert!((s.angular_velocity - 3.0).abs() < 0.2, "wheel spins at the motor speed: {}", s.angular_velocity);
}

#[test]
fn tilemap_and_edge_chain_terrain_hold_bodies() {
    // 10 x 2 tiles of 4 studs with a gap in the middle of the top row.
    let mut solid = vec![true; 10];
    solid.extend((0..10).map(|c| c != 4 && c != 5));
    let tiles = BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::new(-20.0, -8.0)).with_collider(
        ColliderDesc::new(Shape2d::Tilemap {
            tile_size: Vec2::new(4.0, 4.0),
            columns: 10,
            solid,
        }),
    );
    let ramp = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(60.0, 0.0)).with_collider(
        ColliderDesc::new(Shape2d::EdgeChain {
            points: vec![Vec2::new(-20.0, 20.0), Vec2::new(0.0, 0.0), Vec2::new(30.0, 0.0)],
            closed: false,
        }),
    );
    let on_tiles = crate_box(3, -10.0, 10.0, 2.0);
    let in_gap = crate_box(4, -1.0, 10.0, 2.0);
    let ball = BodyDesc::new(BodyId(5), BodyKind::Dynamic, Vec2::new(42.0, 21.0))
        .with_collider(ColliderDesc::circle(1.0));
    let mut scene = Scene2d {
        bodies: vec![tiles, ramp, on_tiles, in_gap, ball],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    run(&mut w, &mut scene, 3.0);
    // Tile top is at -8 + 8 = 0; the gap floor at -4.
    assert!((pos(&scene, 3).y - 1.0).abs() < 0.05, "{:?}", pos(&scene, 3));
    assert!((pos(&scene, 4).y + 3.0).abs() < 0.05, "{:?}", pos(&scene, 4));
    let b = pos(&scene, 5);
    assert!(b.x > 60.0 && (b.y - 1.0).abs() < 0.1, "ball rolled down the ramp: {b:?}");
    // Merged: 10x2 tiles minus 2 = 3 rectangles in the debug view.
    let tile_shapes = w.debug_shapes().iter().filter(|s| s.body == BodyId(1)).count();
    assert_eq!(tile_shapes, 3);
}

#[test]
fn bullets_do_not_tunnel_through_thin_walls() {
    let wall = BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::new(20.0, 0.0))
        .with_collider(ColliderDesc::rect(0.2, 20.0));
    let mut bullet = BodyDesc::new(BodyId(2), BodyKind::Dynamic, Vec2::new(0.0, 0.0))
        .with_collider(ColliderDesc::circle(0.2))
        .bullet();
    bullet.gravity_scale = 0.0;
    let mut scene = Scene2d {
        bodies: vec![wall, bullet],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    w.set_velocity(BodyId(2), Vec2::new(1500.0, 0.0), 0.0)
        .expect("dynamic body");
    run(&mut w, &mut scene, 0.5);
    assert!(pos(&scene, 2).x < 20.0, "bullet stopped at the wall: {:?}", pos(&scene, 2));
}

#[test]
fn moving_a_dynamic_body_in_the_scene_teleports_it() {
    let mut scene = Scene2d {
        bodies: vec![floor(1), crate_box(2, 0.0, 1.0, 2.0)],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    run(&mut w, &mut scene, 0.5);
    if let Some(b) = scene.body_mut(BodyId(2)) {
        b.position = Vec2::new(50.0, 30.0);
    }
    w.step(&mut scene, DT).expect("valid dt");
    let p = pos(&scene, 2);
    assert!((p.x - 50.0).abs() < 1e-3 && p.y > 29.0, "teleported: {p:?}");
    assert!(w.apply_impulse(BodyId(1), Vec2::new(1.0, 0.0)).is_err(), "floor is static");
    assert!(w.apply_impulse(BodyId(2), Vec2::new(f64::NAN, 0.0)).is_err());
}

#[test]
fn render_pose_interpolates_between_ticks() {
    let mut scene = Scene2d {
        bodies: vec![crate_box(2, 0.0, 100.0, 2.0)],
        joints: vec![],
    };
    let mut w = world();
    w.sync_from(&scene).expect("valid scene");
    w.step(&mut scene, DT * 2.5).expect("valid dt");
    assert!((w.interpolation_alpha() - 0.5).abs() < 1e-6);
    let current = w.body_state(BodyId(2)).expect("body").pose.position.y;
    let shown = w.render_pose(BodyId(2)).expect("body").position.y;
    assert!(shown > current, "rendered pose lags half a tick behind the simulation");
}
