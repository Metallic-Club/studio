//! Scheduler tests: the `task` library's order, frame phases, budgets, signal waits and `Once`, and reaping, run
//! against the mock backend and the real class set.

use std::time::Duration;

use metallic_core::{DataModel, Id, ScriptValue};

use crate::tests::{errors, script, service, setup};
use crate::{MAX_DEFER_DEPTH, OutputLevel, Phase, ScriptHost, ThreadState, YieldReason};

/// Runs `source` as a new script named `name`.
fn run(host: &mut ScriptHost, dm: &mut DataModel, name: &str, source: &str) -> Id {
    let id = script(dm, name);
    host.run_script(dm, id, &format!("Scripts/{name}.mock"), source)
        .expect("starts");
    id
}

/// Parents a new `Folder` to the workspace, firing `ChildAdded` on it.
fn add_folder(dm: &mut DataModel) -> Id {
    let class = dm.registry().class_id("Folder").expect("Folder");
    let folder = dm.create(class).expect("create");
    let workspace = service(dm, "Workspace");
    dm.set_parent(folder, Some(workspace)).expect("parent");
    folder
}

fn warnings(host: &ScriptHost) -> usize {
    host.output().of_level(OutputLevel::Warn).count()
}

#[test]
fn spawn_runs_nested_then_defer_then_delay_and_wait_in_sleep_order() {
    let (mut dm, mut host, log) = setup();
    let source = "\
        record main1\n\
        task.spawn record spawn1 ; defer ; record spawn2\n\
        task.defer record defer1\n\
        task.delay 0 record delay0\n\
        wait 0\n\
        record main2\n";
    run(&mut host, &mut dm, "S", source);
    // `task.spawn` ran its thread before the caller went on; the rest is queued.
    assert_eq!(log.borrow().labels(), ["main1", "spawn1"]);
    assert_eq!(host.deferred_count(), 2);
    assert_eq!(host.sleeper_count(), 2);

    // Any step runs the deferred queue, in the order it was filled; sleepers wait for `Heartbeat`.
    let stats = host.step(&mut dm, Phase::PreSimulation, 0.0);
    assert_eq!(stats.resumed, 2);
    assert_eq!(log.borrow().labels(), ["main1", "spawn1", "spawn2", "defer1"]);
    host.step(&mut dm, Phase::PostSimulation, 0.0);
    assert_eq!(log.borrow().records.len(), 4);

    // Equal wake times resume in sleep order: the delay was asked for before the main thread slept.
    host.step(&mut dm, Phase::Heartbeat, 0.0);
    assert_eq!(
        log.borrow().labels(),
        ["main1", "spawn1", "spawn2", "defer1", "delay0", "main2"]
    );
    assert_eq!(host.thread_count(), 0);
    assert!(errors(&host).is_empty());
}

#[test]
fn sleepers_resume_by_wake_time_and_ties_in_sleep_order() {
    let (mut dm, mut host, log) = setup();
    for (name, seconds) in [("A", 2), ("B", 1), ("C", 1), ("D", 2), ("E", 3)] {
        run(&mut host, &mut dm, name, &format!("wait {seconds}\nrecord {name}\n"));
    }
    host.step(&mut dm, Phase::Heartbeat, 0.5);
    assert!(log.borrow().records.is_empty());
    host.step(&mut dm, Phase::Heartbeat, 2.0);
    assert_eq!(log.borrow().labels(), ["B", "C", "A", "D"]);
    host.step(&mut dm, Phase::Heartbeat, 2.5);
    assert_eq!(log.borrow().records.len(), 4);
    host.step(&mut dm, Phase::Heartbeat, 3.0);
    assert_eq!(log.borrow().labels(), ["B", "C", "A", "D", "E"]);
}

