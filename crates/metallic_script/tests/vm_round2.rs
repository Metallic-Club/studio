//! Round-2 hardening tests: bounded pattern matching, weak budget handle, error reporting.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use metallic_script::{Context, OutputSink, ScriptVm};
use mlua::Function;

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
    (
        ScriptVm::with_output(Context::Server, sink.clone()).unwrap(),
        sink,
    )
}

#[test]
fn single_quantifier_pathological_find_times_out_or_errors_fast() {
    let (mut vm, _) = vm();
    let start = Instant::now();
    for src in [
        "string.find(string.rep('a', 1e6), '.-b')",
        "string.match(string.rep('a', 1e6), '.*b')",
        "string.gsub(string.rep('a', 1e6), '.-b', 'x')",
        "for _ in string.gmatch(string.rep('a', 1e6), '.-b') do end",
        "string.find(string.rep('a', 100000), 'a-a-a-a-a-b')",
    ] {
        let err = vm.run_script("T", src).unwrap_err();
        assert!(err.message.starts_with("T:1:"), "{src}: {}", err.message);
        assert!(
            err.message.contains("pattern too complex") || err.message.contains("Script timeout"),
            "{src}: {}",
            err.message
        );
    }
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    vm.run_script("T", "print('alive')").unwrap();
}

