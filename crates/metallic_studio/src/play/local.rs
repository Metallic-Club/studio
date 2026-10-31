//! Own a disposable local Game without sharing the authored world's state.

use std::rc::Rc;
use std::sync::Arc;

use metallic_core::World;
use metallic_runtime::{FrameInput, FrameOutput, Game, RuntimeError};

use super::{PlaySession, PlayStatus};
use crate::output::{OutputSink, ScriptOutput, Tag};

// Also guards partial initialization: Game::start may fail after creating VMs.
struct ActiveGame(Game);

impl Drop for ActiveGame {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A fresh local Game over an independent snapshot of the authored world.
///
/// Stop and Drop shut down the real VMs and discard the play world. Starting again means
/// constructing a new session, rather than restarting its mutated Game. No mode or action is
/// registered automatically. This foundation supplies no viewport or input-event wiring.
///
/// Copying costs O(world). The existing [`super::PlayStart`] seam already supplies a clean copy
/// with parked history nodes removed; starting from that borrowed copy incurs a second copy.
/// Callers with an [`crate::editor::EditorState`] should supply its `clean_world()` snapshot.
/// Startup's StarterGui copying still uses the independently tracked recursive `clone_subtree`
/// implementation (#92/#275); this wrapper does not establish arbitrary-depth startup safety.
pub struct LocalPlaySession {
    game: Option<ActiveGame>,
}

impl LocalPlaySession {
    /// Copies `snapshot` and starts a fresh Game without reading a scene from disk.
    ///
    /// The Game guard exists before fallible initialization. Errors retain their runtime type;
    /// any partially started VMs are stopped on error or unwinding. Panics are not recovered.
    /// Both server and client messages use the combined `Local Play` tag because the runtime
    /// supplies one output sink for both VMs. Their text and severity are preserved.
    pub fn start(
        name: impl Into<String>,
        snapshot: &World,
        output: OutputSink,
    ) -> Result<Self, RuntimeError> {
        let output = Rc::new(ScriptOutput {
            sink: output,
            tag: Tag::Other(Arc::from("Local Play")),
        });
        let mut game = ActiveGame(Game::from_world(name.into(), snapshot.deep_clone(), output));
        game.0.start()?;
        Ok(Self { game: Some(game) })
    }

    /// True until this session has been stopped.
    pub fn is_running(&self) -> bool {
        self.game.as_ref().is_some_and(|game| game.0.is_running())
    }

    /// Advances one caller frame with already evaluated input; returns None after Stop.
    ///
    /// This uses Game's existing fixed ticks and elapsed-time clamp. The PlaySession `update`
    /// adapter advances with neutral input; use either that adapter or this method once per
    /// caller frame, not both. It does not poll devices or dispatch raw input actions.
    pub fn frame(&mut self, input: &FrameInput, dt: f64) -> Option<FrameOutput> {
        self.game.as_mut().map(|game| game.0.frame(input, dt))
    }

    /// Lends the play world read-only for the duration of `inspect`.
    ///
    /// Returns None after Stop or if the world cannot currently be borrowed. No mutable World,
    /// Game or shared world handle is exposed, and a borrowed world reference cannot escape.
    pub fn inspect_world<R>(&self, inspect: impl FnOnce(&World) -> R) -> Option<R> {
        let world = self.game.as_ref()?.0.world();
        let world = world.try_borrow().ok()?;
        Some(inspect(&world))
    }

    /// Stops VMs and releases the play Game. Repeated calls do nothing.
    pub fn stop(&mut self) {
        drop(self.game.take());
    }
}

impl PlaySession for LocalPlaySession {
    fn update(&mut self, dt: f64) {
        let _ = self.frame(&FrameInput::default(), dt);
    }

    fn status(&self) -> PlayStatus {
        if self.is_running() {
            PlayStatus::Running("Local Play".into())
        } else {
            PlayStatus::Ended("Stopped".into())
        }
    }

    fn stop(mut self: Box<Self>) -> Result<(), String> {
        LocalPlaySession::stop(&mut self);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputLog;

    fn pending_session() -> LocalPlaySession {
        let mut world = World::new_with_services();
        let parent = world.root("ServerScriptService").unwrap();
        let script = world.create("Script", Some(parent)).unwrap();
        world
            .set(
                script,
                "Source",
                metallic_core::Value::String(
                    "task.delay(10, function() print('late') end); \
                     game:GetService('RunService').Heartbeat:Connect(function() end)"
                        .into(),
                ),
            )
            .unwrap();
        let log = OutputLog::new(8);
        LocalPlaySession::start("Ownership", &world, log.sink()).unwrap()
    }

    #[test]
    fn stop_and_drop_release_pending_vm_world_ownership() {
        for explicit_stop in [false, true] {
            let mut session = pending_session();
            let world = session.game.as_ref().unwrap().0.world();
            let weak = Rc::downgrade(&world);
            drop(world);
            if explicit_stop {
                session.stop();
                assert!(weak.upgrade().is_none());
            }
            drop(session);
            assert!(weak.upgrade().is_none());
        }
    }

    #[test]
    fn caller_unwind_drops_the_guard_without_a_recovery_contract() {
        let session = pending_session();
        let world = session.game.as_ref().unwrap().0.world();
        let weak = Rc::downgrade(&world);
        drop(world);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _session = session;
            panic!("test caller unwind");
        }));
        assert!(result.is_err());
        assert!(weak.upgrade().is_none());
    }
}
