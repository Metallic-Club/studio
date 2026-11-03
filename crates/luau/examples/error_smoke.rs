//! Runs scripts that raise errors through every path the Luau backend has, and checks each is reported and the
//! process survives. It exists for the shipped `dist` profile, whose panic strategy the tests (default profile)
//! cannot see; CI runs it there (`crates/luau/src/ffi.rs` explains why the strategy matters):
//!
//! ```sh
//! cargo run -p metallic_luau --example error_smoke --profile dist
//! cargo run -p metallic_luau --example error_smoke --profile dist -- Timeout   # one case
//! ```
//!
//! Exits with 0 when every script failed the expected way, 1 otherwise. An abort means a Luau error unwound
//! through Rust in a way the build cannot handle.
#![allow(missing_docs, reason = "an example binary")]

use std::process::ExitCode;
use std::time::Duration;

use metallic_luau::LuauLanguage;
use metallic_script::{OutputLevel, Phase, ScriptHost};

/// `(script name, source, text the reported error must contain)`.
const CASES: &[(&str, &str, &str)] = &[
    // A C function's bad argument (raised by `ffi::raise`).
    ("BadArgument", "local v = Vector3.new('x')", "bad argument #1 to 'new'"),
    ("BadCFrame", "local cf = CFrame.new(1, 2)", "Vector3 expected"),
    (
        "CFrameArity",
        "local cf = CFrame.new(1, 2, 3, 4)",
        "invalid number of arguments",
    ),
    // `Instance` members (raised by `ffi::raise` from `__index`, `__newindex`, `__namecall`).
    (
        "UnknownMember",
        "print(workspace.Nope)",
        "Nope is not a valid member of Workspace",
    ),
    (
        "ReadOnly",
        "Instance.new('Part').ClassName = 'x'",
        "Property is read only",
    ),
    (
        "BadValue",
        "Instance.new('Part').Transparency = 'x'",
        "Unable to assign property Transparency",
    ),
    ("MethodError", "workspace:IsA('NoSuchClass')", "NoSuchClass"),
    (
        "Stale",
        "local p = Instance.new('Part')\np:Destroy()\nprint(p.Name)",
        "destroyed",
    ),
    // An mlua callback's error (raised by mlua).
    ("Callback", "task.cancel(coroutine.running())", "cannot cancel"),
    // A script's own error, in a listener thread.
    (
        "Handler",
        "local e = Instance.new('BindableEvent')\ne.Event:Connect(function() error('in handler') end)\ne:Fire()",
        "in handler",
    ),
    // pcall catches a raised error, and the script goes on to fail later.
    (
        "Caught",
        "local ok = pcall(function() return workspace.Nope end)\nassert(not ok)\nerror('after pcall')",
        "after pcall",
    ),
    // A C function that yields to the host where Luau cannot yield (raised by `lua_yield` in `ffi::run`).
    (
        "YieldBoundary",
        "local f = Instance.new('BindableFunction')\nlocal t = setmetatable({}, { __index = function() return \
         f:Invoke() end })\nprint(t.x)",
        "attempt to yield across",
    ),
    // Errors the host raises where a thread waits (`Thread::resume_error`): a failed callback, a destroyed instance.
    (
        "InvokeError",
        "local f = Instance.new('BindableFunction')\nf.OnInvoke = function() error('in callback') end\nf:Invoke()",
        "in callback",
    ),
    (
        "WaitDestroyed",
        "local e = Instance.new('BindableEvent')\ntask.defer(function() e:Destroy() end)\ne.Event:Wait()",
        "was destroyed",
    ),
    // Assigning a callback member something else than a function (raised from `__newindex`).
    (
        "CallbackValue",
        "Instance.new('BindableFunction').OnInvoke = 5",
        "Expected a function or nil",
    ),
    ("RequireValue", "require('Scripts/Door.luau')", "invalid argument"),
    // The interrupt (raised from Luau's interrupt callback).
    ("Timeout", "while true do end", "time budget"),
    // Out of memory (raised by Luau's allocator under the limit).
    (
        "Memory",
        "local t = {}\nfor i = 1, 1e8 do t[i] = tostring(i) .. 'padding padding' end",
        "memory",
    ),
    // Out of memory inside an engine C function (allocating userdata while a Rust frame is on the stack).
    (
        "EngineMemory",
        "local t = {}\nfor i = 1, 1e8 do t[i] = CFrame.new(i, 0, 0) end",
        "memory",
    ),
];

fn main() -> ExitCode {
    let mut dm = match metallic_classes::new_game() {
        Ok(dm) => dm,
        Err(error) => {
            eprintln!("could not start a game: {error}");
            return ExitCode::FAILURE;
        }
    };
    let luau = match LuauLanguage::new(dm.registry().clone()) {
        Ok(luau) => luau,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let mut host = ScriptHost::new();
    if let Err(error) = host.register_language(Box::new(luau)) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    host.set_resume_budget(Duration::from_millis(50));
    let registry = dm.registry().clone();
    let (Some(script_class), Some(workspace)) = (
        registry.class_id("Script"),
        registry.class_id("Workspace").and_then(|class| dm.service(class)),
    ) else {
        eprintln!("the class set has no Script or Workspace");
        return ExitCode::FAILURE;
    };

    // `error_smoke <Name>` runs one case, to tell which path fails.
    let only = std::env::args().nth(1);
    let mut failed = 0;
    for (name, source, expected) in CASES
        .iter()
        .filter(|(name, ..)| only.as_deref().is_none_or(|o| o == *name))
    {
        let before = host.output().of_level(OutputLevel::Error).count();
        if name.ends_with("Memory") {
            host.set_memory_limit(Some(host.memory_used() + 4 * 1024 * 1024));
        }
        let Ok(script) = dm.create(script_class) else {
            eprintln!("{name}: could not create the script");
            return ExitCode::FAILURE;
        };
        let _ = dm.set_name(script, (*name).into());
        let _ = dm.set_parent(script, Some(workspace));
        let _ = host.run_script(&mut dm, script, &format!("Scripts/{name}.luau"), source);
        for phase in Phase::FRAME {
            host.step(&mut dm, phase, 0.0);
        }
        host.set_memory_limit(None);
        let errors: Vec<String> = host
            .output()
            .of_level(OutputLevel::Error)
            .skip(before)
            .map(|line| line.message.lines().next().unwrap_or_default().to_owned())
            .collect();
        let ok = errors.len() == 1 && errors[0].to_lowercase().contains(&expected.to_lowercase());
        println!("{} {name}: {errors:?}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            failed += 1;
        }
    }

    // The VM still runs scripts after all of that.
    let before = host.output().of_level(OutputLevel::Print).count();
    if let Ok(script) = dm.create(script_class) {
        let _ = dm.set_parent(script, Some(workspace));
        let _ = host.run_script(&mut dm, script, "Scripts/After.luau", "print('still running')");
    }
    let alive = host
        .output()
        .of_level(OutputLevel::Print)
        .skip(before)
        .any(|l| l.message == "still running");
    println!("{} the VM keeps running", if alive { "ok  " } else { "FAIL" });
    if !alive {
        failed += 1;
    }

    if failed == 0 {
        println!("all {} error paths reported; no abort", CASES.len());
        ExitCode::SUCCESS
    } else {
        println!("{failed} check(s) failed");
        ExitCode::FAILURE
    }
}
