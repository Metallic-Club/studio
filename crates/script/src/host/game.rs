//! Running script instances: `start_game`, and following each script's lifecycle (enabled, moved, destroyed,
//! added) through host-internal connections.

use std::collections::HashMap;

use metallic_core::{ClassId, DataModel, Id, ListenerKey, PropId, ScriptValue};

use super::{HostError, RecordKind, ScriptHost, Watch, WatchRecord};
use crate::{OutputSink, OutputSource, RunContext, SourceLoader};

/// The running game: what runs, where, and which scripts are watched.
#[derive(Debug)]
pub(crate) struct Game {
    context: RunContext,
    /// The class whose instances start ([`RunContext::script_class`]).
    class: ClassId,
    /// `BaseScript.Enabled` (one id in every subclass).
    enabled: PropId,
    /// `SourceContainer.Source`.
    source: PropId,
    /// The services that run scripts, in tree order.
    services: Vec<Id>,
    /// Scripts inside those services, and the key of the watch on their `Enabled`.
    scripts: HashMap<Id, ListenerKey>,
}

impl Game {
    /// Whether `id` is inside one of the services that run scripts.
    fn in_service(&self, dm: &DataModel, id: Id) -> bool {
        let mut current = dm.parent(id).ok().flatten();
        while let Some(ancestor) = current {
            if self.services.contains(&ancestor) {
                return true;
            }
            current = dm.parent(ancestor).ok().flatten();
        }
        false
    }
}

impl<O: OutputSink> ScriptHost<O> {
    /// Starts the game: runs every enabled script of `context`'s class inside the services flagged `runs_scripts`,
    /// reading each script's `Source` through `loader`, and from then on follows the scripts' lifecycle (see the
    /// crate docs). Scripts start in tree order: services in the order they sit under the root (`Workspace` before
    /// `ServerScriptService`), and within a service depth-first, a parent before its children and children in
    /// order.
    ///
    /// A script that fails to load, compile or start is reported to the output and does not stop the others. Fails
    /// only when a game is already running or the registry lacks the script classes.
    pub fn start_game(
        &mut self,
        dm: &mut DataModel,
        context: RunContext,
        loader: impl SourceLoader + 'static,
    ) -> Result<(), HostError> {
        if self.game.is_some() {
            return Err(HostError::GameRunning);
        }
        let registry = dm.registry();
        let class = context
            .class_in(registry)
            .ok_or(HostError::MissingClass(context.script_class()))?;
        let base = registry
            .class_id("BaseScript")
            .ok_or(HostError::MissingClass("BaseScript"))?;
        let enabled = registry
            .prop_id(base, "Enabled")
            .ok_or(HostError::MissingClass("BaseScript"))?;
        let container = registry
            .class_id("SourceContainer")
            .ok_or(HostError::MissingClass("SourceContainer"))?;
        let source = registry
            .prop_id(container, "Source")
            .ok_or(HostError::MissingClass("SourceContainer"))?;
        let services: Vec<Id> = match dm.root() {
            Some(root) => dm
                .children(root)?
                .filter(|&s| {
                    dm.class_of(s)
                        .ok()
                        .and_then(|c| registry.class(c))
                        .is_some_and(|info| info.runs_scripts())
                })
                .collect(),
            None => Vec::new(),
        };
        self.loader = Some(Box::new(loader));
        for &service in &services {
            self.watch_service(dm, service, "DescendantAdded", Watch::Added);
            self.watch_service(dm, service, "DescendantRemoving", Watch::Removing);
        }
        let mut scripts = Vec::new();
        for &service in &services {
            scripts.extend(dm.descendants(service)?.filter(|&d| dm.is_a(d, class).unwrap_or(false)));
        }
        self.game = Some(Game {
            context,
            class,
            enabled,
            source,
            services,
            scripts: HashMap::new(),
        });
        for script in scripts {
            self.reconcile(dm, script);
        }
        self.flush_work(dm);
        Ok(())
    }

    /// Stops the game: reaps every script and module, releases every callback function and module value, and drops
    /// the host's watches. Returns `false` when no game was running.
    pub fn stop_game(&mut self, dm: &mut DataModel) -> bool {
        if self.game.take().is_none() {
            return false;
        }
        let scripts: Vec<Id> = self.scripts.keys().copied().collect();
        for script in scripts {
            self.reap_record(dm, script);
        }
        self.work.clear();
        for (_, watch) in self.watches.drain() {
            dm.disconnect(watch.connection);
        }
        self.clear_callbacks();
        self.clear_modules();
        self.invocations.clear();
        self.dirty.clear();
        self.loader = None;
        true
    }

    /// The context the running game was started with, or `None` when no game runs.
    pub fn run_context(&self) -> Option<RunContext> {
        self.game.as_ref().map(|game| game.context)
    }

    /// How many host-internal connections exist: the watches on the services that run scripts, on each script's
    /// `Enabled`, and on instances with a callback function.
    pub fn watch_count(&self) -> usize {
        self.watches.len()
    }

