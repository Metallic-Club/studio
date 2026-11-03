//! Running script instances, `require`, `BindableFunction` and dropped connections, against the mock backend.

use std::cell::RefCell;
use std::rc::Rc;

use metallic_core::{DataModel, Id, ScriptValue, Value};

use crate::mock::{MockLanguage, MockLog};
use crate::tests::{errors, service, setup};
use crate::{HostError, MemorySources, Phase, RunContext, ScriptHost};

/// A game, a host with the mock backend, its log, and the game clock.
struct World {
    dm: DataModel,
    host: ScriptHost,
    log: Rc<RefCell<MockLog>>,
    sources: MemorySources,
    now: f64,
}

impl World {
    fn new() -> Self {
        let (dm, host, log) = setup();
        Self {
            dm,
            host,
            log,
            sources: MemorySources::new(),
            now: 0.0,
        }
    }

    fn service(&self, class: &str) -> Id {
        service(&self.dm, class)
    }

    /// A new `class` instance named `name` under `parent`, running `source` from `<name>.mock` (no `Source` when
    /// `source` is `None`).
    fn add(&mut self, class: &str, parent: Id, name: &str, source: Option<&str>) -> Id {
        let id = self.create(class, name, source);
        self.dm.set_parent(id, Some(parent)).expect("parent");
        id
    }

    /// Like [`World::add`], with no parent yet.
    fn create(&mut self, class: &str, name: &str, source: Option<&str>) -> Id {
        let class = self.dm.registry().class_id(class).expect("class");
        let id = self.dm.create(class).expect("create");
        self.dm.set_name(id, name.into()).expect("name");
        if let Some(source) = source {
            let path = format!("{name}.mock");
            self.sources.insert(&path, source);
            self.dm
                .set_by_name(id, "Source", Value::String(path.into()))
                .expect("Source");
        }
        id
    }

    fn start(&mut self, context: RunContext) {
        let sources = std::mem::take(&mut self.sources);
        self.host
            .start_game(&mut self.dm, context, sources)
            .expect("the game starts");
    }

    /// One frame of 1/60 s.
    fn frame(&mut self) {
        self.frames(1);
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.now += 1.0 / 60.0;
            for phase in Phase::FRAME {
                self.host.step(&mut self.dm, phase, self.now);
            }
        }
    }

    fn labels(&self) -> Vec<String> {
        self.log.borrow().labels()
    }

    fn set_enabled(&mut self, script: Id, enabled: bool) {
        self.dm
            .set_by_name(script, "Enabled", Value::Bool(enabled))
            .expect("Enabled");
    }

    /// Every error message (first line), in order.
    fn errors(&self) -> Vec<String> {
        errors(&self.host)
            .iter()
            .map(|l| l.message.lines().next().unwrap_or_default().to_owned())
            .collect()
    }

    /// Every error as `script:line: message`.
    fn located_errors(&self) -> Vec<String> {
        errors(&self.host)
            .iter()
            .map(|l| {
                let message = l.message.lines().next().unwrap_or_default();
                match l.line {
                    Some(line) => format!("{}:{line}: {message}", l.script),
                    None => format!("{}: {message}", l.script),
                }
            })
            .collect()
    }
}

/// The source of a script that records `label` and then sleeps, so it stays running.
fn recorder(label: &str) -> String {
    format!("record {label}\nwait 1000\n")
}

#[test]
fn start_game_runs_enabled_scripts_in_tree_order_for_the_context() {
    for (context, expected) in [
        (RunContext::Local, &["A", "B", "L", "S"][..]),
        (RunContext::Server, &["A", "B", "S"][..]),
        (RunContext::Client, &["L"][..]),
    ] {
        let mut w = World::new();
        let workspace = w.service("Workspace");
        let server = w.service("ServerScriptService");
        let storage = w.service("ReplicatedStorage");
        w.add("Script", workspace, "A", Some(&recorder("A")));
        let model = w.add("Model", workspace, "Model", None);
        w.add("Script", model, "B", Some(&recorder("B")));
        let off = w.add("Script", workspace, "Off", Some(&recorder("Off")));
        w.set_enabled(off, false);
        w.add("LocalScript", workspace, "L", Some(&recorder("L")));
        w.add("ModuleScript", workspace, "M", Some("record M\nreturn 1\n"));
        w.add("Script", workspace, "Empty", None);
        w.add("Script", storage, "Stored", Some(&recorder("Stored")));
        w.add("Script", server, "S", Some(&recorder("S")));
        w.start(context);
        assert_eq!(w.labels(), expected, "{context:?}");
        assert_eq!(w.host.run_context(), Some(context));
        assert!(w.errors().is_empty(), "{:?}", w.errors());
    }
}