#[test]
fn task_wait_returns_the_elapsed_time_and_lasts_at_least_until_the_next_heartbeat() {
    let (mut dm, mut host, log) = setup();
    host.step(&mut dm, Phase::Heartbeat, 10.0);
    run(&mut host, &mut dm, "S", "wait 0.5\nrecord\nwait 0\nrecord\n");
    host.step(&mut dm, Phase::Heartbeat, 10.25);
    assert!(log.borrow().records.is_empty());
    host.step(&mut dm, Phase::Heartbeat, 10.75);
    assert_eq!(log.borrow().records[0].1, vec![ScriptValue::Number(0.75)]);
    // `wait 0` during a Heartbeat waits for the next one, even at the same game time.
    assert_eq!(log.borrow().records.len(), 1);
    host.step(&mut dm, Phase::Heartbeat, 10.75);
    assert_eq!(log.borrow().records[1].1, vec![ScriptValue::Number(0.0)]);

    // Elapsed time is never less than asked for, whatever the float rounding.
    let (mut dm, mut host, log) = setup();
    host.step(&mut dm, Phase::Heartbeat, 0.1);
    run(&mut host, &mut dm, "S", "wait 0.2\nrecord\n");
    host.step(&mut dm, Phase::Heartbeat, 0.1 + 0.2);
    let records = &log.borrow().records;
    let [ScriptValue::Number(elapsed)] = records[0].1[..] else {
        panic!("expected one number, got {:?}", records[0].1);
    };
    assert!(elapsed >= 0.2, "{elapsed}");
}

#[test]
fn a_wait_during_a_frame_resumes_at_that_frames_heartbeat() {
    let (mut dm, mut host, log) = setup();
    run(
        &mut host,
        &mut dm,
        "S",
        "connect c %Workspace ChildAdded wait 0 ; record woke\n",
    );
    add_folder(&mut dm);
    host.step(&mut dm, Phase::PreSimulation, 1.0);
    assert_eq!(host.sleeper_count(), 1);
    host.step(&mut dm, Phase::PostSimulation, 1.0);
    assert!(log.borrow().records.is_empty());
    host.step(&mut dm, Phase::Heartbeat, 1.0);
    assert_eq!(log.borrow().labels(), ["woke"]);
}

#[test]
fn task_functions_pass_their_arguments() {
    let (mut dm, mut host, log) = setup();
    run(
        &mut host,
        &mut dm,
        "S",
        "task.spawn a 1 | record\ntask.defer b 2 | record\ntask.delay 1 c 3 | record\n",
    );
    host.step(&mut dm, Phase::PreSimulation, 0.0);
    host.step(&mut dm, Phase::Heartbeat, 1.0);
    let values: Vec<_> = log.borrow().records.iter().map(|(_, v)| v.clone()).collect();
    assert_eq!(
        values,
        vec![
            vec![ScriptValue::from("a"), ScriptValue::Number(1.0)],
            vec![ScriptValue::from("b"), ScriptValue::Number(2.0)],
            vec![ScriptValue::from("c"), ScriptValue::Number(3.0)],
        ]
    );
}

#[test]
fn cancel_unschedules_sleepers_deferred_threads_and_waiters() {
    let (mut dm, mut host, log) = setup();
    let source = "\
        task.delay 1 record never -> a\n\
        task.defer record never -> b\n\
        task.spawn waitsignal %Workspace ChildAdded ; record never -> c\n\
        task.cancel $a\n\
        task.cancel $b\n\
        task.cancel $c\n\
        record ok\n";
    run(&mut host, &mut dm, "S", source);
    assert_eq!(host.sleeper_count(), 0);
    assert_eq!(host.deferred_count(), 0);
    assert_eq!(host.waiter_count(), 0);
    assert_eq!(dm.connection_count(), 0);
    assert_eq!(host.thread_count(), 0);
    add_folder(&mut dm);
    host.step(&mut dm, Phase::Heartbeat, 5.0);
    assert_eq!(log.borrow().labels(), ["ok"]);

    // The engine can cancel a sleeping thread too, once.
    let s = script(&mut dm, "Sleeper");
    let started = host
        .run_script(&mut dm, s, "Scripts/Sleeper.mock", "wait 1\nrecord never\n")
        .expect("starts");
    assert!(host.cancel(&mut dm, started.thread));
    assert!(!host.cancel(&mut dm, started.thread));
    assert_eq!(host.sleeper_count(), 0);
    host.step(&mut dm, Phase::Heartbeat, 10.0);
    assert_eq!(log.borrow().labels(), ["ok"]);
    assert!(errors(&host).is_empty());
}

