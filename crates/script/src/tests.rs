//! Host tests, run against the mock backend and the real class set.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use metallic_core::{DataModel, Id, ScriptValue};

use crate::mock::{MockLanguage, MockLog, THREAD_BYTES};
use crate::{
    CompileError, HostError, OutputLevel, OutputLine, Resume, ScriptFailure, ScriptHost, ThreadState, YieldReason,
};

/// A new game, a host with the mock backend, and the mock's log.
pub(crate) fn setup() -> (DataModel, ScriptHost, Rc<RefCell<MockLog>>) {
    let dm = metallic_classes::new_game().unwrap_or_else(|e| panic!("{e}"));
    let mut host = ScriptHost::new();
    let (language, log) = MockLanguage::new();
    host.register_language(Box::new(language)).expect("register mock");
    (dm, host, log)
}

pub(crate) fn service(dm: &DataModel, class: &str) -> Id {
    let class = dm.registry().class_id(class).expect("class");
    dm.service(class).expect("service")
}

/// A stand-in script instance (the script classes come in #47): a `Folder` named `name` in `Workspace`.
pub(crate) fn script(dm: &mut DataModel, name: &str) -> Id {
    let class = dm.registry().class_id("Folder").expect("Folder");
    let id = dm.create(class).expect("create");
    dm.set_name(id, name.into()).expect("name");
    let workspace = service(dm, "Workspace");
    dm.set_parent(id, Some(workspace)).expect("parent");
    id
}

pub(crate) fn errors(host: &ScriptHost) -> Vec<&OutputLine> {
    host.output().of_level(OutputLevel::Error).collect()
}

#[test]
fn a_script_starts_and_finishes() {
    let (mut dm, mut host, log) = setup();
    let builder = script(&mut dm, "Builder");
    let source = "\
        # Builds a part the way a script would.\n\
        new Part -> part\n\
        set $part Name Door\n\
        set $part Parent %Workspace\n\
        call %Workspace FindFirstChild Door -> found\n\
        get $found Name -> name\n\
        print built\n\
        return $name $found\n";
    let run = host
        .run_script(&mut dm, builder, "Scripts/Builder.mock", source)
        .expect("runs");

    let workspace = service(&dm, "Workspace");
    let part = dm
        .find_first_child(workspace, "Door", false)
        .expect("lookup")
        .expect("the script made a part");
    assert_eq!(dm.class_name(part), Ok("Part"));
    assert_eq!(
        run.resume,
        Resume::Finished(vec![ScriptValue::from("Door"), ScriptValue::Instance(part)])
    );

    // The finished main thread is released; the script stays started until reaped.
    assert_eq!(host.thread_count(), 0);
    assert_eq!(log.borrow().released_threads, vec![run.thread.thread]);
    assert!(host.is_running(builder));
    let print = &host.output().lines[0];
    assert_eq!(print.level, OutputLevel::Print);
    assert_eq!(print.script, "Workspace.Builder");
    assert_eq!(print.message, "built");
    assert!(errors(&host).is_empty());
}

#[test]
fn chunk_names_follow_path_and_uid() {
    let (mut dm, mut host, log) = setup();
    let door = script(&mut dm, "Door");
    host.run_script(&mut dm, door, "Scripts/Door.mock", "return")
        .expect("runs");
    let expected = format!("Scripts/Door.mock#{}.{}", door.index(), door.generation());
    assert_eq!(log.borrow().spawned, vec![(door, expected)]);
}