#[test]
fn a_game_starts_once_and_stops_cleanly() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    w.add(
        "Script",
        workspace,
        "A",
        Some("connect _ %Workspace ChildAdded record child\nwait 1000\n"),
    );
    w.start(RunContext::Local);
    let error = w
        .host
        .start_game(&mut w.dm, RunContext::Local, MemorySources::new())
        .expect_err("already running");
    assert_eq!(error, HostError::GameRunning);
    assert_eq!(w.host.thread_count(), 1);
    assert_eq!(w.host.listener_count(), 1);

    assert!(w.host.stop_game(&mut w.dm));
    assert!(!w.host.stop_game(&mut w.dm));
    assert_eq!(w.host.thread_count(), 0);
    assert_eq!(w.host.listener_count(), 0);
    assert_eq!(w.host.watch_count(), 0);
    assert_eq!(w.dm.connection_count(), 0);
    assert_eq!(w.host.run_context(), None);
}

#[test]
fn a_missing_source_file_is_reported_and_others_run() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    let lost = w.add("Script", workspace, "Lost", None);
    w.dm.set_by_name(lost, "Source", Value::String("Scripts/Nowhere.mock".into()))
        .expect("Source");
    w.add("Script", workspace, "Fine", Some(&recorder("fine")));
    w.start(RunContext::Local);
    assert_eq!(w.labels(), ["fine"]);
    assert_eq!(
        w.located_errors(),
        ["Workspace.Lost: cannot load `Scripts/Nowhere.mock`: no such file in the project"]
    );
}

#[test]
fn disable_enable_move_destroy_and_add_follow_the_rules() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    let server = w.service("ServerScriptService");
    let storage = w.service("ReplicatedStorage");
    let script = w.add("Script", workspace, "Door", Some(&recorder("door")));
    // Sources for scripts made after the start must be known before it.
    let late = w.create("Script", "Late", Some(&recorder("late")));
    let nested = w.create("Script", "Nested", Some(&recorder("nested")));
    let quiet = w.create("Script", "Quiet", Some(&recorder("quiet")));
    w.start(RunContext::Local);
    let baseline_watches = w.host.watch_count();
    assert_eq!(w.labels(), ["door"]);

    // Disabled: reaped at the next dispatch. Re-enabled: started again.
    w.set_enabled(script, false);
    w.frame();
    assert!(!w.host.is_running(script));
    assert_eq!(w.host.thread_count(), 0);
    w.set_enabled(script, true);
    w.frame();
    assert!(w.host.is_running(script));
    assert_eq!(w.labels(), ["door", "door"]);

    // Moving between services that run scripts keeps it running; moving out stops it; moving back restarts it.
    w.dm.set_parent(script, Some(server)).expect("move");
    w.frame();
    assert!(w.host.is_running(script));
    w.dm.set_parent(script, Some(storage)).expect("move");
    w.frame();
    assert!(!w.host.is_running(script));
    w.dm.set_parent(script, None).expect("move");
    w.frame();
    w.dm.set_parent(script, Some(workspace)).expect("move");
    w.frame();
    assert!(w.host.is_running(script));
    assert_eq!(w.labels(), ["door", "door", "door"]);

    // Added at runtime, alone or inside a model; added disabled, then enabled.
    w.dm.set_parent(late, Some(workspace)).expect("add");
    let model = w.create("Model", "Model", None);
    w.dm.set_parent(nested, Some(model)).expect("nest");
    w.dm.set_parent(model, Some(server)).expect("add");
    w.set_enabled(quiet, false);
    w.dm.set_parent(quiet, Some(workspace)).expect("add");
    w.frame();
    assert_eq!(w.labels(), ["door", "door", "door", "late", "nested"]);
    w.set_enabled(quiet, true);
    w.frame();
    assert_eq!(w.labels(), ["door", "door", "door", "late", "nested", "quiet"]);

    // Destroyed: reaped, and its watch goes.
    w.dm.destroy(script).expect("destroy");
    w.dm.destroy(model).expect("destroy");
    w.frame();
    assert!(!w.host.is_running(script));
    assert!(!w.host.is_running(nested));
    assert_eq!(w.host.thread_count(), 2, "Late and Quiet sleep on");
    assert_eq!(w.host.watch_count(), baseline_watches + 1);
    assert!(w.errors().is_empty(), "{:?}", w.errors());
}

