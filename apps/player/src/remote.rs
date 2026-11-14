//! A networked game's client (ADR 0011 §8): the world its server replicates, run in a Player.
//!
//! [`RemoteGame::hook`] is a [`Player`](crate::Player) frame hook: before each frame it brings in the server's
//! changes and sends the player's character's place, and it stops the Player when the game ends. Nothing here moves
//! the character or the camera: a game brings its own controller (#638).

use metallic_engine::metallic_core::DataModel;
use metallic_engine::metallic_net::play::GameClient;
use metallic_engine::metallic_script::OutputSink;
use metallic_engine::{Engine, MemorySources};

use crate::player::FrameHook;

/// A client's game: its connection to the server and the replicated world.
#[derive(Debug)]
pub struct RemoteGame {
    /// The connection and the replicated world.
    pub client: GameClient,
}

impl RemoteGame {
    /// The game of `client`.
    pub fn new(client: GameClient) -> Self {
        Self { client }
    }

    /// The sources of a client's scripts from the server's welcome.
    pub fn sources(sources: &[(String, String)]) -> MemorySources {
        let mut memory = MemorySources::new();
        for (member, source) in sources {
            memory.insert(member, source);
        }
        memory
    }

    /// One frame: the server's changes in, the character's place out.
    pub fn step(&mut self, dm: &mut DataModel) {
        self.client.tick(dm);
    }

    /// The frame hook that runs this game before each of a Player's frames.
    pub fn hook<O: OutputSink + 'static>(mut self) -> FrameHook<O> {
        Box::new(move |engine: &mut Engine<O>| {
            self.step(engine.data_model_mut());
            // The server ended the game, or the connection is gone: this client's game is over too.
            if let Some(why) = self.client.ended() {
                log::info!("play: the game ended: {why}");
                engine.stop();
            }
        })
    }
}