#[test]
fn a_compile_error_reports_chunk_and_line() {
    let (mut dm, mut host, log) = setup();
    let bad = script(&mut dm, "Bad");
    let error = host
        .run_script(&mut dm, bad, "Scripts/Bad.mock", "print fine\n\n  frobnicate x\n")
        .expect_err("does not compile");
    assert_eq!(
        error,
        HostError::Compile(CompileError {
            chunk: "Scripts/Bad.mock".to_owned(),
            line: 3,
            column: Some(3),
            message: "unexpected 'frobnicate'".to_owned(),
        })
    );
    assert_eq!(error.to_string(), "Scripts/Bad.mock:3: unexpected 'frobnicate'");

    let reported = errors(&host);
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].script, "Workspace.Bad");
    assert_eq!(reported[0].chunk, "Scripts/Bad.mock");
    assert_eq!(reported[0].line, Some(3));
    // Nothing was spawned and the script is not running.
    assert!(log.borrow().spawned.is_empty());
    assert!(!host.is_running(bad));
}

#[test]
fn a_runtime_error_names_script_and_line_and_others_keep_running() {
    let (mut dm, mut host, _log) = setup();
    let a = script(&mut dm, "A");
    let b = script(&mut dm, "B");
    let c = script(&mut dm, "C");

    let run = host
        .run_script(&mut dm, a, "Scripts/A.mock", "print start\nerror boom\nprint never\n")
        .expect("starts");
    assert_eq!(run.resume, Resume::Errored(ScriptFailure::runtime("boom", Some(2))));
    // An engine call's error is raised at the calling line, with the engine's message.
    let run = host
        .run_script(&mut dm, b, "Scripts/B.mock", "call %Workspace Explode\n")
        .expect("starts");
    assert!(matches!(run.resume, Resume::Errored(_)));
    let run = host
        .run_script(&mut dm, c, "Scripts/C.mock", "return 1\n")
        .expect("starts");
    assert_eq!(run.resume, Resume::Finished(vec![ScriptValue::Number(1.0)]));

    let reported = errors(&host);
    assert_eq!(reported.len(), 2);
    assert_eq!(reported[0].script, "Workspace.A");
    assert_eq!(reported[0].line, Some(2));
    assert_eq!(reported[0].message, "boom");
    assert_eq!(
        reported[0].chunk,
        format!("Scripts/A.mock#{}.{}", a.index(), a.generation())
    );
    assert_eq!(reported[1].script, "Workspace.B");
    assert_eq!(reported[1].line, Some(1));
    assert_eq!(reported[1].message, "Explode is not a valid member of Workspace");
    // "print never" did not run.
    let prints: Vec<_> = host.output().of_level(OutputLevel::Print).collect();
    assert_eq!(prints.len(), 1);
    assert_eq!(host.thread_count(), 0);
}

#[test]
fn property_writes_are_checked_like_any_language() {
    let (mut dm, mut host, _log) = setup();
    let s = script(&mut dm, "S");
    host.run_script(&mut dm, s, "Scripts/S.mock", "new Part -> p\nset $p Name 5\n")
        .expect("starts");
    host.run_script(&mut dm, s, "Scripts/S.mock", "new Part -> p\nset $p ClassName Model\n")
        .expect("starts");
    let reported = errors(&host);
    assert_eq!(reported[0].message, "expected string, got number");
    assert_eq!(reported[0].line, Some(2));
    assert_eq!(reported[1].message, "ClassName is read-only");
}

#[test]
fn one_file_is_compiled_once_per_host() {
    let (mut dm, mut host, log) = setup();
    let first = script(&mut dm, "First");
    let second = script(&mut dm, "Second");
    let source = "record %script\n";
    host.run_script(&mut dm, first, "Scripts/Shared.mock", source)
        .expect("runs");
    host.run_script(&mut dm, second, "Scripts/Shared.mock", source)
        .expect("runs");
    assert_eq!(log.borrow().compiles, vec!["Scripts/Shared.mock".to_owned()]);
    // Both ran, each as itself.
    let records: Vec<_> = log.borrow().records.iter().map(|(script, _)| *script).collect();
    assert_eq!(records, vec![first, second]);

    // New content at the same path is a new chunk.
    host.run_script(&mut dm, first, "Scripts/Shared.mock", "return\n")
        .expect("runs");
    assert_eq!(log.borrow().compiles.len(), 2);
}

