use metallic_script::{Context, ScriptVm};
use mlua::Function;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[test]
fn slow_host_return_times_out_with_a_script_location_and_the_vm_recovers() {
    let mut vm = ScriptVm::new(Context::Server).unwrap();
    let pause = vm
        .lua()
        .create_function(|_, ()| {
            std::thread::sleep(Duration::from_millis(140));
            Ok(())
        })
        .unwrap();
    vm.set_global("host_pause", pause).unwrap();
    let error = vm
        .run_script("Workspace.Slow", "host_pause() return 7")
        .unwrap_err();
    assert!(
        error.message.contains("Workspace.Slow:1:"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("Script timeout"),
        "{}",
        error.message
    );
    vm.run_script("Workspace.Next", "assert(2 + 2 == 4)")
        .unwrap();
}

#[test]
fn expensive_native_calls_do_not_gain_a_thousand_call_polling_window() {
    let mut vm = ScriptVm::new(Context::Server).unwrap();
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let native = vm
        .lua()
        .create_function(move |_, ()| {
            count.set(count.get() + 1);
            std::thread::sleep(Duration::from_millis(20));
            Ok(())
        })
        .unwrap();
    vm.set_global("native_work", native).unwrap();
    let started = Instant::now();
    let error = vm
        .run_script(
            "Workspace.NativeLoop",
            "for i = 1, 100 do native_work() end",
        )
        .unwrap_err();
    assert!(
        error.message.contains("Script timeout"),
        "{}",
        error.message
    );
    assert!(
        calls.get() <= 20,
        "executed {} expensive native calls after a 100 ms budget",
        calls.get()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn nested_call_uses_the_outer_budget_remainder_after_host_work() {
    let mut vm = ScriptVm::new(Context::Server).unwrap();
    let budget = vm.budget();
    let nested = vm
        .lua()
        .create_function(move |_, function: Function| {
            std::thread::sleep(Duration::from_millis(75));
            budget
                .call::<()>(&function, ())
                .map_err(mlua::Error::external)
        })
        .unwrap();
    vm.set_global("host_nested", nested).unwrap();
    let error = vm
        .run_script(
            "Workspace.Nested",
            r#"
        host_nested(function()
            local started = os.clock()
            while os.clock() - started < 0.05 do end
        end)
    "#,
        )
        .unwrap_err();
    assert!(
        error.message.contains("Script timeout"),
        "{}",
        error.message
    );
    vm.run_script("Workspace.Next", "assert(true)").unwrap();
}

#[test]
fn repeated_timeout_and_recovery_do_not_inherit_an_old_expiration() {
    let mut vm = ScriptVm::new(Context::Server).unwrap();
    for _ in 0..3 {
        let error = vm
            .run_script(
                "Workspace.Catch",
                r#"
            while true do pcall(function() while true do end end) end
        "#,
            )
            .unwrap_err();
        assert!(
            error.message.contains("Script timeout"),
            "{}",
            error.message
        );
        vm.run_script("Workspace.Next", "assert(6 * 7 == 42)")
            .unwrap();
    }
}

#[test]
fn dropping_vm_closes_budget_handles_even_if_a_trusted_raw_lua_handle_remains() {
    let vm = ScriptVm::new(Context::Server).unwrap();
    let raw = vm.lua().clone();
    let function: Function = raw
        .load("return function() while true do end end")
        .eval()
        .unwrap();
    let thread = raw.create_thread(function).unwrap();
    let budget = vm.budget();
    drop(vm);
    let error = budget.resume(&thread, ()).unwrap_err();
    assert!(error.message.contains("shut down"), "{}", error.message);
    drop(raw);
}
