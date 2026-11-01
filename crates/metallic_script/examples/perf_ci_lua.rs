//! Fixed Luau workloads for the instruction gate (`docs/performance-ci.md`): property reads,
//! property writes and method calls from a real `Script`, through the public `ScriptHost`.
//!
//! Collection covers one `ScriptHost::step`, which resumes a script thread that does the fixed
//! work and waits again. Part creation, compiling and starting the script, a one-pass warm-up,
//! printing the result and validating it all happen outside the named Callgrind functions and
//! outside `elapsed_ns`. The work runs on a resumed thread rather than in a Heartbeat handler
//! because a handler is deferred once the step's wall-clock budget has passed, which a run under
//! Valgrind could reach; the first due thread of a step is always resumed.
use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_script::{OutputSink, ScriptHost};
use std::cell::RefCell;
use std::error::Error;
use std::hint::black_box;
use std::rc::Rc;
use std::time::{Duration, Instant};

const PARTS: usize = 1_000;
const PASSES: usize = 10;
const USAGE: &str = "choose lua_reads, lua_writes or lua_method_calls";

/// Wall-clock budget of one resume. Valgrind slows the VM far past the default 0.1 s, and a
/// timeout would end the workload early (validation then refuses the run).
const RESUME_BUDGET: Duration = Duration::from_secs(3_600);

/// Before every measured step: the script's first `task.wait(1)`, started at time 0, is still
/// pending, so this step only initializes lazy host state.
const WARM_STEP_AT: f64 = 0.25;
/// After every measured step: the script's second `task.wait(1)` has ended, so it prints.
const REPORT_STEP_AT: f64 = 10.0;
const STEP_DT: f64 = 1.0 / 60.0;

/// Reads a number, a boolean and a string per Part. Returns their checksum.
const READS: &str = r#"
local function run(passes)
    local sum = 0
    for _ = 1, passes do
        for i = 1, count do
            local part = parts[i]
            sum += part.Transparency
            if part.Anchored then sum += 1 end
            sum += #part.Name
        end
    end
    return sum
end
"#;

/// Writes a number and a boolean per Part; every pass changes both values on every Part.
/// Returns how many passes have written, the warm-up included.
const WRITES: &str = r#"
local written = 0
local function run(passes)
    for _ = 1, passes do
        written += 1
        local transparency = written / 16
        local canCollide = written % 2 == 1
        for i = 1, count do
            local part = parts[i]
            part.Transparency = transparency
            part.CanCollide = canCollide
        end
    end
    return written
end
"#;

/// Calls three methods per Part: one returning a boolean, one finding nothing and one returning
/// an Instance. Returns how many calls gave the expected answer.
const METHOD_CALLS: &str = r#"
local function run(passes)
    local hits = 0
    for _ = 1, passes do
        for i = 1, count do
            local part = parts[i]
            if part:IsA("BasePart") then hits += 1 end
            if part:FindFirstChild("Missing") == nil then hits += 1 end
            if container:FindFirstChild("Part") ~= nil then hits += 1 end
        end
    end
    return hits
end
"#;

fn script_source(body: &str) -> String {
    format!(
        "local container = workspace
local parts = container:GetChildren()
local count = #parts
assert(count == {PARTS})
{body}
run(1)
task.wait(1)
local result = run({PASSES})
task.wait(1)
print(result)
"
    )
}

/// Everything a script says, kept for validation outside collection.
#[derive(Default)]
struct Capture {
    printed: RefCell<Vec<String>>,
    problems: RefCell<Vec<String>>,
}

impl OutputSink for Capture {
    fn print(&self, line: &str) {
        self.printed.borrow_mut().push(line.to_string());
    }
    fn warn(&self, line: &str) {
        self.problems.borrow_mut().push(line.to_string());
    }
    fn error(&self, line: &str) {
        self.problems.borrow_mut().push(line.to_string());
    }
}

struct Fixture {
    world: Rc<RefCell<World>>,
    host: ScriptHost,
    output: Rc<Capture>,
    parts: Vec<InstanceId>,
}