#[test]
fn deferred_work_runs_until_the_queue_is_empty_up_to_the_depth_limit() {
    let (mut dm, mut host, log) = setup();
    let depth = MAX_DEFER_DEPTH as usize;
    let deepest = format!("{}record deepest\n", "task.defer ".repeat(depth));
    let too_deep = format!("{}record never\n", "task.defer ".repeat(depth + 1));
    run(&mut host, &mut dm, "Deep", &deepest);
    run(&mut host, &mut dm, "TooDeep", &too_deep);
    run(&mut host, &mut dm, "Other", "task.defer record other\n");

    let stats = host.step(&mut dm, Phase::PreSimulation, 0.0);
    // Two chains of 80 and the other script's one: every one ran in this step.
    assert_eq!(stats.resumed, 2 * depth + 1);
    assert_eq!(log.borrow().labels(), ["other", "deepest"]);
    assert_eq!(host.deferred_count(), 0);
    // Only the thread that deferred too deep failed, at its call.
    let reported = errors(&host);
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].script, "Workspace.TooDeep");
    assert_eq!(reported[0].line, Some(1));
    assert_eq!(
        reported[0].message,
        "maximum re-entrancy depth (80) exceeded calling task.defer"
    );
    assert_eq!(host.thread_count(), 0);

    // The depth starts again at the next resumption point.
    run(&mut host, &mut dm, "Again", &deepest);
    host.step(&mut dm, Phase::PreSimulation, 0.0);
    assert_eq!(log.borrow().labels(), ["other", "deepest", "deepest"]);
}

#[test]
fn once_fires_once_and_is_released() {
    let (mut dm, mut host, log) = setup();
    run(&mut host, &mut dm, "S", "once c %Workspace ChildAdded record\n");
    assert_eq!(host.listener_count(), 1);
    let first = add_folder(&mut dm);
    add_folder(&mut dm);
    host.step(&mut dm, Phase::Heartbeat, 0.0);
    assert_eq!(log.borrow().records.len(), 1);
    assert_eq!(log.borrow().records[0].1, vec![ScriptValue::Instance(first)]);
    assert_eq!(host.listener_count(), 0);
    assert_eq!(dm.connection_count(), 0);
    assert_eq!(log.borrow().released_listeners.len(), 1);
    add_folder(&mut dm);
    host.step(&mut dm, Phase::Heartbeat, 0.0);
    assert_eq!(log.borrow().records.len(), 1);

    // A `Once` can be disconnected before it fires.
    run(
        &mut host,
        &mut dm,
        "T",
        "once c %Workspace ChildAdded record\ndisconnect $c\n",
    );
    assert_eq!(host.listener_count(), 0);
    add_folder(&mut dm);
    host.step(&mut dm, Phase::Heartbeat, 0.0);
    assert_eq!(log.borrow().records.len(), 1);
}

#[test]
fn signal_wait_resumes_with_the_event_args_then_disconnects() {
    let (mut dm, mut host, log) = setup();
    let s = script(&mut dm, "S");
    let started = host
        .run_script(
            &mut dm,
            s,
            "Scripts/S.mock",
            "waitsignal %Workspace ChildAdded\nrecord\nreturn\n",
        )
        .expect("starts");
    assert!(matches!(
        host.thread_state(started.thread),
        Some(ThreadState::Yielded(YieldReason::WaitSignal { .. }))
    ));
    assert_eq!(host.waiter_count(), 1);
    assert_eq!(dm.connection_count(), 1);
    // The waiter's connection is the host's, not a script listener.
    assert_eq!(host.listener_count(), 0);

    let folder = add_folder(&mut dm);
    add_folder(&mut dm);
    host.step(&mut dm, Phase::PostSimulation, 0.0);
    assert_eq!(log.borrow().records, vec![(s, vec![ScriptValue::Instance(folder)])]);
    assert_eq!(host.waiter_count(), 0);
    assert_eq!(dm.connection_count(), 0);
    assert_eq!(host.thread_count(), 0);

    // A property's changed signal works the same way, with no arguments. The spawned thread is waiting as soon as
    // `task.spawn` returns, so the change its caller makes right after wakes it.
    let source = "new Part -> p\ntask.spawn waitchanged $p Name ; record\nset $p Name Door\n";
    let p = run(&mut host, &mut dm, "P", source);
    assert_eq!(host.waiter_count(), 1);
    host.step(&mut dm, Phase::PostSimulation, 0.0);
    assert_eq!(log.borrow().records.last(), Some(&(p, vec![])));
    assert_eq!(host.waiter_count(), 0);
}