#[test]
fn a_script_disabled_by_another_script_stops() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    let target = w.add(
        "Script",
        workspace,
        "Target",
        Some("record target\nwait 1\nrecord still running\n"),
    );
    w.add(
        "Script",
        workspace,
        "Switch",
        Some("call %Workspace FindFirstChild Target -> t\nset $t Enabled false\nrecord switched\n"),
    );
    w.start(RunContext::Local);
    w.frames(120);
    assert!(!w.host.is_running(target));
    assert_eq!(w.labels(), ["target", "switched"]);
}

#[test]
fn a_move_made_by_a_listener_takes_effect_at_the_following_dispatch() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    let storage = w.service("ReplicatedStorage");
    let late = w.add("Script", storage, "Late", Some(&recorder("late")));
    w.add(
        "Script",
        workspace,
        "Mover",
        Some(
            "once _ %Workspace ChildAdded call %ReplicatedStorage FindFirstChild Late -> t ; set $t Parent %Workspace\n\
             record mover\nwait 1000\n",
        ),
    );
    w.start(RunContext::Local);
    w.add("Folder", workspace, "Trigger", None);

    // The listener moves `Late` in during this dispatch; like its events, the move waits for the next one.
    w.host.dispatch_signals(&mut w.dm);
    assert_eq!(w.dm.parent(late).expect("parent"), Some(workspace));
    assert!(!w.host.is_running(late));
    w.host.dispatch_signals(&mut w.dm);
    assert!(w.host.is_running(late));
    assert_eq!(w.labels(), ["mover", "late"]);
    assert!(w.errors().is_empty(), "{:?}", w.errors());
}

#[test]
fn a_restarted_game_follows_scripts_again() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    let storage = w.service("ReplicatedStorage");
    let script = w.add("Script", storage, "Door", Some(&recorder("door")));
    let sources = std::mem::take(&mut w.sources);
    // More restarts than the data model has change consumers: each game gives its consumer back.
    for round in 0..2 * metallic_core::MAX_CONSUMERS {
        w.host
            .start_game(&mut w.dm, RunContext::Local, sources.clone())
            .expect("the game starts");
        w.dm.set_parent(script, Some(workspace)).expect("move in");
        w.frame();
        assert!(w.host.is_running(script), "round {round}");
        assert!(w.host.stop_game(&mut w.dm));
        // Stopped: moves are no longer followed, and nothing queued carries over to the next game.
        w.dm.set_parent(script, Some(storage)).expect("move out");
        w.frame();
        assert!(!w.host.is_running(script));
    }
    assert_eq!(w.labels().len(), 2 * metallic_core::MAX_CONSUMERS);
    assert_eq!(w.dm.connection_count(), 0);
}

