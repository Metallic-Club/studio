//! Manual get/set observations using the default system allocator, without allocation counting.
use metallic_core::classes::default_value_ref;
use metallic_core::{InstanceId, Value, Vec3, World};
use std::hint::black_box;
use std::time::Instant;

fn position(x: f64) -> Value {
    Value::Vector3(Vec3::new(x, 1.0, 0.0))
}

fn fixture() -> (World, InstanceId) {
    // Initialize the shared default registry and grow the journal through its first eviction.
    // Neither fixture allocation nor first-insertion/tree capacity is part of a warmed update.
    black_box(default_value_ref("Part", "Position").unwrap());
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    for index in 0..20_000 {
        world.set(part, "Position", position(index as f64)).unwrap();
    }
    world.set(part, "Position", position(1.0)).unwrap();
    (world, part)
}

#[test]
#[ignore = "manual release get/set timing observations; run in a coordinated quiet window"]
fn release_get_set_observations() {
    if cfg!(debug_assertions) {
        panic!("run with --release");
    }
    let (mut world, part) = fixture();
    const OPERATIONS: usize = 100_000;
    let start = Instant::now();
    for _ in 0..OPERATIONS {
        black_box(world.get(black_box(part), black_box("Position")).unwrap());
    }
    let get_ns = start.elapsed().as_nanos();
    let start = Instant::now();
    for _ in 0..OPERATIONS {
        black_box(
            world
                .get_ref(black_box(part), black_box("Position"))
                .unwrap(),
        );
    }
    let borrowed_get_ns = start.elapsed().as_nanos();
    let start = Instant::now();
    for index in 0..OPERATIONS {
        world
            .set(
                black_box(part),
                black_box("Position"),
                position(index as f64 + 2.0),
            )
            .unwrap();
    }
    let set_ns = start.elapsed().as_nanos();
    assert_eq!(
        world.get(part, "Position").unwrap(),
        position(OPERATIONS as f64 + 1.0)
    );
    eprintln!(
        "property_timing operations={OPERATIONS} get_total_ns={get_ns} borrowed_get_total_ns={borrowed_get_ns} set_total_ns={set_ns} get_ns={:.3} borrowed_get_ns={:.3} set_ns={:.3}",
        get_ns as f64 / OPERATIONS as f64,
        borrowed_get_ns as f64 / OPERATIONS as f64,
        set_ns as f64 / OPERATIONS as f64
    );
}
