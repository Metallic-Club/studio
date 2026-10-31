//! Fixed core workloads shared by the instruction gate and wall-time trend collector.
//!
//! Setup and output are outside the named, non-inlined Callgrind collection functions.
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

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let workload = std::env::args()
        .nth(1)
        .ok_or("choose reads, writes or clone")?;
    let (mut world, parts) = fixture()?;
    let start = Instant::now();
    match workload.as_str() {
        "reads" => metallic_perf_reads(&world, &parts)?,
        "writes" => metallic_perf_writes(&mut world, &parts)?,
        "clone" => metallic_perf_clone(&world),
        _ => return Err("choose reads, writes or clone".into()),
    }
    let elapsed = start.elapsed().as_nanos();
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
