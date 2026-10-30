//! Plain/literal byte search must remain bounded even for adversarial repeated prefixes.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use metallic_script::{Context, ScriptVm};

fn bounded_repro(mode: &str) {
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args([
            "--exact",
            "plain_find_adversarial_worker",
            "--ignored",
            "--nocapture",
        ])
        .env("METALLIC_PLAIN_FIND_CASE", mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().expect("isolated search process");
    let started = Instant::now();
    loop {
        if child.try_wait().expect("child status").is_some() {
            let output = child.wait_with_output().expect("child output");
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            print!("{}", String::from_utf8_lossy(&output.stdout));
            break;
        }
        if started.elapsed() > Duration::from_secs(3) {
            child.kill().expect("kill unresponsive search");
            child.wait().expect("reap unresponsive search");
            panic!("{mode} search exceeded the 3 s outer process safety limit");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn explicit_plain_find_cannot_hold_the_host() {
    bounded_repro("plain");
}

#[test]
fn implicit_literal_find_cannot_hold_the_host() {
    bounded_repro("literal");
}

#[test]
#[ignore = "run by the outer tests in a separately killable process"]
fn plain_find_adversarial_worker() {
    let mode = std::env::var("METALLIC_PLAIN_FIND_CASE").expect("isolated worker mode");
    let mut vm = ScriptVm::new(Context::Server).expect("VM");
    let plain_argument = if mode == "plain" { ", 1, true" } else { "" };
    let source = format!(
        "assert(string.find(string.rep('a', 1e7), string.rep('a', 1e5)..'b'{plain_argument}) == nil)"
    );
    let started = Instant::now();
    let result = vm.run_script("Workspace.PlainFind", &source);
    let elapsed = started.elapsed();
    println!("{mode} adversarial search returned in {elapsed:?}");
    assert!(elapsed < Duration::from_millis(500), "{mode}: {elapsed:?}");
    if let Err(error) = result {
        assert!(
            error.message.starts_with("Workspace.PlainFind:1:"),
            "{}",
            error.message
        );
        assert!(
            error
                .message
                .contains("Script timeout: exhausted allowed execution time"),
            "{}",
            error.message
        );
    }
    vm.run_script(
        "Workspace.AfterPlainFind",
        "assert(string.find('alive', 'live') == 2)",
    )
    .expect("another script still runs after the search");
}

#[test]
fn plain_find_preserves_lua_byte_indexes_and_relative_starts() {
    let mut vm = ScriptVm::new(Context::Server).expect("VM");
    vm.run_script(
        "Workspace.FindSemantics",
        r#"
        local function check(s, needle, init, plain, first, last)
            local a, b = string.find(s, needle, init, plain)
            assert(a == first and b == last)
        end
        for _, plain in {false, true} do
            check('abcabc', 'bc', 1, plain, 2, 3)
            check('abcabc', 'bc', 3, plain, 5, 6)
            check('abcabc', 'bc', -3, plain, 5, 6)
            check('abcabc', 'bc', -100, plain, 2, 3)
            check('abcabc', 'bc', 0, plain, 2, 3)
            check('abcabc', 'bc', 8, plain, nil, nil)
            check('abc', '', 1, plain, 1, 0)
            check('abc', '', 4, plain, 4, 3)
            check('abc', '', 5, plain, nil, nil)
            check('', '', 1, plain, 1, 0)
            check('abc', 'abcdef', 1, plain, nil, nil)
            check('a\0b\0c', '\0b\0', 1, plain, 2, 4)
            check(string.char(255, 0, 254, 0, 255), string.char(254, 0), 1, plain, 3, 4)
            check('éé', 'é', -2, plain, 3, 4)
        end
        check('a.b[c]%', '.b[', 1, true, 2, 4)
        check('abc', '.', 1, true, nil, nil)
        check('abc', '.', 1, false, 1, 1)
        check('a.b', '.', 1, 0, 2, 2) -- Lua treats zero as truthy.
    "#,
    )
    .expect("Lua byte-search semantics");
}

#[test]
fn plain_find_observes_an_enclosing_deadline_after_a_slow_host_callback() {
    let mut vm = ScriptVm::new(Context::Server).expect("VM");
    let string: mlua::Table = vm.lua().globals().get("string").expect("string library");
    let find: mlua::Function = string.get("find").expect("string.find");
    let slow = vm
        .lua()
        .create_function(move |_, ()| {
            std::thread::sleep(Duration::from_millis(125));
            // Direct native-to-native call: no Luau bytecode interrupt runs between
            // the host callback consuming the budget and string.find checking it.
            find.call::<mlua::MultiValue>(("abc", "b", 1, true))
        })
        .expect("slow callback");
    vm.set_global("slow_find", slow)
        .expect("install host callback");
    let started = Instant::now();
    let error = vm
        .run_script("Workspace.DelayedFind", "slow_find()")
        .expect_err("expired enclosing budget must reject a successful search");
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(
        error.message.starts_with("Workspace.DelayedFind:1:"),
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .contains("Script timeout: exhausted allowed execution time"),
        "{}",
        error.message
    );
    vm.run_script(
        "Workspace.AfterTimeout",
        "assert(string.find('alive', 'live') == 2)",
    )
    .expect("timeout is isolated to the failed script");
}
