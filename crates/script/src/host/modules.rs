//! `require`: the module cache, loading a `ModuleScript`, waiting for a load, and cycles.

use std::collections::HashMap;

use metallic_core::{DataModel, Id, Value};

use super::{RecordKind, ScriptHost, Work};
use crate::context::{MODULE_SCRIPT, cross_language_message};
use crate::scheduler::Slot;
use crate::{LanguageId, ModuleKey, OutputSink, OutputSource, ThreadRef, YieldReason};

/// The error of a `require` while no source loader is set.
const NO_LOADER: &str = "require needs the game's sources: start the game with ScriptHost::start_game";

/// Where one `ModuleScript` stands.
#[derive(Debug)]
pub(crate) enum ModuleState {
    /// Its thread is running (or waiting); `waiters` resume when it returns.
    Loading {
        /// The thread running the module's code.
        thread: ThreadRef,
        /// The key its backend will keep the value under.
        key: ModuleKey,
        /// The waiting `require`s, in the order they came.
        waiters: Vec<ThreadRef>,
    },
    /// It returned; its backend keeps the value under `key`.
    Loaded {
        /// The value's key in the backend.
        key: ModuleKey,
        /// The backend that ran it: only scripts of the same language may require it.
        language: LanguageId,
    },
    /// It failed; every `require` of it raises `message`.
    Failed {
        /// The error, naming the module and line.
        message: String,
    },
}

/// The module cache: one entry per `ModuleScript` required in this host (one context).
#[derive(Debug, Default)]
pub(crate) struct Modules {
    pub(crate) map: HashMap<Id, ModuleState>,
    next: u64,
}

impl Modules {
    /// Where `module` stands, or `None` when nothing required it yet.
    pub(crate) fn state(&self, module: Id) -> Option<&ModuleState> {
        self.map.get(&module)
    }

    /// A key no module of this host has used.
    fn next_key(&mut self) -> ModuleKey {
        self.next += 1;
        ModuleKey(self.next)
    }
}

impl<O: OutputSink> ScriptHost<O> {
    /// Handles a thread that yielded in `require(module)`: resumes it when the module is loaded or failed, makes it
    /// wait for a load in progress (unless that would be a cycle), or starts the load.
    pub(super) fn require(&mut self, dm: &mut DataModel, thread: ThreadRef, module: Id) {
        let Some(requester) = self.yielded_for(
            thread,
            |r| matches!(r, YieldReason::Require { module: m } if *m == module),
        ) else {
            return;
        };
        match self.modules.map.get(&module) {
            Some(ModuleState::Loaded { .. }) => {
                // The backend asks again and gets the value (or the cross-language error).
                self.resume_now(dm, requester, thread, &[]);
            }
            Some(ModuleState::Failed { message }) => {
                let message = message.clone();
                self.resume_error_now(dm, requester, thread, &message);
            }
            Some(ModuleState::Loading { .. }) => {
                if let Some(cycle) = self.require_cycle(thread, module) {
                    self.resume_error_now(dm, requester, thread, &cycle);
                    return;
                }
                if let Some(ModuleState::Loading { waiters, .. }) = self.modules.map.get_mut(&module) {
                    waiters.push(thread);
                }
                if let Some(record) = self.threads.get_mut(&thread) {
                    record.slot = Slot::Require(module);
                }
            }
            None => match self.start_module(dm, thread, module) {
                Ok(loader) => {
                    if let Some(record) = self.threads.get_mut(&thread) {
                        record.slot = Slot::Require(module);
                    }
                    self.resume_now(dm, module, loader, &[]);
                }
                Err(message) => self.resume_error_now(dm, requester, thread, &message),
            },
        }
    }

