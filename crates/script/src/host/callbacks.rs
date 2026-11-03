//! Callback members (`BindableFunction.OnInvoke`): the function assigned per (instance, callback), and running it
//! for a thread that yielded in a method that invokes it (`Invoke`).

use std::collections::{HashMap, HashSet};

use metallic_core::{DataModel, Id, ListenerKey, ScriptValue};

use super::{Invocation, ScriptHost, Watch, WatchRecord, Work};
use crate::scheduler::Slot;
use crate::{LanguageId, OutputSink, ThreadRef, YieldReason};

/// A callback member of one instance.
pub(crate) type CallbackSlot = (Id, &'static str);

/// The function assigned to a callback member: its key in the backend that holds it, and the script that assigned
/// it (which owns it: reaping the script releases it).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CallbackRecord {
    pub(crate) listener: ListenerKey,
    pub(crate) script: Id,
    pub(crate) language: LanguageId,
}

/// One assignment made during a call, handled by the host after it.
#[derive(Debug)]
struct Change {
    target: Id,
    callback: &'static str,
    /// The function the assignment replaced, for the host to release.
    replaced: Option<CallbackRecord>,
}

/// A thread waiting in `Invoke` for a function to be assigned.
#[derive(Debug)]
struct Waiting {
    /// Its [`Slot::Callback`] ticket.
    ticket: u64,
    thread: ThreadRef,
    args: Vec<ScriptValue>,
}

/// Every assigned callback function, and the threads waiting for one.
#[derive(Debug, Default)]
pub(crate) struct Callbacks {
    map: HashMap<CallbackSlot, CallbackRecord>,
    waiting: HashMap<CallbackSlot, Vec<Waiting>>,
    changes: Vec<Change>,
    /// Instances with a function assigned or an `Invoke` waiting, and the key of the host's `Destroying` watch,
    /// whose dropped connection tells the host they are gone.
    watched: HashMap<Id, ListenerKey>,
}

impl Callbacks {
    /// Assigns (`Some`) or clears (`None`) the function of `callback` on `target`. The host handles the change
    /// after the call.
    pub(crate) fn assign(&mut self, target: Id, callback: &'static str, record: Option<CallbackRecord>) {
        let replaced = match record {
            Some(record) => self.map.insert((target, callback), record),
            None => self.map.remove(&(target, callback)),
        };
        self.changes.push(Change {
            target,
            callback,
            replaced,
        });
    }
}

/// The error of an `Invoke` whose instance was destroyed.
fn destroyed_message(callback: &str) -> String {
    format!("cannot invoke {callback}: the instance was destroyed")
}

impl<O: OutputSink> ScriptHost<O> {
    /// How many callback members have a function assigned, across every script.
    pub fn callback_count(&self) -> usize {
        self.callbacks.map.len()
    }

    /// Handles the assignments the last call made: releases replaced functions, watches the instances, and lets
    /// threads waiting in `Invoke` run.
    pub(super) fn apply_callback_changes(&mut self, dm: &mut DataModel) {
        let changes = std::mem::take(&mut self.callbacks.changes);
        for change in changes {
            if let Some(old) = change.replaced
                && let Some(backend) = self.languages.get_mut(old.language.0)
            {
                backend.release_listener(old.listener);
            }
            if !self.callbacks.map.contains_key(&(change.target, change.callback)) {
                continue;
            }
            self.watch_callback_target(dm, change.target);
            let waiting = self
                .callbacks
                .waiting
                .remove(&(change.target, change.callback))
                .unwrap_or_default();
            for waiter in waiting {
                if self.take_slot(waiter.thread, Slot::Callback(waiter.ticket)).is_some() {
                    self.work.push_back(Work::Invoke {
                        thread: waiter.thread,
                        target: change.target,
                        callback: change.callback,
                        args: waiter.args,
                    });
                }
            }
        }
    }

