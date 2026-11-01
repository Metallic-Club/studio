//! Requested-heap fixtures and storage-component prototypes for research issue #187.
//!
//! Run `cargo run --release -p metallic_core --example compact_storage -- memory` or `-- cpu`.
//! Output is JSON Lines. Production World storage is unchanged; component rows omit World overhead.
#![warn(missing_docs)]

#[path = "support/overrides.rs"]
mod overrides;
#[path = "support/requested_heap.rs"]
mod requested_heap;

use metallic_core::{CoreError, InstanceId, Value, Vec3, World, classes::default_value};
use overrides::{KEYS, KeyMap, NamedMap, Overrides, SortedValues};
use requested_heap::{Counts, measure};
use std::hint::black_box;
use std::mem::size_of;
use std::time::Instant;

type ProbeResult<T> = Result<T, Box<dyn std::error::Error>>;

fn schema() -> ProbeResult<Vec<Value>> {
    KEYS.iter()
        .enumerate()
        .map(|(index, key)| {
            default_value(if index == 8 { "Player" } else { "Part" }, key)
                .ok_or_else(|| format!("schema property missing: {key}").into())
        })
        .collect()
}

fn changed_value(key: usize, index: usize) -> Value {
    match key {
        0 => Value::Vector3(Vec3::new(index as f64, 1.0, 0.0)),
        1 => Value::Vector3(Vec3::new(0.0, index as f64 + 1.0, 0.0)),
        2 => Value::Vector3(Vec3::new(4.0, 2.0, 3.0)),
        3 => Value::Bool(true),
        4 => Value::Bool(false),
        5 => Value::String("Metal".into()),
        6 => Value::String("Ball".into()),
        _ => Value::Nil,
    }
}

fn level(count: usize, overrides: usize) -> Result<(World, Vec<InstanceId>), CoreError> {
    let mut world = World::new_with_services();
    let model = world.create("Model", world.root("Workspace"))?;
    let mut parts = Vec::with_capacity(count);
    for index in 0..count {
        let part = world.create("Part", Some(model))?;
        for (key, name) in KEYS.iter().enumerate().take(overrides) {
            world.set(part, name, changed_value(key, index))?;
        }
        parts.push(part);
    }
    // Saturate history without changing Part override density. One temporary slot/free-list
    // allocation is retained and included in the reported fixture heap.
    for _ in 0..20_000 {
        let temporary = world.create("Folder", world.root("Workspace"))?;
        world.destroy(temporary);
    }
    Ok((world, parts))
}

fn stores<S: Overrides>(count: usize, overridden: usize, defaults: &[Value]) -> Vec<S> {
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let mut value = S::default();
        for key in 0..overridden {
            value.set(key, changed_value(key, index), defaults);
        }
        result.push(value);
    }
    result
}

fn live(counts: Counts) -> ProbeResult<usize> {
    counts
        .allocated
        .checked_sub(counts.freed)
        .ok_or_else(|| "interval freed pre-existing allocations".into())
}

fn memory_row(
    label: &str,
    count: usize,
    overridden: usize,
    counts: Counts,
    stack_bytes: usize,
) -> ProbeResult<()> {
    let retained = live(counts)?;
    println!(
        "{{\"kind\":\"memory\",\"scope\":\"{label}\",\"parts\":{count},\"overrides\":{overridden},\"live_requested_bytes\":{retained},\"requested_bytes\":{},\"freed_bytes\":{},\"allocation_calls\":{},\"outer_stack_bytes\":{stack_bytes}}}",
        counts.allocated, counts.freed, counts.allocations
    );
    Ok(())
}

fn component_memory<S: Overrides>(
    count: usize,
    overridden: usize,
    defaults: &[Value],
) -> ProbeResult<()> {
    let (values, counts) = measure(|| stores::<S>(count, overridden, defaults));
    if values.iter().any(|value| value.len() != overridden) {
        return Err("prototype override density differs".into());
    }
    memory_row(S::LABEL, count, overridden, counts, size_of::<Vec<S>>())?;
    black_box(values);
    Ok(())
}

