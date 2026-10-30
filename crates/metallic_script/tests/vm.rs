//! Behaviour tests for the sandboxed Luau VM.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use metallic_script::{Context, OutputSink, ScriptVm};

#[derive(Default)]
struct Capture {
    lines: RefCell<Vec<String>>,
}

impl OutputSink for Capture {
    fn print(&self, line: &str) {
        self.lines.borrow_mut().push(format!("print:{line}"));
    }
    fn warn(&self, line: &str) {
        self.lines.borrow_mut().push(format!("warn:{line}"));
    }
    fn error(&self, line: &str) {
        self.lines.borrow_mut().push(format!("error:{line}"));
    }
}

fn vm() -> (ScriptVm, Rc<Capture>) {
    let sink = Rc::new(Capture::default());
    (ScriptVm::with_output(Context::Server, sink.clone()), sink)
}

#[test]
fn runs_basic_code() {
    let (mut vm, sink) = vm();
    vm.run_script("Workspace.Ok", "print('hi', 1 + 2)").unwrap();
    assert_eq!(*sink.lines.borrow(), vec!["print:hi 3".to_string()]);
}

#[test]
fn forbidden_globals_are_nil() {
    let (mut vm, _) = vm();
    for name in [
        "io",
        "debug",
        "loadstring",
        "load",
        "getfenv",
        "setfenv",
        "require",
        "dofile",
    ] {
        vm.run_script("T", &format!("assert({name} == nil, '{name}')"))
            .unwrap();
    }
    vm.run_script(
        "T",
        "assert(os.execute == nil and os.getenv == nil and os.remove == nil and os.date == nil)",
    )
    .unwrap();
    vm.run_script(
        "T",
        "assert(type(os.clock()) == 'number' and type(os.time()) == 'number')",
    )
    .unwrap();
    vm.run_script("T", "assert(type(collectgarbage('count')) == 'number')")
        .unwrap();
    assert!(vm.run_script("T", "collectgarbage('collect')").is_err());
}

#[test]
fn endless_loop_times_out() {
    let (mut vm, sink) = vm();
    let start = Instant::now();
    let err = vm
        .run_script("Workspace.Loop", "while true do end")
        .unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(
        err.message
            .contains("Script timeout: exhausted allowed execution time"),
        "{}",
        err.message
    );
    vm.run_script("Workspace.Next", "print('still alive')")
        .unwrap();
    assert_eq!(sink.lines.borrow().last().unwrap(), "print:still alive");
}

#[test]
fn memory_cap_stops_script() {
    let (mut vm, _) = vm();
    let err = vm
        .run_script(
            "Workspace.Hog",
            "local t = {} for i = 20, 40 do t[i] = table.create(2 ^ i) end",
        )
        .unwrap_err();
    assert!(err.message.contains("not enough memory"), "{}", err.message);
    vm.run_script("Workspace.After", "local x = 1 + 1").unwrap();
}

#[test]
fn error_message_has_chunk_and_line() {
    let (mut vm, _) = vm();
    let err = vm
        .run_script("Workspace.Bad.Script", "local x = nil + 1")
        .unwrap_err();
    assert!(
        err.message.starts_with("Workspace.Bad.Script:1:"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains("attempt to perform arithmetic"),
        "{}",
        err.message
    );
    let err = vm
        .run_script("Workspace.Bad.Script", "syntax error here")
        .unwrap_err();
    assert!(
        err.message.starts_with("Workspace.Bad.Script:1:"),
        "{}",
        err.message
    );
}

#[test]
fn task_wait_resumes_after_step() {
    let (mut vm, sink) = vm();
    vm.run_script(
        "T",
        "print('a') local e = task.wait(1) print('b', e >= 1) task.spawn(function(x) print('spawn', x) end, 7)
         task.delay(2, function(x) print('delayed', x) end, 'z')",
    )
    .unwrap();
    assert_eq!(sink.lines.borrow().len(), 1);
    vm.step(0.5);
    assert_eq!(sink.lines.borrow().len(), 1);
    vm.step(1.0);
    assert_eq!(
        *sink.lines.borrow(),
        vec!["print:a", "print:b true", "print:spawn 7"]
    );
    vm.step(3.0);
    assert_eq!(sink.lines.borrow().last().unwrap(), "print:delayed z");
}

#[test]
fn shutdown_drops_pending_delays() {
    let (mut vm, sink) = vm();
    vm.run_script(
        "T",
        "task.delay(1, function() print('late') end) task.wait(1) print('late2')",
    )
    .unwrap();
    vm.shutdown();
    assert!(sink.lines.borrow().is_empty());
}

#[test]
fn thread_error_goes_to_sink() {
    let (mut vm, sink) = vm();
    vm.run_script("Workspace.T", "task.delay(1, function() error('boom') end)")
        .unwrap();
    vm.step(2.0);
    let lines = sink.lines.borrow();
    assert!(
        lines[0].starts_with("error:Workspace.T:1: boom"),
        "{lines:?}"
    );
}