#[test]
fn a_budget_overrun_interrupts_only_that_thread() {
    let (mut dm, mut host, _log) = setup();
    host.set_resume_budget(Duration::from_millis(20));
    let sleeper = script(&mut dm, "Sleeper");
    let spinner = script(&mut dm, "Spinner");
    let after = script(&mut dm, "After");

    let sleeping = host
        .run_script(&mut dm, sleeper, "Scripts/Sleeper.mock", "wait 1\nreturn woke\n")
        .expect("starts");
    assert_eq!(sleeping.resume, Resume::Yielded(YieldReason::Wait { seconds: 1.0 }));

    let spun = host
        .run_script(
            &mut dm,
            spinner,
            "Scripts/Spinner.mock",
            "print go\nspin\nprint never\n",
        )
        .expect("starts");
    assert_eq!(spun.resume, Resume::Errored(ScriptFailure::Timeout { line: Some(2) }));
    let reported = errors(&host);
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].script, "Workspace.Spinner");
    assert_eq!(reported[0].line, Some(2));
    assert!(reported[0].message.contains("time budget"));

    // The other threads are untouched: the sleeper resumes and a new script runs.
    let woke = host.resume_thread(&mut dm, sleeping.thread, &[]).expect("resumes");
    assert_eq!(woke, Resume::Finished(vec![ScriptValue::from("woke")]));
    let ran = host
        .run_script(&mut dm, after, "Scripts/After.mock", "return 2\n")
        .expect("starts");
    assert_eq!(ran.resume, Resume::Finished(vec![ScriptValue::Number(2.0)]));
}

#[test]
fn a_signal_delivery_invokes_the_listener_with_its_args() {
    let (mut dm, mut host, log) = setup();
    let watcher = script(&mut dm, "Watcher");
    let workspace = service(&dm, "Workspace");
    host.run_script(
        &mut dm,
        watcher,
        "Scripts/Watcher.mock",
        "connect c %Workspace ChildAdded record\n",
    )
    .expect("starts");
    assert_eq!(host.listener_count(), 1);
    // Connecting fired nothing yet.
    assert_eq!(host.dispatch_signals(&mut dm), 0);

    let class = dm.registry().class_id("Part").expect("Part");
    let part = dm.create(class).expect("create");
    dm.set_parent(part, Some(workspace)).expect("parent");
    assert_eq!(host.dispatch_signals(&mut dm), 1);
    assert_eq!(log.borrow().records, vec![(watcher, vec![ScriptValue::Instance(part)])]);
    // The listener thread finished and was released.
    assert_eq!(host.thread_count(), 0);
}

#[test]
fn listener_threads_fail_or_yield_on_their_own() {
    let (mut dm, mut host, log) = setup();
    let workspace = service(&dm, "Workspace");
    let failing = script(&mut dm, "Failing");
    let yielding = script(&mut dm, "Yielding");
    let recording = script(&mut dm, "Recording");
    host.run_script(
        &mut dm,
        failing,
        "Scripts/F.mock",
        "connect c %Workspace ChildAdded error nope\n",
    )
    .expect("starts");
    host.run_script(
        &mut dm,
        yielding,
        "Scripts/Y.mock",
        "connect c %Workspace ChildAdded defer\n",
    )
    .expect("starts");
    host.run_script(
        &mut dm,
        recording,
        "Scripts/R.mock",
        "connect c %Workspace ChildAdded record\n",
    )
    .expect("starts");

    let class = dm.registry().class_id("Folder").expect("Folder");
    let folder = dm.create(class).expect("create");
    dm.set_parent(folder, Some(workspace)).expect("parent");
    assert_eq!(host.dispatch_signals(&mut dm), 3);

    let reported = errors(&host);
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].script, "Workspace.Failing");
    assert_eq!(reported[0].message, "nope");
    assert_eq!(reported[0].line, Some(1));
    assert_eq!(log.borrow().records.len(), 1);
    // The yielding handler waits for the scheduler.
    let threads: Vec<_> = host.threads_of(yielding).collect();
    assert_eq!(threads.len(), 1);
    assert_eq!(
        host.thread_state(threads[0]),
        Some(&ThreadState::Yielded(YieldReason::Defer))
    );
    assert_eq!(host.thread_count(), 1);
}