#[test]
fn require_runs_a_module_once_and_caches_its_value() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    let module = w.add("ModuleScript", workspace, "Config", Some("record loading\nreturn 42\n"));
    let requirer = "call %Workspace FindFirstChild Config -> m\nrequire $m -> v\nrecord got $v\n";
    w.add("Script", workspace, "A", Some(requirer));
    w.add("Script", workspace, "B", Some(requirer));
    w.start(RunContext::Local);
    assert!(w.errors().is_empty(), "{:?}", w.errors());
    let records = w.log.borrow().records.clone();
    let got: Vec<&Vec<ScriptValue>> = records.iter().map(|(_, values)| values).collect();
    assert_eq!(
        got,
        [
            &vec![ScriptValue::from("loading")],
            &vec![ScriptValue::from("got"), ScriptValue::Number(42.0)],
            &vec![ScriptValue::from("got"), ScriptValue::Number(42.0)],
        ]
    );
    assert_eq!(records[0].0, module, "the module runs as itself");
    assert_eq!(w.log.borrow().modules_spawned, [module]);
    assert_eq!(w.log.borrow().held_modules, 1);

    // Stopping the game releases the value.
    w.host.stop_game(&mut w.dm);
    assert_eq!(w.log.borrow().held_modules, 0);
    assert_eq!(w.log.borrow().released_modules.len(), 1);
}

#[test]
fn a_module_that_yields_makes_every_requirer_wait() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    w.add(
        "ModuleScript",
        workspace,
        "Slow",
        Some("record loading\nwait 0.5\nrecord loaded\nreturn 7\n"),
    );
    let requirer =
        |label: &str| format!("call %Workspace FindFirstChild Slow -> m\nrequire $m -> v\nrecord {label} $v\n");
    w.add("Script", workspace, "A", Some(&requirer("A")));
    w.add("Script", workspace, "B", Some(&requirer("B")));
    w.start(RunContext::Local);
    assert_eq!(w.labels(), ["loading"]);
    w.frames(60);
    assert_eq!(w.labels(), ["loading", "loaded", "A", "B"]);
    assert_eq!(w.log.borrow().modules_spawned.len(), 1);
    assert!(w.errors().is_empty(), "{:?}", w.errors());
}

#[test]
fn module_errors_reach_every_requirer_with_the_modules_name_and_line() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    w.add(
        "ModuleScript",
        workspace,
        "Broken",
        Some("record loading\nerror boom\n"),
    );
    let requirer = "call %Workspace FindFirstChild Broken -> m\nrequire $m -> v\nrecord unreachable\n";
    w.add("Script", workspace, "A", Some(requirer));
    w.add("Script", workspace, "B", Some(requirer));
    let late = w.create("Script", "Late", Some(requirer));
    w.start(RunContext::Local);
    w.dm.set_parent(late, Some(workspace)).expect("add");
    w.frame();
    let expected = "Requested module experienced an error while loading: Workspace.Broken:2: boom";
    assert_eq!(
        w.located_errors(),
        [
            "Workspace.Broken:2: boom".to_owned(),
            format!("Workspace.A:2: {expected}"),
            format!("Workspace.B:2: {expected}"),
            format!("Workspace.Late:2: {expected}"),
        ]
    );
    assert_eq!(w.labels(), ["loading"], "the module ran once");
}

#[test]
fn a_module_must_return_exactly_one_value() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    w.add("ModuleScript", workspace, "None", Some("return\n"));
    w.add("ModuleScript", workspace, "Two", Some("return 1 2\n"));
    w.add(
        "Script",
        workspace,
        "A",
        Some("call %Workspace FindFirstChild None -> m\nrequire $m\n"),
    );
    w.add(
        "Script",
        workspace,
        "B",
        Some("call %Workspace FindFirstChild Two -> m\nrequire $m\n"),
    );
    w.start(RunContext::Local);
    let errors = w.located_errors();
    assert_eq!(errors.len(), 4, "{errors:?}");
    assert_eq!(
        errors[0],
        "Workspace.None: Module code did not return exactly one value"
    );
    assert!(errors[1].ends_with("Workspace.None: Module code did not return exactly one value"));
    assert_eq!(errors[2], "Workspace.Two: Module code did not return exactly one value");
}

