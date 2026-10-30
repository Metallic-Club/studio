//! Short release-mode budget cost probe; run on the same machine before and after a change.
//!
//! `cargo run --release -p metallic_script --example budget_bench -- 100000`
//! Reports observed sample percentiles, not confidence intervals or a performance guarantee.

use metallic_script::{Context, ScriptVm};
use mlua::{Function, Lua};
use std::error::Error;
use std::hint::black_box;
use std::time::{Duration, Instant};

const SAMPLES: usize = 20;
const LOOP: &str = "return function(n) local s = 0 for i = 1, n do s += i end return s end";

fn measure(
    name: &str,
    batch: usize,
    mut operation: impl FnMut() -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    operation()?;
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        for _ in 0..batch {
            operation()?;
        }
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let per_op = |duration: Duration| duration.as_nanos() as f64 / batch as f64;
    println!(
        "{name}: samples={SAMPLES} batch={batch} p50_ns/op={:.1} p95_ns/op={:.1}",
        per_op(samples[SAMPLES / 2]),
        per_op(samples[SAMPLES * 95 / 100 - 1]),
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let iterations: u32 = std::env::args()
        .nth(1)
        .map_or(Ok(100_000), |value| value.parse())?;
    println!(
        "budget_probe loop_iterations={iterations}; concurrent machine load is not controlled"
    );
    measure("vm_create_shutdown", 10, || {
        ScriptVm::new(Context::Server)?.shutdown();
        Ok(())
    })?;
    let vm = ScriptVm::new(Context::Server)?;
    let short: Function = vm
        .lua()
        .load("return function(a,b) return a+b end")
        .eval()?;
    measure("budgeted_short_call", 1_000, || {
        black_box(vm.call_with_budget::<i64>(&short, (2, 3))?);
        Ok(())
    })?;
    let loop_function: Function = vm.lua().load(LOOP).eval()?;
    measure("budgeted_arithmetic_loop", 1, || {
        black_box(vm.call_with_budget::<f64>(&loop_function, iterations)?);
        Ok(())
    })?;
    vm.shutdown();
    let lua = Lua::new();
    let bare: Function = lua.load(LOOP).eval()?;
    measure("bare_luau_arithmetic_loop", 1, || {
        black_box(bare.call::<f64>(iterations)?);
        Ok(())
    })?;
    Ok(())
}