#[test]
fn trim_pattern_on_100kb_works() {
    let (mut vm, _) = vm();
    let start = Instant::now();
    vm.run_script(
        "T",
        "local s = '  ' .. string.rep('x', 100000) .. '  '
         assert(#string.match(s, '^%s*(.-)%s*$') == 100000)
         assert(#string.gsub(s, '^%s+', '') == 100002)
         assert(string.find(s, 'x+') == 3)",
    )
    .unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn nan_init_is_treated_as_start() {
    let (mut vm, _) = vm();
    vm.run_script(
        "T",
        "assert(string.find('abc', 'b', 0/0) == 2)
         assert(string.match('abc', '.', 0/0) == 'a')",
    )
    .unwrap();
}

const FMT: &str = "local function fmt(...) local o = {} for i = 1, select('#', ...) do o[#o + 1] = tostring((select(i, ...))) end return table.concat(o, '|') end
local function it(f, s, p) local r = {} for a, b in f(s, p) do r[#r + 1] = tostring(a) .. ',' .. tostring(b) end return table.concat(r, ';') end
";

#[test]
fn pattern_results_match_luau_for_common_cases() {
    let cases = [
        "fmt(string.find('hello world', 'wor'))",
        "fmt(string.find('hello', 'l+'))",
        "fmt(string.find('hello', '(l)(l)'))",
        "fmt(string.find('hello', '()ll()'))",
        "fmt(string.find('hello', 'xyz'))",
        "fmt(string.find('hello', '^h'))",
        "fmt(string.find('hello', '^e'))",
        "fmt(string.find('hello', 'o$'))",
        "fmt(string.find('a.b', '.', 1, true))",
        "fmt(string.find('abc', 'b', -1))",
        "fmt(string.find('abc', '', 10))",
        "fmt(string.find('abc', '', 4))",
        "fmt(string.match('key=value', '(%w+)=(%w+)'))",
        "fmt(string.match('  trim  ', '^%s*(.-)%s*$'))",
        "fmt(string.match('THE (quick) fox', '%((%a+)%)'))",
        "fmt(string.match('x(a(b)c)y', '%b()'))",
        "fmt(string.match('THE (quick) fox', '%f[%a]%a+', 5))",
        "fmt(string.match('hello', '.-'))",
        "fmt(string.match('2024-10-30', '(%d+)-(%d+)-(%d+)'))",
        "fmt(string.match('abc', 'a?b?c?d?'))",
        "fmt(string.match('aaa', '(a)%1'))",
        "fmt(string.match('abab', '(ab)%1'))",
        "fmt(string.match('[test]', '%[(.-)%]'))",
        "fmt(string.match('foo.bar', '[^.]+'))",
        "fmt(string.match('x', '[a-c]'))",
        "fmt(string.match('a-b', '[%a-]+'))",
        "fmt(string.match('hello123', '%a+()'))",
        "fmt(string.gsub('hello world', 'o', '0'))",
        "fmt(string.gsub('hello', 'l', 'L', 1))",
        "fmt(string.gsub('abc', '%w', '%0%0'))",
        "fmt(string.gsub('hello world', '(%w+) (%w+)', '%2 %1'))",
        "fmt(string.gsub('abc', '', '-'))",
        "fmt(string.gsub('hello', '.', {h = 'H', e = false}))",
        "fmt(string.gsub('abc', '%w', function(c) return c:upper() .. '!' end))",
        "fmt(string.gsub('abc', 'b', '%%'))",
        "fmt(string.gsub('  x  ', '^%s+', ''))",
        "fmt(string.gsub('a,b,,c', ',', ';'))",
        "fmt(string.gsub('abc', '(b)', '[%1]'))",
        "fmt(string.gsub('abc', 'b', 5))",
        "fmt(string.gsub('abc', '[', ''))",
        "fmt(string.find('abc', '%'))",
        "it(string.gmatch, 'one two  three', '%a+')",
        "it(string.gmatch, 'k1=v1,k2=v2', '(%w+)=(%w+)')",
        "it(string.gmatch, 'abc', '%a*')",
        "it(string.gmatch, 'abc', '()')",
        "it(string.gmatch, 'a b', '%f[%w]%w+')",
        "it(string.gmatch, 'aXb', 'x')",
    ];
    let reference = mlua::Lua::new();
    let (mut vm, sink) = vm();
    for case in cases {
        let call = format!("local ok, r = pcall(function() return {case} end) ");
        let expected: String = reference
            .load(format!("{FMT}{call}return ok and r or 'ERR'"))
            .eval()
            .unwrap();
        sink.lines.borrow_mut().clear();
        vm.run_script("T", &format!("{FMT}{call}print(ok and r or 'ERR')"))
            .unwrap();
        assert_eq!(
            sink.lines.borrow()[0],
            format!("print:{expected}"),
            "case: {case}"
        );
    }
}

#[test]
fn string_rep_with_string_count_is_bounded() {
    let (mut vm, _) = vm();
    let start = Instant::now();
    let err = vm
        .run_script("T", "string.rep('x', '100000000')")
        .unwrap_err();
    assert!(
        err.message.starts_with("T:1:") && err.message.contains("too large"),
        "{}",
        err.message
    );
    assert!(
        vm.run_script("T", "string.rep(12345, '100000000')")
            .is_err()
    );
    assert!(vm.run_script("T", "('x'):rep(1e300)").is_err());
    vm.run_script(
        "T",
        "assert(('x'):rep('3') == 'xxx' and string.rep('ab', 2) == 'abab')",
    )
    .unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
}

struct DropSink(Rc<RefCell<u32>>);

impl OutputSink for DropSink {
    fn print(&self, _: &str) {}
    fn warn(&self, _: &str) {}
    fn error(&self, _: &str) {}
}

impl Drop for DropSink {
    fn drop(&mut self) {
        *self.0.borrow_mut() += 1;
    }
}

#[test]
fn vm_is_freed_after_shutdown_even_with_budget_closures() {
    let drops = Rc::new(RefCell::new(0));
    let vm = ScriptVm::with_output(Context::Server, Rc::new(DropSink(drops.clone()))).unwrap();
    let budget = vm.budget();
    let host = vm
        .lua()
        .create_function(move |_, f: Function| {
            budget.call::<()>(&f, ()).map_err(mlua::Error::external)
        })
        .unwrap();
    vm.set_global("host_call", host).unwrap();
    vm.shutdown();
    assert_eq!(
        *drops.borrow(),
        1,
        "VM (and its sink) leaked through the Budget closure"
    );
}

#[test]
fn budget_errors_cleanly_when_vm_is_gone() {
    let (vm, _) = vm();
    let budget = vm.budget();
    let f = vm.lua().create_function(|_, ()| Ok(())).unwrap();
    let thread = vm.lua().create_thread(f).unwrap();
    drop(vm);
    let err = budget.resume(&thread, ()).unwrap_err();
    assert!(err.message.contains("no longer exists"), "{}", err.message);
}

#[test]
fn timeout_reported_once() {
    let (mut vm, sink) = vm();
    // A spawned thread times out inside run_script: one error, returned, none in the sink.
    let err = vm
        .run_script("T", "task.spawn(function() while true do end end)")
        .unwrap_err();
    assert!(err.message.contains("Script timeout"), "{}", err.message);
    assert!(sink.lines.borrow().is_empty(), "{:?}", sink.lines.borrow());
    // In a stepped thread: exactly one sink line.
    vm.run_script(
        "U",
        "task.delay(1, function() task.spawn(function() while true do end end) end)",
    )
    .unwrap();
    vm.step(2.0);
    assert_eq!(sink.lines.borrow().len(), 1, "{:?}", sink.lines.borrow());
    sink.lines.borrow_mut().clear();
    // A pcall-caught timeout followed by a yield is reported once and the thread is dropped.
    vm.run_script(
        "V",
        "task.delay(1, function() pcall(function() while true do end end) task.wait(1) print('after') end)",
    )
    .unwrap();
    vm.step(3.0);
    vm.step(10.0);
    assert_eq!(sink.lines.borrow().len(), 1, "{:?}", sink.lines.borrow());
    assert!(sink.lines.borrow()[0].starts_with("error:V:1: Script timeout"));
}

/// Runs `src` and asserts it stops with a pattern/timeout error within 0.5 s (#166).
fn assert_pattern_stops_fast(src: &str) {
    let (mut vm, _) = vm();
    let start = Instant::now();
    let err = vm.run_script("T", src).unwrap_err();
    let took = start.elapsed();
    assert!(
        err.message.contains("pattern too complex") || err.message.contains("Script timeout"),
        "{}",
        err.message
    );
    assert!(took < Duration::from_millis(500), "took {took:?}");
    vm.run_script("T", "assert(string.match('abc', '[a-c]+') == 'abc')")
        .unwrap();
}

#[test]
fn long_bracket_class_star_is_bounded() {
    // A 1 MB class whose only match is its last byte, repeated over 10k characters.
    assert_pattern_stops_fast(
        "local class = string.rep('b', 1000000) .. 'a'
         string.find(string.rep('a', 10000), '[' .. class .. ']*$')",
    );
}

#[test]
fn long_bracket_class_lazy_is_bounded() {
    assert_pattern_stops_fast(
        "local class = string.rep('b', 1000000) .. 'a'
         string.find(string.rep('a', 100000), '[' .. class .. ']-c')",
    );
}

#[test]
fn big_sort_without_comparator_is_refused_fast() {
    // #274: 8M numbers sorted in C ran 1.1 s past the budget (1M here keeps the test light). Now it is refused up front.
    let (mut vm, _) = vm();
    let start = Instant::now();
    let err = vm
        .run_script("T", "local t = table.create(1000000, 1) table.sort(t)")
        .unwrap_err();
    assert!(
        err.message.contains("table.sort: table too large"),
        "{}",
        err.message
    );
    assert!(err.message.starts_with("T:1:"), "{}", err.message);
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "{:?}",
        start.elapsed()
    );
    // Raw length counts: a __len metamethod cannot hide the size.
    let err = vm
        .run_script(
            "T",
            "local t = setmetatable(table.create(2000000, 1), {__len = function() return 1 end}) table.sort(t)",
        )
        .unwrap_err();
    assert!(err.message.contains("too large"), "{}", err.message);
}

#[test]
fn big_sort_with_a_comparator_stays_interruptible() {
    let (mut vm, _) = vm();
    let start = Instant::now();
    let err = vm
        .run_script(
            "T",
            "local t = table.create(2000000, 1) table.sort(t, function(a, b) while true do end end)",
        )
        .unwrap_err();
    assert!(err.message.contains("Script timeout"), "{}", err.message);
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn normal_sorts_behave_as_before() {
    let (mut vm, _) = vm();
    vm.run_script(
        "T",
        "local t = {5, 2, 9, 1, 7}
         table.sort(t)
         assert(table.concat(t, ',') == '1,2,5,7,9')
         table.sort(t, function(a, b) return a > b end)
         assert(table.concat(t, ',') == '9,7,5,2,1')
         local w = {'b', 'a', 'c'}
         table.sort(w)
         assert(w[1] == 'a' and w[3] == 'c')
         -- a table at the limit still sorts, errors keep Luau's own wording
         local big = table.create(262144, 3)
         big[1] = 1
         table.sort(big)
         assert(big[1] == 1)
         local ok, e = pcall(table.sort, 5)
         assert(not ok)
         local ok2, e2 = pcall(table.sort, {3, 'x', 1})
         assert(not ok2)",
    )
    .unwrap();
}
