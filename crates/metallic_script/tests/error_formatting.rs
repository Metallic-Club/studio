//! Structured native errors and protected-call compatibility.

mod common;

use common::Rig;
use metallic_script::{Context, OutputSink, ScriptVm};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

fn run(source: &str) {
    ScriptVm::new(Context::Server)
        .unwrap()
        .run_script("Case", source)
        .unwrap();
}

#[test]
fn native_errors_have_exact_positioned_text_without_internal_tracebacks() {
    for (source, message) in [
        (
            "collectgarbage('step')",
            "collectgarbage: only \"count\" is allowed",
        ),
        ("string.match('x', '[')", "malformed pattern (missing ']')"),
    ] {
        let mut vm = ScriptVm::new(Context::Server).unwrap();
        assert_eq!(
            vm.run_script("Case", source).unwrap_err().message,
            format!("Case:1: {message}")
        );
    }
}

#[test]
fn native_pcall_errors_are_strings_and_keep_argument_diagnostics() {
    run(r#"
local ok, err = pcall(string.match, {}, 'x')
assert(not ok and type(err) == 'string', 'native error type: '..type(err))
assert(string.find(err, 'bad argument #1', 1, true), err)
assert(not string.find(err, '\nstack traceback', 1, true), err)
ok, err = pcall(collectgarbage, 'step')
assert(not ok and type(err) == 'string', 'collector error type: '..type(err))
ok, err = pcall(string.match, 'x', '[')
assert(not ok and type(err) == 'string', 'pattern error type: '..type(err))
"#);
}

#[test]
fn native_error_uses_the_failing_inner_line() {
    run(r#"local function fail()
 collectgarbage('step')
end
local ok, err = pcall(fail)
assert(not ok and err == 'Case:2: collectgarbage: only "count" is allowed', tostring(err))
"#);
}

#[test]
fn protected_user_errors_keep_values_and_literal_multiline_messages() {
    run(r#"
local text = 'runtime error: literal\nstack traceback:\nkeep this\n'
local ok, err = pcall(error, text, 0)
assert(not ok and err == text)
local object = setmetatable({}, {__tostring=function() error('must not stringify') end})
ok, err = pcall(error, object, 0)
assert(not ok and rawequal(err, object))
ok, err = pcall(error, nil, 0)
assert(not ok and err == nil)
ok, err = pcall(error, false, 0)
assert(not ok and err == false)
ok, err = pcall(error, 17, 0)
assert(not ok and err == 17)
"#);
}

#[test]
fn pcall_preserves_callable_tables_metamethod_errors_and_nil_results() {
    run(r#"
local function values(...) return select('#', ...), ... end
local count, ok, a, b, c = values(pcall(function() return 12, nil, 'last', nil end))
assert(count == 5 and ok and a == 12 and b == nil and c == 'last')
local callable = setmetatable({}, {__call=function(_, x, y) return x + y, nil end})
count, ok, a, b = values(pcall(callable, 3, 4))
assert(count == 3 and ok and a == 7 and b == nil)
local object = {}
callable = setmetatable({}, {__call=function() error(object, 0) end})
ok, a = pcall(callable)
assert(not ok and rawequal(a, object))
ok, a = pcall(nil)
assert(not ok and type(a) == 'string')
ok, a = pcall(pcall)
assert(not ok and type(a) == 'string')
"#);
}

#[test]
fn xpcall_user_handler_return_and_failure_semantics_are_preserved() {
    run(r#"
local object, seen = {}, nil
local ok, result = xpcall(function() error(object, 0) end, function(e) seen=e return 'handled' end)
assert(not ok and rawequal(seen, object) and result == 'handled')
ok, result = xpcall(function() error('original') end, function() error('handler failed') end)
assert(not ok and type(result) == 'string')
assert(string.find(result, 'error in error handling', 1, true), result)
"#);
}

#[test]
fn api_member_error_keeps_traceback_looking_property_name() {
    let mut rig = Rig::new();
    rig.server_script("Literal", r#"local p = Instance.new('Part')
local ok, err = pcall(function() return p['missing\nstack traceback:\nkept'] end)
assert(not ok and err == 'ServerScriptService.Literal:2: missing\nstack traceback:\nkept is not a valid member of Part', tostring(err))
"#);
    rig.host.start().unwrap();
    assert!(rig.lines().is_empty(), "{:?}", rig.lines());
}

#[test]
fn pcall_keeps_user_instance_error_identity() {
    let mut rig = Rig::new();
    rig.server_script(
        "Identity",
        "local p=Instance.new('Part') local ok,e=pcall(error,p,0) assert(not ok and e==p)",
    );
    rig.host.start().unwrap();
    assert!(rig.lines().is_empty(), "{:?}", rig.lines());
}

const LITERAL: &str = "runtime error: user prefix\nstack traceback:\nkeep user text\n";

#[derive(Default)]
struct Capture(RefCell<Vec<String>>);
impl OutputSink for Capture {
    fn print(&self, line: &str) {
        self.0.borrow_mut().push(format!("print:{line}"));
    }
    fn warn(&self, line: &str) {
        self.0.borrow_mut().push(format!("warn:{line}"));
    }
    fn error(&self, line: &str) {
        self.0.borrow_mut().push(format!("error:{line}"));
    }
}

fn native_vm() -> (ScriptVm, Rc<Capture>) {
    let sink = Rc::new(Capture::default());
    let vm = ScriptVm::with_output(Context::Server, sink.clone()).unwrap();
    let native = vm
        .lua()
        .create_function(|_, ()| Err::<(), _>(mlua::Error::runtime(LITERAL)))
        .unwrap();
    vm.set_global("native", native).unwrap();
    (vm, sink)
}

#[test]
fn native_callback_keeps_multiline_message_in_direct_and_delayed_errors() {
    let (mut vm, sink) = native_vm();
    assert_eq!(
        vm.run_script("Case", "native()").unwrap_err().message,
        format!("Case:1: {LITERAL}")
    );
    vm.run_script("Later", "task.delay(0, function()\n native()\nend)")
        .unwrap();
    vm.step(0.0);
    assert_eq!(*sink.0.borrow(), [format!("error:Later:2: {LITERAL}")]);
}

#[test]
fn pcall_is_yieldable_and_keeps_all_resumed_results() {
    let (mut vm, sink) = native_vm();
    vm.run_script("Yield", "task.spawn(function() local ok, elapsed = pcall(task.wait, 0) assert(ok and type(elapsed)=='number') print('resumed') end)").unwrap();
    vm.step(0.0);
    assert_eq!(*sink.0.borrow(), ["print:resumed"]);
}

#[test]
fn returned_gmatch_iterator_errors_are_guarded() {
    run(r#"local iterator = string.gmatch('x', '[')
local ok, err = pcall(iterator)
assert(not ok and type(err)=='string', 'iterator error type: '..type(err))
assert(err == "Case:2: malformed pattern (missing ']')", err)
"#);
}

#[test]
fn returned_gmatch_iterator_survives_gc_without_retaining_the_vm() {
    let vm = ScriptVm::new(Context::Server).unwrap();
    let iterator: mlua::Function = vm
        .lua()
        .load("return string.gmatch('ab', '.')")
        .eval()
        .unwrap();
    vm.lua().gc_collect().unwrap();
    assert_eq!(vm.call_with_budget::<String>(&iterator, ()).unwrap(), "a");
    vm.lua().gc_collect().unwrap();
    assert_eq!(vm.call_with_budget::<String>(&iterator, ()).unwrap(), "b");
    assert_eq!(
        vm.call_with_budget::<Option<String>>(&iterator, ())
            .unwrap(),
        None
    );
    let budget = vm.budget();
    drop(vm);
    assert!(
        budget
            .call::<mlua::Value>(&iterator, ())
            .unwrap_err()
            .message
            .contains("no longer exists")
    );
}

#[test]
fn raw_embedding_callback_pcall_keeps_mlua_error_objects() {
    // Raw host callbacks are deliberately outside the trusted engine wrappers.
    // They retain mlua's structured error-object semantics under native pcall.
    let (mut vm, _) = native_vm();
    vm.run_script(
        "Embedding",
        "local ok,err=pcall(native) assert(not ok and type(err)=='userdata')",
    )
    .unwrap();
}

#[test]
fn caught_timeout_stays_fatal_and_next_resume_recovers() {
    let mut vm = ScriptVm::new(Context::Server).unwrap();
    let error = vm
        .run_script(
            "Timed",
            "pcall(function() while true do end end) return 'caught'",
        )
        .unwrap_err();
    assert!(
        error.message.contains("Script timeout"),
        "{}",
        error.message
    );
    vm.run_script("Healthy", "assert(1 + 1 == 2)").unwrap();
}

#[test]
fn uncaught_malformed_pattern_has_only_the_positioned_message() {
    let mut vm = ScriptVm::new(Context::Server).unwrap();
    let error = vm.run_script("Case", "string.match('x','%')").unwrap_err();
    assert_eq!(error.message, "Case:1: malformed pattern (ends with '%')");
}

#[test]
fn one_thousand_successful_pcalls_fit_the_unchanged_budget() {
    let mut vm = ScriptVm::new(Context::Server).unwrap();
    let start = Instant::now();
    vm.run_script("Smoke", "local function f(x) return x,nil end for i=1,1000 do local ok,v,n=pcall(f,i) assert(ok and v==i and n==nil) end").unwrap();
    eprintln!(
        "1000 pcall smoke (uncontrolled host load): {:?}",
        start.elapsed()
    );
}
