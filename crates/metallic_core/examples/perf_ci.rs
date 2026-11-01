//! Fixed core workloads shared by the instruction gate and wall-time trend collector.
//!
//! Setup, mutation validation and output are outside the named, non-inlined Callgrind collection
//! functions. Each creation/destruction sample performs exactly 1,000 individual Part mutations.
use metallic_core::{CoreError, InstanceId, Value, Vec3, World};
use std::hint::black_box;
use std::time::Instant;

const PARTS: usize = 1_000;
const PROPERTIES: [&str; 7] = [
    "Position",
    "Orientation",
    "Size",
    "Anchored",
    "CanCollide",
    "Material",
    "Shape",
];

fn fixture() -> Result<(World, Vec<InstanceId>), CoreError> {
    let mut world = World::new_with_services();
    let parent = world.root("Workspace").ok_or(CoreError::InvalidInstance)?;
    let mut parts = Vec::with_capacity(PARTS);
    for index in 0..PARTS {
        let id = world.create("Part", Some(parent))?;
        world.set(
            id,
            "Position",
            Value::Vector3(Vec3::new(index as f64, 1.0, 0.0)),
        )?;
        world.set(id, "Anchored", Value::Bool(true))?;
        parts.push(id);
    }
    // Initialize lazy registry/allocator state before entering a measured function.
    for property in PROPERTIES {
        black_box(world.get(parts[0], property)?);
    }
    Ok((world, parts))
}

// SAFETY: each exported name is unique in this standalone executable; no foreign callers exist.
#[unsafe(no_mangle)]
#[inline(never)]
fn metallic_perf_reads(world: &World, parts: &[InstanceId]) -> Result<(), CoreError> {
    for _ in 0..10 {
        for &id in parts {
            for property in PROPERTIES {
                black_box(world.get(black_box(id), black_box(property))?);
            }
        }
    }
    Ok(())
}

// SAFETY: this symbol is used only to select collection within this executable.
#[unsafe(no_mangle)]
#[inline(never)]
fn metallic_perf_writes(world: &mut World, parts: &[InstanceId]) -> Result<(), CoreError> {
    for pass in 1..=10 {
        for &id in parts {
            world.set(
                black_box(id),
                "Position",
                Value::Vector3(Vec3::new(pass as f64, 2.0, 3.0)),
            )?;
        }
    }
    Ok(())
}

// SAFETY: this symbol is used only to select collection within this executable.
#[unsafe(no_mangle)]
#[inline(never)]
fn metallic_perf_clone(world: &World) {
    for _ in 0..3 {
        black_box(world.deep_clone());
    }
}

// SAFETY: this symbol is used only to select collection within this executable.
#[unsafe(no_mangle)]
#[inline(never)]
fn metallic_perf_create(world: &mut World, parent: InstanceId) -> Result<(), CoreError> {
    for _ in 0..PARTS {
        black_box(world.create(black_box("Part"), Some(black_box(parent)))?);
    }
    Ok(())
}

// SAFETY: this symbol is used only to select collection within this executable.
#[unsafe(no_mangle)]
#[inline(never)]
fn metallic_perf_destroy(world: &mut World, parts: &[InstanceId]) {
    for &id in parts {
        world.destroy(black_box(id));
    }
}

// Keep completion checks outside Callgrind collection and elapsed_ns. The collector must not
// accept a successful process that skipped the requested mutations.
fn validate_mutator(
    workload: &str,
    world: &World,
    parts: &[InstanceId],
    parent: InstanceId,
) -> Result<(), &'static str> {
    match workload {
        "create" if world.children(parent).len() != 2 * PARTS => {
            Err("creation workload did not add every Part")
        }
        "destroy"
            if !world.children(parent).is_empty() || parts.iter().any(|&id| world.is_alive(id)) =>
        {
            Err("destruction workload did not remove every Part")
        }
        _ => Ok(()),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let workload = std::env::args()
        .nth(1)
        .ok_or("choose reads, writes, clone, create or destroy")?;
    let (mut world, parts) = fixture()?;
    let parent = world.root("Workspace").ok_or(CoreError::InvalidInstance)?;
    let start = Instant::now();
    match workload.as_str() {
        "reads" => metallic_perf_reads(&world, &parts)?,
        "writes" => metallic_perf_writes(&mut world, &parts)?,
        "clone" => metallic_perf_clone(&world),
        "create" => metallic_perf_create(&mut world, parent)?,
        "destroy" => metallic_perf_destroy(&mut world, &parts),
        _ => return Err("choose reads, writes, clone, create or destroy".into()),
    }
    let elapsed = start.elapsed().as_nanos();
    validate_mutator(&workload, &world, &parts, parent)?;
    println!("{{\"workload\":\"{workload}\",\"elapsed_ns\":{elapsed},\"completed\":true}}");
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("performance workload failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_adds_exactly_one_thousand_parts_under_the_existing_workspace() {
        let (mut world, original) = fixture().unwrap();
        let parent = world.root("Workspace").unwrap();
        metallic_perf_create(&mut world, parent).unwrap();
        validate_mutator("create", &world, &original, parent).unwrap();
        let children = world.children(parent);
        assert_eq!(&children[..PARTS], original.as_slice());
        for &id in &children[PARTS..] {
            assert_eq!(world.class_name(id), Some("Part"));
            assert_eq!(world.parent(id), Some(parent));
        }
    }

    #[test]
    fn destruction_removes_all_precreated_parts_but_keeps_workspace() {
        let (mut world, parts) = fixture().unwrap();
        let parent = world.root("Workspace").unwrap();
        metallic_perf_destroy(&mut world, &parts);
        validate_mutator("destroy", &world, &parts, parent).unwrap();
        assert!(world.is_alive(parent));
        assert_eq!(world.root("Workspace"), Some(parent));
    }

    #[test]
    fn skipped_mutations_fail_completion_validation() {
        let (world, parts) = fixture().unwrap();
        let parent = world.root("Workspace").unwrap();
        assert!(validate_mutator("create", &world, &parts, parent).is_err());
        assert!(validate_mutator("destroy", &world, &parts, parent).is_err());
    }
}