#[test]
fn require_cycles_are_errors_naming_the_chain() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    w.add(
        "ModuleScript",
        workspace,
        "Ping",
        Some("call %Workspace FindFirstChild Pong -> m\nrequire $m\nreturn 1\n"),
    );
    w.add(
        "ModuleScript",
        workspace,
        "Pong",
        Some("call %Workspace FindFirstChild Ping -> m\nrequire $m\nreturn 2\n"),
    );
    w.add(
        "ModuleScript",
        workspace,
        "Narcissus",
        Some("require %script\nreturn 3\n"),
    );
    w.add(
        "Script",
        workspace,
        "Main",
        Some("call %Workspace FindFirstChild Ping -> m\nrequire $m\n"),
    );
    w.add(
        "Script",
        workspace,
        "Mirror",
        Some("call %Workspace FindFirstChild Narcissus -> m\nrequire $m\n"),
    );
    w.start(RunContext::Local);
    let errors = w.errors();
    let cycle =
        "Requested module was required recursively: Workspace.Pong requires Workspace.Ping requires Workspace.Pong";
    assert_eq!(errors.first().map(String::as_str), Some(cycle), "{errors:#?}");
    assert!(errors.iter().any(|e| {
        e.contains("Requested module was required recursively: Workspace.Narcissus requires Workspace.Narcissus")
    }));
    // Every requirer up the chain fails too, and nothing is left waiting.
    assert!(
        errors
            .iter()
            .any(|e| e.contains("Workspace.Ping:2: Requested module experienced"))
    );
    assert_eq!(w.host.thread_count(), 0);
}

#[test]
fn require_refuses_other_languages_and_non_modules() {
    let mut w = World::new();
    let (other, _) = MockLanguage::named("other", &["other"]);
    w.host.register_language(Box::new(other)).expect("register");
    let workspace = w.service("Workspace");
    let foreign = w.add("ModuleScript", workspace, "Foreign", None);
    w.dm.set_by_name(foreign, "Source", Value::String("Foreign.other".into()))
        .expect("Source");
    w.sources.insert("Foreign.other", "return 1\n");
    w.add(
        "Script",
        workspace,
        "A",
        Some("call %Workspace FindFirstChild Foreign -> m\nrequire $m\n"),
    );
    w.add("Script", workspace, "B", Some("require %Workspace\n"));
    w.start(RunContext::Local);
    assert_eq!(
        w.located_errors(),
        [
            "Workspace.A:2: cannot require Workspace.Foreign: it is written in another language, and require \
             between languages is not supported",
            "Workspace.B:1: Attempted to call require with invalid argument(s).",
        ]
    );
}

#[test]
fn bindable_function_runs_a_yielding_callback_and_returns_its_values() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    let function = w.add("BindableFunction", workspace, "Ask", None);
    // The caller starts first, so its Invoke waits until the server assigns the callback.
    w.add(
        "Script",
        workspace,
        "Caller",
        Some("call %Workspace FindFirstChild Ask -> f\ncall $f Invoke 20 1 -> r\nrecord got $r\n"),
    );
    let server = w.add(
        "Script",
        workspace,
        "Server",
        Some(
            "call %Workspace FindFirstChild Ask -> f\noninvoke $f OnInvoke record invoked @1 @2 ; wait 0.5 ; \
             return 21\nwait 1000\n",
        ),
    );
    w.start(RunContext::Local);
    assert_eq!(w.labels(), ["invoked"]);
    assert_eq!(w.host.callback_count(), 1);
    w.frames(60);
    assert_eq!(w.labels(), ["invoked", "got"]);
    let records = w.log.borrow().records.clone();
    assert_eq!(records[0].0, server, "the callback runs as the script that assigned it");
    let invoked = [
        ScriptValue::from("invoked"),
        ScriptValue::Number(20.0),
        ScriptValue::Number(1.0),
    ];
    assert_eq!(records[0].1, invoked);
    assert_eq!(records[1].1, [ScriptValue::from("got"), ScriptValue::Number(21.0)]);
    assert!(w.errors().is_empty(), "{:?}", w.errors());

    // Reaping the owner releases the function.
    let held = w.log.borrow().held_listeners;
    w.set_enabled(server, false);
    w.frame();
    assert_eq!(w.host.callback_count(), 0);
    assert_eq!(w.log.borrow().held_listeners, held - 1);
    assert!(w.dm.contains(function));
}