    /// Starts loading `module` for the thread `requester`: reads and compiles its source, and creates its thread
    /// (not resumed yet), with `requester` as the first waiter. A module that cannot be loaded is cached as failed;
    /// either way the error for the requester is returned.
    fn start_module(&mut self, dm: &mut DataModel, requester: ThreadRef, module: Id) -> Result<ThreadRef, String> {
        let name = dm.full_name(module).map_err(|e| e.to_string())?;
        let is_module = dm
            .registry()
            .class_id(MODULE_SCRIPT)
            .is_some_and(|class| dm.is_a(module, class).unwrap_or(false));
        let path = match dm.get_by_name(module, "Source") {
            Ok(Value::String(path)) if is_module => path,
            _ => return Err(crate::context::INVALID_REQUIRE.to_owned()),
        };
        if path.is_empty() {
            return Err(self.module_failed(module, &format!("{name} has no Source")));
        }
        let language = match self.language_for_path(&path) {
            Ok(language) => language,
            Err(error) => return Err(self.module_failed(module, &format!("{name}: {error}"))),
        };
        if language != requester.language {
            return Err(cross_language_message(&name));
        }
        let Some(loader) = self.loader.as_ref() else {
            return Err(NO_LOADER.to_owned());
        };
        let source = match loader.load(&path) {
            Ok(source) => source,
            Err(error) => return Err(self.module_failed(module, &format!("{name}: {error}"))),
        };
        let chunk = match self.chunk(language, &path, &source) {
            Ok(chunk) => chunk,
            Err(error) => {
                let from = OutputSource {
                    script: &name,
                    chunk: &path,
                    line: Some(error.line),
                };
                self.output.error(&from, &error.to_string());
                return Err(self.module_failed(module, &error.to_string()));
            }
        };
        // A record left from an earlier run of the instance (by `run_script`) goes first.
        self.reap_record(dm, module);
        self.insert_record(module, language, name.into(), &path, RecordKind::Module);
        let key = self.modules.next_key();
        let thread = match self.call(dm, module, |backend, ctx| backend.spawn_module(chunk, key, ctx)) {
            Some(Ok(thread)) => thread,
            Some(Err(failure)) => {
                self.report(module, &failure);
                let at = self.failure_position(module, &failure);
                return Err(self.module_failed(module, &at));
            }
            None => return Err(self.module_failed(module, "the module could not start")),
        };
        let loader = self.adopt(module, language, thread);
        self.loaders.insert(loader, module);
        self.modules.map.insert(
            module,
            ModuleState::Loading {
                thread: loader,
                key,
                waiters: vec![requester],
            },
        );
        Ok(loader)
    }

    /// Caches `module` as failed with `error` (which names the module and line) and returns what its requirers
    /// raise. Threads waiting for it are woken with that error.
    pub(super) fn module_failed(&mut self, module: Id, error: &str) -> String {
        let message = format!("Requested module experienced an error while loading: {error}");
        let previous = self.modules.map.insert(
            module,
            ModuleState::Failed {
                message: message.clone(),
            },
        );
        if let Some(ModuleState::Loading { waiters, .. }) = previous {
            for thread in waiters {
                self.work.push_back(Work::Fail {
                    thread,
                    slot: Slot::Require(module),
                    message: message.clone(),
                });
            }
        }
        message
    }

    /// The module's thread returned and its backend keeps the value: the threads waiting for it resume.
    pub(super) fn module_loaded(&mut self, module: Id, language: LanguageId) {
        let Some(ModuleState::Loading { key, .. }) = self.modules.map.get(&module) else {
            return;
        };
        let key = *key;
        let previous = self.modules.map.insert(module, ModuleState::Loaded { key, language });
        if let Some(ModuleState::Loading { waiters, .. }) = previous {
            for thread in waiters {
                self.work.push_back(Work::Resume {
                    thread,
                    slot: Slot::Require(module),
                    args: Vec::new(),
                });
            }
        }
    }

    /// The error for `requester` requiring `module` when that closes a cycle: `requester` is loading a module that
    /// `module`'s load waits for, directly or through other modules. `None` when there is no cycle.
    fn require_cycle(&self, requester: ThreadRef, module: Id) -> Option<String> {
        let loading = *self.loaders.get(&requester)?;
        let mut chain = vec![loading, module];
        let mut current = module;
        for _ in 0..=self.modules.map.len() {
            let Some(ModuleState::Loading { thread: loader, .. }) = self.modules.map.get(&current) else {
                return None;
            };
            if *loader == requester {
                let names: Vec<String> = chain.iter().map(|&m| self.module_name(m)).collect();
                return Some(format!(
                    "Requested module was required recursively: {}",
                    names.join(" requires ")
                ));
            }
            let Slot::Require(next) = self.threads.get(loader)?.slot else {
                return None;
            };
            chain.push(next);
            current = next;
        }
        None
    }

    /// The module's full name as it was when it started loading.
    pub(super) fn module_name(&self, module: Id) -> String {
        self.scripts
            .get(&module)
            .map_or_else(|| "a module".to_owned(), |record| record.name.to_string())
    }

    /// Forgets every module and has the backends drop the values they keep.
    pub(super) fn clear_modules(&mut self) {
        for (_, state) in self.modules.map.drain() {
            if let ModuleState::Loaded { key, language } = state
                && let Some(backend) = self.languages.get_mut(language.0)
            {
                backend.release_module(key);
            }
        }
        self.loaders.clear();
    }
}