fn memory(defaults: &[Value], schema_heap: usize) -> ProbeResult<()> {
    // Warm global registry/runtime initialization outside measured intervals.
    black_box(level(1, 7)?);
    println!(
        "{{\"kind\":\"layout\",\"value_bytes\":{},\"id_value_entry_bytes\":{},\"named_header_bytes\":{},\"id_map_header_bytes\":{},\"sorted_header_bytes\":{},\"world_stack_bytes\":{},\"journal_entry_bytes\":{},\"prototype_schema_requested_bytes\":{}}}",
        size_of::<Value>(),
        size_of::<(u16, Value)>(),
        size_of::<NamedMap>(),
        size_of::<KeyMap>(),
        size_of::<SortedValues>(),
        size_of::<World>(),
        size_of::<(u64, InstanceId)>(),
        schema_heap
    );
    for count in [300, 5_000] {
        for overridden in [0, 2, 7] {
            let (fixture, counts) = measure(|| level(count, overridden));
            let (world, parts) = fixture?;
            if parts.len() != count || world.get(parts[0], KEYS[0]).is_err() {
                return Err("world fixture failed".into());
            }
            memory_row(
                "world_total",
                count,
                overridden,
                counts,
                size_of::<World>() + size_of::<Vec<InstanceId>>(),
            )?;
            black_box((world, parts));
            component_memory::<NamedMap>(count, overridden, defaults)?;
            component_memory::<KeyMap>(count, overridden, defaults)?;
            component_memory::<SortedValues>(count, overridden, defaults)?;
        }
    }
    Ok(())
}

fn cpu_row<S: Overrides>(
    count: usize,
    overridden: usize,
    sample: usize,
    defaults: &[Value],
) -> ProbeResult<()> {
    let mut values = stores::<S>(count, overridden, defaults);
    if values[0].read_name("unknown", defaults).is_some() {
        return Err("unknown property accepted".into());
    }
    let start = Instant::now();
    for _ in 0..20 {
        for value in black_box(&values) {
            for key in 0..7 {
                black_box(value.read(black_box(key), defaults));
            }
        }
    }
    let reads = start.elapsed().as_nanos();
    let start = Instant::now();
    for pass in 1..=20 {
        for value in black_box(&mut values) {
            value.set(
                0,
                Value::Vector3(Vec3::new(f64::from(pass), 2.0, 3.0)),
                defaults,
            );
        }
    }
    let writes = start.elapsed().as_nanos();
    // Explicit default removal and fallback are checked outside the timed update body.
    values[0].set(0, defaults[0].clone(), defaults);
    if values[0].stored(0).is_some() || values[0].read(0, defaults) != &defaults[0] {
        return Err("default fallback failed".into());
    }
    println!(
        "{{\"kind\":\"cpu_component\",\"scope\":\"{}\",\"parts\":{count},\"overrides\":{overridden},\"sample\":{sample},\"read_operations\":{},\"write_operations\":{},\"read_ns\":{reads},\"write_ns\":{writes}}}",
        S::LABEL,
        count * 7 * 20,
        count * 20
    );
    Ok(())
}

fn cpu(defaults: &[Value]) -> ProbeResult<()> {
    for count in [300, 5_000] {
        for overridden in [0, 2, 7] {
            for sample in 0..6 {
                // Rotate execution order to expose rather than conceal frequency/load drift.
                match sample % 3 {
                    0 => {
                        cpu_row::<NamedMap>(count, overridden, sample, defaults)?;
                        cpu_row::<KeyMap>(count, overridden, sample, defaults)?;
                        cpu_row::<SortedValues>(count, overridden, sample, defaults)?;
                    }
                    1 => {
                        cpu_row::<KeyMap>(count, overridden, sample, defaults)?;
                        cpu_row::<SortedValues>(count, overridden, sample, defaults)?;
                        cpu_row::<NamedMap>(count, overridden, sample, defaults)?;
                    }
                    _ => {
                        cpu_row::<SortedValues>(count, overridden, sample, defaults)?;
                        cpu_row::<NamedMap>(count, overridden, sample, defaults)?;
                        cpu_row::<KeyMap>(count, overridden, sample, defaults)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn run() -> ProbeResult<()> {
    let (defaults, schema_counts) = measure(schema);
    let defaults = defaults?;
    match std::env::args().nth(1).as_deref().unwrap_or("memory") {
        "memory" => memory(&defaults, live(schema_counts)?),
        "cpu" => cpu(&defaults),
        _ => Err("choose memory or cpu".into()),
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("storage research failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
