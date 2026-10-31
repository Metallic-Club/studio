//! Stored CFrames keep a right-handed orthonormal rotation (issue #363): `World::set` and the
//! scene loader fix scaled or skewed matrices (keeping the look vector) and refuse degenerate ones.
use metallic_core::*;

mod common;
use common::TestDir;

fn world_and_part() -> (World, InstanceId) {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let part = world.create("Part", Some(workspace)).unwrap();
    (world, part)
}

fn cframe(position: Vec3, rotation: [[f64; 3]; 3]) -> Value {
    Value::CFrame(Box::new(CFrame::from_parts(position, rotation)))
}

fn stored(world: &World, id: InstanceId, prop: &str) -> CFrame {
    match world.get(id, prop).unwrap() {
        Value::CFrame(cf) => *cf,
        other => panic!("{prop} is not a CFrame: {other:?}"),
    }
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn close(a: Vec3, b: Vec3, eps: f64) -> bool {
    (a - b).magnitude() < eps
}

/// Unit, perpendicular, right-handed axes.
fn assert_orthonormal(cf: CFrame) {
    let (r, u, b) = (cf.right_vector(), cf.up_vector(), cf.back_vector());
    for axis in [r, u, b] {
        assert!((axis.magnitude() - 1.0).abs() < 1e-12, "{cf:?}");
    }
    assert!(dot(r, u).abs() < 1e-12 && dot(u, b).abs() < 1e-12 && dot(b, r).abs() < 1e-12);
    let rxu = Vec3::new(
        r.y * u.z - r.z * u.y,
        r.z * u.x - r.x * u.z,
        r.x * u.y - r.y * u.x,
    );
    assert!(close(rxu, b, 1e-12), "not right-handed: {cf:?}");
}

/// The write is refused like a NaN one: an InvalidValue error and no change, journal entry or
/// signal.
fn assert_refused(world: &mut World, id: InstanceId, prop: &str, value: Value, reason: &str) {
    let old = world.get(id, prop).unwrap();
    let before = world.change_cursor();
    let fired = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = std::sync::Arc::clone(&fired);
    world.property_changed(id, prop).connect(move |_| {
        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    match world.set(id, prop, value) {
        Err(CoreError::InvalidValue {
            property,
            reason: why,
        }) => {
            assert_eq!(property, prop);
            assert!(why.contains(reason), "{why}");
        }
        other => panic!("expected InvalidValue, got {other:?}"),
    }
    assert_eq!(world.get(id, prop).unwrap(), old);
    assert_eq!(world.change_cursor(), before);
    assert_eq!(fired.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn zero_rotation_cframe_is_refused() {
    let (mut world, part) = world_and_part();
    let zero = cframe(Vec3::new(1.0, 2.0, 3.0), [[0.0; 3]; 3]);
    assert_refused(&mut world, part, "PivotOffset", zero, "degenerate");
    // One zero column, and two parallel columns, are degenerate too.
    let one_zero = cframe(
        Vec3::default(),
        [[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    );
    assert_refused(&mut world, part, "PivotOffset", one_zero, "degenerate");
    let parallel = cframe(
        Vec3::default(),
        [[1.0, 2.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    );
    assert_refused(&mut world, part, "PivotOffset", parallel, "degenerate");
    let model = world.create("Model", None).unwrap();
    let zero = cframe(Vec3::default(), [[0.0; 3]; 3]);
    assert_refused(&mut world, model, "WorldPivot", zero, "degenerate");
}

#[test]
fn scaled_rotation_cframe_is_normalized() {
    let (mut world, part) = world_and_part();
    let position = Vec3::new(1.0, 2.0, 3.0);
    let turned = CFrame::from_euler_xyz(0.3, -0.7, 1.1).rotation;
    let scaled = turned.map(|row| row.map(|v| v * 2.0));
    world
        .set(part, "PivotOffset", cframe(position, scaled))
        .unwrap();
    let cf = stored(&world, part, "PivotOffset");
    assert_orthonormal(cf);
    assert_eq!(cf.position, position);
    for (got, want) in cf.rotation.iter().flatten().zip(turned.iter().flatten()) {
        assert!((got - want).abs() < 1e-12, "{cf:?}");
    }
    // 2 x identity is the identity rotation.
    let doubled = [[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0]];
    world
        .set(part, "PivotOffset", cframe(position, doubled))
        .unwrap();
    assert_eq!(
        stored(&world, part, "PivotOffset"),
        CFrame::from_position(position)
    );
}

#[test]
fn skewed_rotation_cframe_is_normalized() {
    let (mut world, part) = world_and_part();
    // Right (1, 0, 0), up (1, 1, 0), back (0, 0.5, 2): every pair is skewed.
    let skewed = CFrame::from_axes(
        Vec3::new(4.0, 5.0, 6.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(0.0, 0.5, 2.0),
    );
    world
        .set(part, "PivotOffset", Value::CFrame(Box::new(skewed)))
        .unwrap();
    let cf = stored(&world, part, "PivotOffset");
    assert_orthonormal(cf);
    assert_eq!(cf.position, skewed.position);
    // The look vector keeps its direction; up stays on the side it pointed to.
    assert!(close(cf.look_vector(), skewed.look_vector().unit(), 1e-12));
    assert!(dot(cf.up_vector(), skewed.up_vector()) > 0.0);
    // Stored values are a fixed point: writing it back changes nothing.
    let before = world.change_cursor();
    world
        .set(part, "PivotOffset", Value::CFrame(Box::new(cf)))
        .unwrap();
    assert_eq!(stored(&world, part, "PivotOffset"), cf);
    assert_eq!(world.change_cursor(), before);
    // The pivot math that relies on an orthonormal rotation now holds.
    let p = Vec3::new(1.0, -2.0, 3.0);
    assert!(close(cf.inverse() * (cf * p), p, 1e-12));
}

#[test]
fn nearly_orthonormal_cframe_is_kept() {
    let (mut world, part) = world_and_part();
    // An exact rotation comes back bit for bit.
    let exact =
        CFrame::from_position(Vec3::new(1.0, 2.0, 3.0)) * CFrame::from_euler_xyz(0.2, 0.5, -0.9);
    world
        .set(part, "PivotOffset", Value::CFrame(Box::new(exact)))
        .unwrap();
    assert_eq!(stored(&world, part, "PivotOffset"), exact);
    // Float drift well under the tolerance (an f32 round trip, say) is kept bit for bit too.
    let mut drifted = exact;
    drifted.rotation[0][1] += 1e-7;
    drifted.rotation[2][2] -= 1e-7;
    world
        .set(part, "PivotOffset", Value::CFrame(Box::new(drifted)))
        .unwrap();
    assert_eq!(stored(&world, part, "PivotOffset"), drifted);
    // A 1e-6 drift is accepted and stays within about 1e-6 of what was written.
    let mut drifted = exact;
    drifted.rotation[0][1] += 1e-6;
    world
        .set(part, "PivotOffset", Value::CFrame(Box::new(drifted)))
        .unwrap();
    let cf = stored(&world, part, "PivotOffset");
    for (got, want) in cf.components().iter().zip(drifted.components()) {
        assert!((got - want).abs() <= 2e-6, "{got} vs {want}");
    }
}

#[test]
fn nan_cframe_is_still_refused() {
    let (mut world, part) = world_and_part();
    let mut rotation = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    rotation[1][2] = f64::NAN;
    let nan = cframe(Vec3::default(), rotation);
    assert_refused(&mut world, part, "PivotOffset", nan, "finite");
    let inf = Value::CFrame(Box::new(CFrame::from_position(Vec3::new(
        f64::INFINITY,
        0.0,
        0.0,
    ))));
    assert_refused(&mut world, part, "PivotOffset", inf, "finite");
}

#[test]
fn scene_load_normalizes_cframe() {
    let dir = TestDir::new("scene_load_normalizes_cframe");
    let (mut world, part) = world_and_part();
    let position = Vec3::new(1.0, 2.0, 3.0);
    // World::set refuses to store a bad matrix, so a scene with one has to be edited by hand.
    world
        .set(
            part,
            "PivotOffset",
            Value::CFrame(Box::new(CFrame::from_position(position))),
        )
        .unwrap();
    let path = dir.file("skewed.ron");
    save_scene(&world, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let identity = ron::to_string(&CFrame::IDENTITY.rotation).unwrap();
    let skewed = [[2.0, 2.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0]];
    assert!(text.contains(&identity), "{text}");
    let edited = text.replace(&identity, &ron::to_string(&skewed).unwrap());
    std::fs::write(&path, edited).unwrap();

    let loaded = load_scene(&path).unwrap();
    let part = loaded.children(loaded.root("Workspace").unwrap())[0];
    let cf = stored(&loaded, part, "PivotOffset");
    assert_orthonormal(cf);
    assert_eq!(cf.position, position);
    assert!(close(cf.look_vector(), Vec3::new(0.0, 0.0, -1.0), 1e-12));
    assert!(close(
        cf.up_vector(),
        Vec3::new(1.0, 1.0, 0.0).unit(),
        1e-12
    ));

    // A degenerate matrix in a scene file fails the load.
    std::fs::write(
        &path,
        text.replace(&identity, &ron::to_string(&[[0.0; 3]; 3]).unwrap()),
    )
    .unwrap();
    assert!(matches!(
        load_scene(&path),
        Err(CoreError::InvalidValue { property, .. }) if property == "PivotOffset"
    ));
}