#[test]
fn callback_errors_are_raised_in_the_caller() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    w.add("BindableFunction", workspace, "Ask", None);
    w.add(
        "Script",
        workspace,
        "Server",
        Some("call %Workspace FindFirstChild Ask -> f\noninvoke $f OnInvoke error no answer\nwait 1000\n"),
    );
    w.add(
        "Script",
        workspace,
        "Caller",
        Some("call %Workspace FindFirstChild Ask -> f\ncall $f Invoke -> r\nrecord unreachable\n"),
    );
    w.start(RunContext::Local);
    assert_eq!(
        w.located_errors(),
        ["Workspace.Caller:2: Workspace.Server:2: no answer"]
    );
    assert!(w.labels().is_empty());
}

#[test]
fn replacing_clearing_and_destroying_release_callbacks() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    let function = w.add("BindableFunction", workspace, "Ask", None);
    w.add(
        "Script",
        workspace,
        "Server",
        Some(
            "call %Workspace FindFirstChild Ask -> f\noninvoke $f OnInvoke return 1\noninvoke $f OnInvoke return 2\n\
             clearcallback $f OnInvoke\nwait 1000\n",
        ),
    );
    w.add(
        "Script",
        workspace,
        "Caller",
        Some("call %Workspace FindFirstChild Ask -> f\ncall $f Invoke -> r\nrecord unreachable\n"),
    );
    w.start(RunContext::Local);
    assert_eq!(w.host.callback_count(), 0);
    assert_eq!(w.log.borrow().held_listeners, 0);
    assert_eq!(w.log.borrow().released_listeners.len(), 2);
    // The caller waits for a function; destroying the instance fails it instead of leaving it parked.
    assert_eq!(w.host.thread_count(), 2);
    w.dm.destroy(function).expect("destroy");
    w.frame();
    assert_eq!(
        w.located_errors(),
        ["Workspace.Caller:2: cannot invoke OnInvoke: the instance was destroyed"]
    );
    assert_eq!(w.host.thread_count(), 1, "the server sleeps on");
}

#[test]
fn listeners_of_destroyed_instances_are_released() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    let mut source = String::new();
    for i in 0..10 {
        source.push_str(&format!(
            "new Part -> p{i}\nset $p{i} Parent %Workspace\nconnect _ $p{i} Changed record changed\n"
        ));
    }
    source.push_str("wait 1000\n");
    w.add("Script", workspace, "Watcher", Some(&source));
    w.start(RunContext::Local);
    let baseline = w.dm.connection_count() - 10;
    assert_eq!(w.host.listener_count(), 10);
    assert_eq!(w.log.borrow().held_listeners, 10);

    let parts: Vec<Id> =
        w.dm.children(workspace)
            .expect("children")
            .filter(|&c| w.dm.class_name(c) == Ok("Part"))
            .collect();
    for part in parts {
        w.dm.destroy(part).expect("destroy");
    }
    w.frame();
    assert_eq!(w.host.listener_count(), 0);
    assert_eq!(w.log.borrow().held_listeners, 0);
    assert_eq!(w.log.borrow().released_listeners.len(), 10);
    assert_eq!(w.dm.connection_count(), baseline);
}

#[test]
fn a_wait_on_a_destroyed_instance_fails_instead_of_leaking() {
    let mut w = World::new();
    let workspace = w.service("Workspace");
    w.add(
        "Script",
        workspace,
        "Waiter",
        Some("new BindableEvent -> e\nset $e Name Ping\nset $e Parent %Workspace\nwaitsignal $e Event\nrecord woke\n"),
    );
    w.add(
        "Script",
        workspace,
        "Breaker",
        Some("call %Workspace FindFirstChild Ping -> e\ncall $e Destroy\n"),
    );
    w.start(RunContext::Local);
    assert_eq!(w.host.waiter_count(), 1);
    w.frame();
    assert_eq!(w.host.waiter_count(), 0);
    assert_eq!(w.host.thread_count(), 0);
    assert_eq!(
        w.located_errors(),
        ["Workspace.Waiter:4: the instance whose signal this thread was waiting on was destroyed"]
    );
    assert!(w.labels().is_empty());
}