    /// Connects a host-internal listener to `signal` of `service`.
    fn watch_service(&mut self, dm: &mut DataModel, service: Id, signal: &str, watch: Watch) {
        let Ok(signal) = dm.signal_id(service, signal) else {
            return;
        };
        let key = self.listeners.next_key();
        if let Ok(connection) = dm.connect(service, signal, key) {
            self.watches.insert(key, WatchRecord { watch, connection });
        }
    }

    /// A watch delivered an event: the instances it concerns are reconciled after the drain.
    pub(super) fn on_watch(&mut self, watch: Watch, args: &[ScriptValue]) {
        match watch {
            Watch::Added | Watch::Removing => {
                if let Some(ScriptValue::Instance(id)) = args.first() {
                    self.dirty.push(*id);
                }
            }
            Watch::Enabled(script) => self.dirty.push(script),
            // Only its dropped connection matters.
            Watch::Destroying(_) => {}
        }
    }

    /// A watch's connection was dropped with its instance.
    pub(super) fn watch_dropped(&mut self, key: ListenerKey, watch: Watch) {
        match watch {
            Watch::Enabled(script) => {
                if let Some(game) = &mut self.game
                    && game.scripts.get(&script) == Some(&key)
                {
                    game.scripts.remove(&script);
                }
                self.dirty.push(script);
            }
            Watch::Destroying(target) => self.callback_target_destroyed(target),
            Watch::Added | Watch::Removing => {}
        }
    }

    /// Reconciles every instance marked since the last call, in the order they were marked.
    pub(super) fn reconcile_dirty(&mut self, dm: &mut DataModel) {
        if self.dirty.is_empty() {
            return;
        }
        let mut dirty = std::mem::take(&mut self.dirty);
        let mut seen = std::collections::HashSet::with_capacity(dirty.len());
        for id in dirty.drain(..) {
            if seen.insert(id) {
                self.reconcile(dm, id);
            }
        }
        // Reconciling may have marked more (a script that destroys another while starting); keep them for later.
        dirty.append(&mut self.dirty);
        self.dirty = dirty;
    }

    /// Brings one instance in line with the rules: a script of the context's class inside a service that runs
    /// scripts is watched, and runs exactly when it is enabled. Anything else that runs as a script is reaped.
    fn reconcile(&mut self, dm: &mut DataModel, id: Id) {
        let Some(game) = self.game.as_ref() else {
            return;
        };
        let alive = dm.contains(id);
        let script = alive && dm.is_a(id, game.class).unwrap_or(false);
        let watched = game.scripts.contains_key(&id);
        let running = self.scripts.get(&id).is_some_and(|r| r.kind == RecordKind::Script);
        if alive && !script {
            // Not a script this context runs (a part, a module, a LocalScript on the server...).
            return;
        }
        let in_service = script && game.in_service(dm, id);
        let enabled = in_service && dm.bool(id, game.enabled).unwrap_or(false);
        if in_service && !watched {
            self.watch_enabled(dm, id);
        } else if !in_service && watched {
            self.unwatch_enabled(dm, id);
        }
        if enabled && !running {
            self.start_script(dm, id);
        } else if !enabled && running {
            self.reap_record(dm, id);
        }
    }

    /// Watches the `Enabled` property of `script`.
    fn watch_enabled(&mut self, dm: &mut DataModel, script: Id) {
        let Some(game) = self.game.as_ref() else {
            return;
        };
        let key = self.listeners.next_key();
        if let Ok(connection) = dm.connect_property_changed(script, game.enabled, key) {
            self.watches.insert(
                key,
                WatchRecord {
                    watch: Watch::Enabled(script),
                    connection,
                },
            );
            if let Some(game) = &mut self.game {
                game.scripts.insert(script, key);
            }
        }
    }

    /// Stops watching `script`.
    fn unwatch_enabled(&mut self, dm: &mut DataModel, script: Id) {
        let Some(key) = self.game.as_mut().and_then(|game| game.scripts.remove(&script)) else {
            return;
        };
        if let Some(watch) = self.watches.remove(&key) {
            dm.disconnect(watch.connection);
        }
    }

    /// Runs `script` from its `Source`. An empty `Source` runs nothing; a file that cannot be read is reported.
    fn start_script(&mut self, dm: &mut DataModel, script: Id) {
        let Some(game) = self.game.as_ref() else {
            return;
        };
        let path = match dm.string(script, game.source) {
            Ok(path) if !path.is_empty() => path.to_owned(),
            _ => return,
        };
        let Some(loader) = self.loader.as_ref() else {
            return;
        };
        match loader.load(&path) {
            Ok(source) => {
                // Failures are reported by `run_script`.
                let _ = self.run_script(dm, script, &path, &source);
            }
            Err(error) => {
                let name = dm.full_name(script).unwrap_or_default();
                let from = OutputSource {
                    script: &name,
                    chunk: &path,
                    line: None,
                };
                self.output.error(&from, &error.to_string());
            }
        }
    }
}