fn fixture(body: &str) -> Result<Fixture, Box<dyn Error>> {
    let mut world = World::new_with_services();
    let parent = world.root("Workspace").ok_or("no Workspace")?;
    let mut parts = Vec::with_capacity(PARTS);
    for index in 0..PARTS {
        let id = world.create("Part", Some(parent))?;
        world.set(
            id,
            "Position",
            Value::Vector3(Vec3::new(index as f64, 1.0, 0.0)),
        )?;
        world.set(id, "Anchored", Value::Bool(true))?;
        // Quarters add up exactly, so the reads checksum needs no tolerance.
        world.set(id, "Transparency", Value::Number((index % 4) as f64 * 0.25))?;
        parts.push(id);
    }
    let services = world
        .root("ServerScriptService")
        .ok_or("no ServerScriptService")?;
    let script = world.create("Script", Some(services))?;
    world.set(script, "Source", Value::String(script_source(body)))?;

    let world = Rc::new(RefCell::new(world));
    let output = Rc::new(Capture::default());
    let mut host = ScriptHost::new_server(world.clone(), output.clone());
    host.set_resume_budget(RESUME_BUDGET);
    host.start()?;
    host.step(WARM_STEP_AT, STEP_DT);
    Ok(Fixture {
        world,
        host,
        output,
        parts,
    })
}

// Each function steps to a different time on purpose. With identical bodies the compiler may merge
// them into one symbol, and Callgrind's --toggle-collect would then collect nothing for the others.

// SAFETY: each exported name is unique in this standalone executable; no foreign callers exist.
#[unsafe(no_mangle)]
#[inline(never)]
fn metallic_perf_lua_reads(host: &mut ScriptHost) {
    host.step(black_box(2.0), black_box(STEP_DT));
}

// SAFETY: this symbol is used only to select collection within this executable.
#[unsafe(no_mangle)]
#[inline(never)]
fn metallic_perf_lua_writes(host: &mut ScriptHost) {
    host.step(black_box(3.0), black_box(STEP_DT));
}

// SAFETY: this symbol is used only to select collection within this executable.
#[unsafe(no_mangle)]
#[inline(never)]
fn metallic_perf_lua_method_calls(host: &mut ScriptHost) {
    host.step(black_box(4.0), black_box(STEP_DT));
}

fn number(world: &World, part: InstanceId, property: &str) -> Result<f64, String> {
    match world.get(part, property) {
        Ok(Value::Number(value)) => Ok(value),
        other => Err(format!("{property} is not a number: {other:?}")),
    }
}

/// What the script must print once the measured pass count has run, computed from the World.
fn expected_result(workload: &str, world: &World, parts: &[InstanceId]) -> Result<f64, String> {
    match workload {
        "lua_reads" => {
            let mut one_pass = 0.0;
            for &part in parts {
                one_pass += number(world, part, "Transparency")? + 1.0 + "Part".len() as f64;
            }
            Ok(one_pass * PASSES as f64)
        }
        "lua_writes" => {
            // The warm-up pass and the measured passes; the last value must have reached the World.
            let written = PASSES + 1;
            let can_collide = Value::Bool(written % 2 == 1);
            for &part in parts {
                if number(world, part, "Transparency")? != written as f64 / 16.0
                    || world.get(part, "CanCollide").as_ref().ok() != Some(&can_collide)
                {
                    return Err("write workload did not change every Part".to_string());
                }
            }
            Ok(written as f64)
        }
        "lua_method_calls" => Ok((3 * PARTS * PASSES) as f64),
        _ => Err(USAGE.to_string()),
    }
}