#[test]
fn disconnect_stops_deliveries() {
    let (mut dm, mut host, log) = setup();
    let s = script(&mut dm, "S");
    let workspace = service(&dm, "Workspace");
    host.run_script(
        &mut dm,
        s,
        "Scripts/S.mock",
        "connect c %Workspace ChildAdded record\ndisconnect $c\n",
    )
    .expect("starts");
    assert_eq!(host.listener_count(), 0);
    assert_eq!(dm.connection_count(), 0);
    let class = dm.registry().class_id("Folder").expect("Folder");
    let folder = dm.create(class).expect("create");
    dm.set_parent(folder, Some(workspace)).expect("parent");
    assert_eq!(host.dispatch_signals(&mut dm), 0);
    assert!(log.borrow().records.is_empty());
}

#[test]
fn reap_releases_everything_a_script_created() {
    let (mut dm, mut host, log) = setup();
    let workspace = service(&dm, "Workspace");
    let s = script(&mut dm, "S");
    let other = script(&mut dm, "Other");
    let source = "\
        connect a %Workspace ChildAdded wait 10\n\
        connect b %Workspace ChildRemoved record\n\
        fork\n\
        wait 5\n";
    let run = host.run_script(&mut dm, s, "Scripts/S.mock", source).expect("starts");
    assert_eq!(run.resume, Resume::Yielded(YieldReason::Wait { seconds: 5.0 }));
    host.run_script(
        &mut dm,
        other,
        "Scripts/O.mock",
        "connect c %Workspace ChildAdded record\n",
    )
    .expect("starts");

    // A delivery leaves a suspended listener thread behind.
    let class = dm.registry().class_id("Folder").expect("Folder");
    let folder = dm.create(class).expect("create");
    dm.set_parent(folder, Some(workspace)).expect("parent");
    assert_eq!(host.dispatch_signals(&mut dm), 2);
    // Main thread, forked thread and the yielded listener thread.
    let threads: Vec<_> = host.threads_of(s).collect();
    assert_eq!(threads.len(), 3);
    assert!(
        threads
            .iter()
            .any(|&t| host.thread_state(t) == Some(&ThreadState::Created))
    );
    assert_eq!(host.listener_count(), 3);
    assert_eq!(dm.connection_count(), 3);

    let released_before = log.borrow().released_threads.len();
    assert!(host.reap(&mut dm, s));
    {
        let log = log.borrow();
        let mut released: Vec<_> = log.released_threads[released_before..].to_vec();
        released.sort_by_key(|t| t.0);
        let mut expected: Vec<_> = threads.iter().map(|t| t.thread).collect();
        expected.sort_by_key(|t| t.0);
        assert_eq!(released, expected);
        assert_eq!(log.released_listeners.len(), 2);
    }
    assert_eq!(host.threads_of(s).count(), 0);
    assert_eq!(host.thread_count(), 0);
    assert_eq!(host.listener_count(), 1);
    assert_eq!(dm.connection_count(), 1);
    assert!(!host.is_running(s));
    assert!(!host.reap(&mut dm, s));
    assert_eq!(
        host.resume_thread(&mut dm, run.thread, &[]),
        Err(HostError::UnknownThread(run.thread))
    );

    // The other script still receives its events; the reaped one receives none.
    dm.set_parent(folder, None).expect("unparent");
    let folder2 = dm.create(class).expect("create");
    dm.set_parent(folder2, Some(workspace)).expect("parent");
    let records_before = log.borrow().records.len();
    assert_eq!(host.dispatch_signals(&mut dm), 1);
    let log = log.borrow();
    assert_eq!(log.records.len(), records_before + 1);
    assert_eq!(log.records[records_before].0, other);
}