#[test]
fn a_handler_may_yield_and_a_failing_one_ends_alone() {
    let (mut dm, mut host, log) = setup();
    // Both script instances exist before either connects, so only the folder below fires.
    let yielding = script(&mut dm, "Yielding");
    let failing = script(&mut dm, "Failing");
    let source = "connect c %Workspace ChildAdded record h1 ; wait 1 ; record h2\n";
    host.run_script(&mut dm, yielding, "Scripts/Yielding.mock", source)
        .expect("starts");
    let source = "connect c %Workspace ChildAdded record f1 ; error handler broke ; record never\n";
    host.run_script(&mut dm, failing, "Scripts/Failing.mock", source)
        .expect("starts");
    add_folder(&mut dm);
    host.step(&mut dm, Phase::PostSimulation, 0.0);
    assert_eq!(log.borrow().labels(), ["h1", "f1"]);
    let reported = errors(&host);
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].script, "Workspace.Failing");
    assert_eq!(reported[0].message, "handler broke");
    assert_eq!(reported[0].line, Some(1));

    host.step(&mut dm, Phase::Heartbeat, 0.5);
    assert_eq!(log.borrow().records.len(), 2);
    host.step(&mut dm, Phase::Heartbeat, 1.0);
    assert_eq!(log.borrow().labels(), ["h1", "f1", "h2"]);
    // Both listeners are still connected for the next event.
    assert_eq!(host.listener_count(), 2);
}

#[test]
fn errors_name_script_and_line_and_other_threads_continue() {
    let (mut dm, mut host, log) = setup();
    run(&mut host, &mut dm, "A", "wait 1\nerror boom\n");
    run(&mut host, &mut dm, "B", "wait 1\nrecord b\n");
    run(&mut host, &mut dm, "C", "task.spawn error kid\nrecord after-spawn\n");
    host.step(&mut dm, Phase::Heartbeat, 1.0);
    assert_eq!(log.borrow().labels(), ["after-spawn", "b"]);
    let reported = errors(&host);
    assert_eq!(reported.len(), 2);
    assert_eq!(
        (
            reported[0].script.as_str(),
            reported[0].line,
            reported[0].message.as_str()
        ),
        ("Workspace.C", Some(1), "kid")
    );
    assert_eq!(
        (
            reported[1].script.as_str(),
            reported[1].line,
            reported[1].message.as_str()
        ),
        ("Workspace.A", Some(2), "boom")
    );
}

#[test]
fn a_spent_phase_budget_carries_due_work_over_in_order() {
    let (mut dm, mut host, log) = setup();
    // A zero budget lets exactly one resume through per step.
    host.set_phase_budget(Duration::ZERO);
    for name in ["A", "B", "C"] {
        run(&mut host, &mut dm, name, &format!("wait 1\nrecord {name}\n"));
    }
    for name in ["d1", "d2"] {
        run(&mut host, &mut dm, name, &format!("task.defer record {name}\n"));
    }

    // Deferred work first runs at any step; one per step here.
    let stats = host.step(&mut dm, Phase::PreSimulation, 0.0);
    assert_eq!((stats.resumed, stats.carried_over), (1, true));
    assert_eq!(warnings(&host), 1);
    let stats = host.step(&mut dm, Phase::PostSimulation, 0.0);
    assert_eq!((stats.resumed, stats.carried_over), (1, false));
    assert_eq!(log.borrow().labels(), ["d1", "d2"]);

    let stats = host.step(&mut dm, Phase::Heartbeat, 2.0);
    assert_eq!((stats.resumed, stats.carried_over), (1, true));
    // A new frame, so a new warning; the other phases of the same frame do not warn again.
    assert_eq!(warnings(&host), 2);
    host.step(&mut dm, Phase::PreSimulation, 2.1);
    assert_eq!(log.borrow().labels(), ["d1", "d2", "A"]);
    // A new sleeper that is also due waits behind the carried ones.
    run(&mut host, &mut dm, "Late", "wait 0\nrecord Late\n");
    for _ in 0..3 {
        host.step(&mut dm, Phase::Heartbeat, 3.0);
    }
    assert_eq!(log.borrow().labels(), ["d1", "d2", "A", "B", "C", "Late"]);
    let warning = host.output().of_level(OutputLevel::Warn).next().expect("a warning");
    assert_eq!(warning.script, "TaskScheduler");
    assert!(
        warning.message.starts_with("PreSimulation used up its"),
        "{}",
        warning.message
    );

    // Signal deliveries carry over too.
    run(&mut host, &mut dm, "L", "connect c %Workspace ChildAdded record\n");
    let first = add_folder(&mut dm);
    let second = add_folder(&mut dm);
    let stats = host.step(&mut dm, Phase::PostSimulation, 3.0);
    assert_eq!((stats.resumed, stats.carried_over), (1, true));
    host.step(&mut dm, Phase::PostSimulation, 3.0);
    let records = &log.borrow().records;
    let n = records.len();
    assert_eq!(records[n - 2].1, vec![ScriptValue::Instance(first)]);
    assert_eq!(records[n - 1].1, vec![ScriptValue::Instance(second)]);
}