/// Runs the rest of the script and checks it finished. Keep this outside Callgrind collection and
/// elapsed_ns: the collector must not accept a successful process that skipped the work.
fn validate(workload: &str, fixture: &mut Fixture) -> Result<(), String> {
    fixture.host.step(REPORT_STEP_AT, STEP_DT);
    let problems = fixture.output.problems.borrow();
    if let Some(problem) = problems.first() {
        return Err(format!("script reported: {problem}"));
    }
    let printed = fixture.output.printed.borrow();
    let [line] = printed.as_slice() else {
        return Err(format!("expected one result line, got {printed:?}"));
    };
    let result: f64 = line
        .parse()
        .map_err(|_| format!("result is not a number: {line}"))?;
    let expected = expected_result(workload, &fixture.world.borrow(), &fixture.parts)?;
    if result != expected {
        return Err(format!("expected result {expected}, script gave {result}"));
    }
    Ok(())
}

fn body_of(workload: &str) -> Option<&'static str> {
    match workload {
        "lua_reads" => Some(READS),
        "lua_writes" => Some(WRITES),
        "lua_method_calls" => Some(METHOD_CALLS),
        _ => None,
    }
}

fn measure(workload: &str, host: &mut ScriptHost) {
    match workload {
        "lua_reads" => metallic_perf_lua_reads(host),
        "lua_writes" => metallic_perf_lua_writes(host),
        _ => metallic_perf_lua_method_calls(host),
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let workload = std::env::args().nth(1).ok_or(USAGE)?;
    let body = body_of(&workload).ok_or(USAGE)?;
    let mut fixture = fixture(body)?;
    let start = Instant::now();
    measure(&workload, &mut fixture.host);
    let elapsed = start.elapsed().as_nanos();
    validate(&workload, &mut fixture)?;
    println!("{{\"workload\":\"{workload}\",\"elapsed_ns\":{elapsed},\"completed\":true}}");
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("performance workload failed: {error}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKLOADS: [&str; 3] = ["lua_reads", "lua_writes", "lua_method_calls"];

    #[test]
    fn collection_functions_stay_separate_symbols() {
        // Meaningful with `--release`: merged functions would share one address.
        let reads = metallic_perf_lua_reads as *const () as usize;
        let writes = metallic_perf_lua_writes as *const () as usize;
        let method_calls = metallic_perf_lua_method_calls as *const () as usize;
        assert!(reads != writes && writes != method_calls && reads != method_calls);
    }

    #[test]
    fn every_workload_completes_its_measured_step_and_validates() {
        for workload in WORKLOADS {
            let mut fixture = fixture(body_of(workload).unwrap()).unwrap();
            measure(workload, &mut fixture.host);
            validate(workload, &mut fixture).unwrap();
        }
    }

    #[test]
    fn the_measured_work_runs_inside_the_measured_step_and_nowhere_else() {
        // Writes show it directly: only the warm-up has reached the World before the step, and
        // every measured pass has reached it afterwards.
        let mut fixture = fixture(WRITES).unwrap();
        let part = fixture.parts[0];
        let transparency = |fixture: &Fixture| fixture.world.borrow().get(part, "Transparency");
        assert_eq!(transparency(&fixture).unwrap(), Value::Number(1.0 / 16.0));
        metallic_perf_lua_writes(&mut fixture.host);
        assert_eq!(
            transparency(&fixture).unwrap(),
            Value::Number((PASSES + 1) as f64 / 16.0)
        );
    }

    #[test]
    fn a_skipped_measured_step_fails_validation() {
        // The report step alone wakes the first wait and runs the work, but never gets to print.
        for workload in WORKLOADS {
            let mut fixture = fixture(body_of(workload).unwrap()).unwrap();
            assert!(validate(workload, &mut fixture).is_err(), "{workload}");
        }
    }

    #[test]
    fn a_failing_script_fails_validation() {
        let mut fixture = fixture("local function run() error('workload broke') end").unwrap();
        measure("lua_reads", &mut fixture.host);
        let error = validate("lua_reads", &mut fixture).unwrap_err();
        assert!(error.contains("workload broke"), "{error}");
    }

    #[test]
    fn a_wrong_result_fails_validation() {
        let mut fixture = fixture("local function run() return 1 end").unwrap();
        measure("lua_reads", &mut fixture.host);
        let error = validate("lua_reads", &mut fixture).unwrap_err();
        assert!(error.contains("expected result"), "{error}");
    }
}