#[test]
fn running_a_script_again_restarts_it() {
    let (mut dm, mut host, log) = setup();
    let s = script(&mut dm, "S");
    host.run_script(
        &mut dm,
        s,
        "Scripts/S.mock",
        "connect c %Workspace ChildAdded record\nwait 1\n",
    )
    .expect("starts");
    assert_eq!(host.thread_count(), 1);
    host.run_script(&mut dm, s, "Scripts/S.mock", "return\n")
        .expect("restarts");
    assert_eq!(host.thread_count(), 0);
    assert_eq!(host.listener_count(), 0);
    assert_eq!(log.borrow().released_listeners.len(), 1);
}

#[test]
fn an_unknown_extension_is_a_clear_error() {
    let (mut dm, mut host, log) = setup();
    let s = script(&mut dm, "S");
    let error = host
        .run_script(&mut dm, s, "Scripts/Door.py", "print hi\n")
        .expect_err("no python");
    assert_eq!(
        error,
        HostError::UnknownExtension {
            path: "Scripts/Door.py".to_owned(),
            extension: "py".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "Scripts/Door.py: no script language runs `.py` files"
    );
    let error = host
        .run_script(&mut dm, s, "Scripts/Door", "print hi\n")
        .expect_err("no extension");
    assert!(matches!(error, HostError::NoExtension { .. }));
    assert_eq!(errors(&host).len(), 2);
    assert_eq!(errors(&host)[0].script, "Workspace.S");
    assert!(log.borrow().compiles.is_empty());
    // Extensions match without regard to case.
    host.run_script(&mut dm, s, "Scripts/Door.MOCK", "return\n")
        .expect("runs");
}

#[test]
fn two_backends_cannot_share_an_extension() {
    let (_dm, mut host, _log) = setup();
    let (second, _) = MockLanguage::new();
    let error = host.register_language(Box::new(second)).expect_err("taken");
    assert_eq!(error.to_string(), "`.mock` files are already run by mock");
}

#[test]
fn memory_limits_reach_every_backend() {
    let (mut dm, mut host, log) = setup();
    host.set_memory_limit(Some(THREAD_BYTES));
    assert_eq!(log.borrow().memory_limit, Some(THREAD_BYTES));

    let a = script(&mut dm, "A");
    let b = script(&mut dm, "B");
    host.run_script(&mut dm, a, "Scripts/A.mock", "wait 1\n")
        .expect("starts");
    assert_eq!(host.memory_used(), THREAD_BYTES);
    let error = host
        .run_script(&mut dm, b, "Scripts/B.mock", "return\n")
        .expect_err("out of memory");
    assert_eq!(error, HostError::Spawn(ScriptFailure::OutOfMemory));
    assert_eq!(errors(&host)[0].script, "Workspace.B");
    assert!(!host.is_running(b));

    host.set_memory_limit(None);
    assert_eq!(log.borrow().memory_limit, None);
}

#[test]
fn a_boxed_sink_works_too() {
    let mut dm = metallic_classes::new_game().unwrap_or_else(|e| panic!("{e}"));
    let mut host: ScriptHost<Box<dyn crate::OutputSink>> =
        ScriptHost::with_output(Box::new(crate::CollectOutput::new()));
    let (language, _log) = MockLanguage::new();
    host.register_language(Box::new(language)).expect("register");
    let s = script(&mut dm, "S");
    let run = host
        .run_script(&mut dm, s, "Scripts/S.mock", "warn careful\nreturn\n")
        .expect("runs");
    assert_eq!(run.resume, Resume::Finished(vec![]));
}