    /// Handles a thread that yielded in a method invoking `callback` of `target`: runs the assigned function in a
    /// new thread of its owner with `args` (the invoker resumes with its results), or parks the invoker until a
    /// function is assigned.
    pub(super) fn invoke(
        &mut self,
        dm: &mut DataModel,
        thread: ThreadRef,
        target: Id,
        callback: &'static str,
        args: Vec<ScriptValue>,
    ) {
        let Some(invoker) = self.yielded_for(
            thread,
            |r| matches!(r, YieldReason::InvokeCallback { target: t, .. } if *t == target),
        ) else {
            return;
        };
        if !dm.contains(target) {
            self.resume_error_now(dm, invoker, thread, &destroyed_message(callback));
            return;
        }
        let Some(function) = self.callbacks.map.get(&(target, callback)).copied() else {
            let ticket = self.sched.ticket();
            if let Some(record) = self.threads.get_mut(&thread) {
                record.slot = Slot::Callback(ticket);
            }
            self.callbacks
                .waiting
                .entry((target, callback))
                .or_default()
                .push(Waiting { ticket, thread, args });
            self.watch_callback_target(dm, target);
            return;
        };
        let spawned = self.call(dm, function.script, |backend, ctx| {
            backend.spawn_listener(function.listener, ctx)
        });
        match spawned {
            Some(Ok(worker)) => {
                let worker = self.adopt(function.script, function.language, worker);
                let ticket = self.sched.ticket();
                if let Some(record) = self.threads.get_mut(&thread) {
                    record.slot = Slot::Invoke(ticket);
                }
                self.invocations.insert(
                    worker,
                    Invocation {
                        invoker: thread,
                        ticket,
                        callback,
                    },
                );
                self.resume_now(dm, function.script, worker, &args);
            }
            Some(Err(failure)) => self.resume_error_now(dm, invoker, thread, &failure.to_string()),
            None => {
                let message = format!("the script that assigned {callback} is not running");
                self.resume_error_now(dm, invoker, thread, &message);
            }
        }
    }

    /// Watches `target`'s destruction, once, so its functions are released and its waiters woken when it goes.
    fn watch_callback_target(&mut self, dm: &mut DataModel, target: Id) {
        if self.callbacks.watched.contains_key(&target) {
            return;
        }
        let Ok(signal) = dm.signal_id(target, "Destroying") else {
            return;
        };
        let key = self.listeners.next_key();
        if let Ok(connection) = dm.connect(target, signal, key) {
            self.watches.insert(
                key,
                WatchRecord {
                    watch: Watch::Destroying(target),
                    connection,
                },
            );
            self.callbacks.watched.insert(target, key);
        }
    }

    /// `target` was destroyed: its functions are released and the threads waiting to invoke them fail.
    pub(super) fn callback_target_destroyed(&mut self, target: Id) {
        self.callbacks.watched.remove(&target);
        let gone: Vec<CallbackSlot> = self.callbacks.map.keys().filter(|s| s.0 == target).copied().collect();
        for slot in gone {
            if let Some(record) = self.callbacks.map.remove(&slot)
                && let Some(backend) = self.languages.get_mut(record.language.0)
            {
                backend.release_listener(record.listener);
            }
        }
        let waiting: Vec<CallbackSlot> = self
            .callbacks
            .waiting
            .keys()
            .filter(|s| s.0 == target)
            .copied()
            .collect();
        for slot in waiting {
            for waiter in self.callbacks.waiting.remove(&slot).unwrap_or_default() {
                self.work.push_back(Work::Fail {
                    thread: waiter.thread,
                    slot: Slot::Callback(waiter.ticket),
                    message: destroyed_message(slot.1),
                });
            }
        }
    }

    /// Releases the functions `script` assigned that are still its own (a later assignment by another script
    /// replaced the rest).
    pub(super) fn release_callbacks_of(&mut self, script: Id, slots: &HashSet<CallbackSlot>) {
        for slot in slots {
            let owned = self.callbacks.map.get(slot).is_some_and(|r| r.script == script);
            if owned
                && let Some(record) = self.callbacks.map.remove(slot)
                && let Some(backend) = self.languages.get_mut(record.language.0)
            {
                backend.release_listener(record.listener);
            }
        }
    }

    /// Forgets every function and waiter (the game stopped; their scripts are reaped).
    pub(super) fn clear_callbacks(&mut self) {
        for (_, record) in self.callbacks.map.drain() {
            if let Some(backend) = self.languages.get_mut(record.language.0) {
                backend.release_listener(record.listener);
            }
        }
        self.callbacks.waiting.clear();
        self.callbacks.watched.clear();
        self.callbacks.changes.clear();
    }
}