#[test]
fn reap_clears_sleepers_deferred_threads_and_waiters() {
    let (mut dm, mut host, log) = setup();
    let s = script(&mut dm, "S");
    let other = script(&mut dm, "Other");
    host.run_script(&mut dm, other, "Scripts/Other.mock", "wait 4\nrecord other\n")
        .expect("starts");
    let source = "\
        task.delay 5 record never\n\
        task.defer record never\n\
        task.spawn waitsignal %Workspace ChildRemoved ; record never\n\
        connect c %Workspace ChildAdded wait 1 ; record never\n\
        wait 3\n\
        record never\n";
    host.run_script(&mut dm, s, "Scripts/S.mock", source).expect("starts");
    // The listener fires and its thread sleeps too.
    let folder = add_folder(&mut dm);
    assert_eq!(host.dispatch_signals(&mut dm), 1);
    assert_eq!(host.sleeper_count(), 4);
    assert_eq!(host.deferred_count(), 1);
    assert_eq!(host.waiter_count(), 1);
    assert_eq!(dm.connection_count(), 2);

    assert!(host.reap(&mut dm, s));
    assert_eq!(host.sleeper_count(), 1);
    assert_eq!(host.deferred_count(), 0);
    assert_eq!(host.waiter_count(), 0);
    assert_eq!(dm.connection_count(), 0);
    assert_eq!(host.thread_count(), 1);

    // Nothing of the reaped script runs again; the other script's sleeper still does.
    dm.set_parent(folder, None).expect("unparent");
    add_folder(&mut dm);
    host.step(&mut dm, Phase::Heartbeat, 10.0);
    assert_eq!(log.borrow().labels(), ["other"]);
    assert!(errors(&host).is_empty());
}

#[test]
fn reap_works_after_the_script_instance_is_destroyed() {
    let (mut dm, mut host, log) = setup();
    let s = run(
        &mut host,
        &mut dm,
        "S",
        "task.delay 1 record never\ntask.spawn waitsignal %Workspace ChildAdded\nwait 1\n",
    );
    dm.destroy(s).expect("destroy");
    assert!(host.reap(&mut dm, s));
    assert_eq!(
        (host.sleeper_count(), host.waiter_count(), host.thread_count()),
        (0, 0, 0)
    );
    assert_eq!(dm.connection_count(), 0);
    host.step(&mut dm, Phase::Heartbeat, 2.0);
    assert!(log.borrow().records.is_empty());
}

#[test]
fn resume_waiting_resumes_sleepers_without_dispatching_signals() {
    let (mut dm, mut host, log) = setup();
    run(&mut host, &mut dm, "W", "wait 1\nrecord slept\n");
    run(
        &mut host,
        &mut dm,
        "L",
        "connect c %Workspace ChildAdded record event\n",
    );
    add_folder(&mut dm);
    let stats = host.resume_waiting(&mut dm, 1.0);
    assert_eq!(stats.resumed, 1);
    assert_eq!(log.borrow().labels(), ["slept"]);
    assert_eq!(host.now(), 1.0);
    host.step(&mut dm, Phase::PreSimulation, 0.5);
    // Time never goes back.
    assert_eq!(host.now(), 1.0);
    assert_eq!(log.borrow().records.len(), 2);
}

#[test]
fn a_manual_resume_abandons_the_scheduled_one() {
    let (mut dm, mut host, log) = setup();
    let s = script(&mut dm, "S");
    let started = host
        .run_script(&mut dm, s, "Scripts/S.mock", "wait 5\nrecord\n")
        .expect("starts");
    assert_eq!(host.sleeper_count(), 1);
    host.resume_thread(&mut dm, started.thread, &[ScriptValue::from("early")])
        .expect("resumes");
    assert_eq!(host.sleeper_count(), 0);
    host.step(&mut dm, Phase::Heartbeat, 10.0);
    assert_eq!(log.borrow().labels(), ["early"]);
}
