//! A game with a script host running the Luau backend, for the integration tests.
#![allow(dead_code, reason = "each test file uses part of the helpers")]

use metallic_core::{DataModel, Id, Value};
use metallic_luau::LuauLanguage;
use metallic_script::{MemorySources, OutputLevel, Phase, RunContext, ScriptHost, ScriptRun};

/// A new game and a host with the Luau backend.
pub(crate) struct Game {
    pub(crate) dm: DataModel,
    pub(crate) host: ScriptHost,
    /// Game time, advanced by [`Game::frame`].
    pub(crate) now: f64,
    /// The sources of the instances made by [`Game::add`], handed to the host by [`Game::start`].
    pub(crate) sources: MemorySources,
}

impl Game {
    pub(crate) fn new() -> Self {
        let dm = metallic_classes::new_game().unwrap_or_else(|e| panic!("{e}"));
        let mut host = ScriptHost::new();
        let luau = LuauLanguage::new(dm.registry().clone()).expect("Luau VM");
        host.register_language(Box::new(luau)).expect("register");
        Self {
            dm,
            host,
            now: 0.0,
            sources: MemorySources::new(),
        }
    }

    /// A new `class` instance named `name` under `parent` (none when `None`); with `source`, its `Source` is
    /// `Scripts/<name>.luau` holding that text.
    pub(crate) fn add(&mut self, class: &str, parent: Option<Id>, name: &str, source: Option<&str>) -> Id {
        let class = self.dm.registry().class_id(class).expect("class");
        let id = self.dm.create(class).expect("create");
        self.dm.set_name(id, name.into()).expect("name");
        if let Some(source) = source {
            let path = format!("Scripts/{name}.luau");
            self.sources.insert(&path, source);
            self.dm
                .set_by_name(id, "Source", Value::String(path.into()))
                .expect("Source");
        }
        if parent.is_some() {
            self.dm.set_parent(id, parent).expect("parent");
        }
        id
    }

    /// Starts the game in one local context, with the sources of every instance [`Game::add`] made.
    pub(crate) fn start(&mut self) {
        let sources = std::mem::take(&mut self.sources);
        self.host
            .start_game(&mut self.dm, RunContext::Local, sources)
            .expect("the game starts");
    }

    pub(crate) fn service(&self, class: &str) -> Id {
        let class = self.dm.registry().class_id(class).expect("class");
        self.dm.service(class).expect("service")
    }

    /// A `Script` named `name` in `Workspace`.
    pub(crate) fn script(&mut self, name: &str) -> Id {
        let class = self.dm.registry().class_id("Script").expect("Script");
        let id = self.dm.create(class).expect("create");
        self.dm.set_name(id, name.into()).expect("name");
        let workspace = self.service("Workspace");
        self.dm.set_parent(id, Some(workspace)).expect("parent");
        id
    }

    /// Runs `source` as a new script `Workspace.<name>`, from `Scripts/<name>.luau`.
    pub(crate) fn run(&mut self, name: &str, source: &str) -> ScriptRun {
        let script = self.script(name);
        self.host
            .run_script(&mut self.dm, script, &format!("Scripts/{name}.luau"), source)
            .unwrap_or_else(|e| panic!("{name} did not start: {e}"))
    }

    /// One frame of `dt` seconds: every phase, in order.
    pub(crate) fn frame(&mut self, dt: f64) {
        self.now += dt;
        for phase in Phase::FRAME {
            self.host.step(&mut self.dm, phase, self.now);
        }
    }

    /// Every printed line, in order.
    pub(crate) fn prints(&self) -> Vec<String> {
        self.lines(OutputLevel::Print)
    }

    /// Every warning, in order.
    pub(crate) fn warnings(&self) -> Vec<String> {
        self.lines(OutputLevel::Warn)
    }

    /// Every error as `script:line: message`, in order.
    pub(crate) fn errors(&self) -> Vec<String> {
        self.host
            .output()
            .of_level(OutputLevel::Error)
            .map(|l| {
                // The host appends the traceback after a newline; the tests compare the message.
                let message = l.message.lines().next().unwrap_or_default();
                match l.line {
                    Some(line) => format!("{}:{line}: {message}", l.script),
                    None => format!("{}: {message}", l.script),
                }
            })
            .collect()
    }

    fn lines(&self, level: OutputLevel) -> Vec<String> {
        self.host.output().of_level(level).map(|l| l.message.clone()).collect()
    }

    /// Fails with the errors if there are any.
    pub(crate) fn assert_no_errors(&self) {
        let errors = self.errors();
        assert!(errors.is_empty(), "script errors: {errors:#?}");
    }
}
