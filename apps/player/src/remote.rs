//! A networked game's client (ADR 0011 §8): the world its server replicates, and the player's own character moved
//! with the keyboard and followed by the camera.
//!
//! [`RemoteGame::hook`] is a [`Player`](crate::Player) frame hook: before each frame it brings in the server's
//! changes, moves the character (W, A, S, D; Space and Shift up and down; Q and E turn the view), places the camera
//! behind it, and sends the character's place to the server, which passes it on to the other players.

use std::time::Instant;

use metallic_engine::metallic_classes::input::key_code;
use metallic_engine::metallic_classes::services::user_input_service::HELD_KEYS;
use metallic_engine::metallic_core::glam::{Quat, Vec3};
use metallic_engine::metallic_core::{CFrame, DataModel, Value};
use metallic_engine::metallic_net::play::GameClient;
use metallic_engine::metallic_script::OutputSink;
use metallic_engine::{Engine, MemorySources};

use crate::player::FrameHook;

/// How fast a character walks, in metres a second.
pub const SPEED: f32 = 16.0;
/// How fast Q and E turn the view, in radians a second.
pub const TURN: f32 = 2.0;

/// Whether the key named `key` (an `Enum.KeyCode` item) is held in `dm`'s `UserInputService`.
pub fn held(dm: &DataModel, key: &str) -> bool {
    let (Some(index), Some(class)) = (key_code::index(key), dm.registry().class_id("UserInputService")) else {
        return false;
    };
    let (Some(service), Some(word)) = (dm.service(class), HELD_KEYS.get(index / 64)) else {
        return false;
    };
    matches!(dm.get_by_name(service, word), Ok(Value::Int(bits)) if (bits as u64) & (1 << (index % 64)) != 0)
}

/// A client's game and its character's controls.
pub struct RemoteGame {
    /// The connection and the replicated world.
    pub client: GameClient,
    yaw: f32,
    last: Option<Instant>,
    /// Whether the keyboard moves the character (off while the host's window is not the game's).
    pub controls: bool,
}

impl std::fmt::Debug for RemoteGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteGame")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

impl RemoteGame {
    /// The game of `client`.
    pub fn new(client: GameClient) -> Self {
        Self {
            client,
            yaw: 0.0,
            last: None,
            controls: true,
        }
    }

    /// The sources of a client's scripts from the server's welcome.
    pub fn sources(sources: &[(String, String)]) -> MemorySources {
        let mut memory = MemorySources::new();
        for (member, source) in sources {
            memory.insert(member, source);
        }
        memory
    }

    /// One frame: the server's changes in, the character moved and followed, its place out.
    pub fn step(&mut self, dm: &mut DataModel) {
        let now = Instant::now();
        let dt = self
            .last
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.1));
        self.last = Some(now);
        self.client.tick(dm);
        let Some(character) = self.client.character() else {
            return;
        };
        let Ok(Value::CFrame(at)) = dm.get_by_name(character, "CFrame") else {
            return;
        };
        let mut position = at.position();
        if self.controls {
            let axis =
                |plus: &str, minus: &str| f32::from(u8::from(held(dm, plus))) - f32::from(u8::from(held(dm, minus)));
            self.yaw += axis("Q", "E") * TURN * dt;
            let turn = Quat::from_rotation_y(self.yaw);
            let forward = turn * Vec3::NEG_Z;
            let right = turn * Vec3::X;
            let walk = forward * axis("W", "S") + right * axis("D", "A");
            let walk = if walk.length_squared() > 0.0 {
                walk.normalize()
            } else {
                walk
            };
            let rise = Vec3::Y * axis("Space", "LeftShift");
            position += (walk + rise) * SPEED * dt;
            position.y = position.y.max(2.5);
            let moved = CFrame::from_parts(position, turn);
            if moved != at {
                let _ = dm.set_by_name(character, "CFrame", Value::CFrame(moved));
            }
        }
        // The camera behind and above the character, looking at it.
        let turn = Quat::from_rotation_y(self.yaw);
        let eye = position + turn * Vec3::new(0.0, 6.0, 14.0);
        if let Some(camera) = current_camera(dm) {
            let _ = dm.set_by_name(
                camera,
                "CFrame",
                Value::CFrame(CFrame::look_at(eye, position + Vec3::Y * 2.0, Vec3::Y)),
            );
        }
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

/// The instance `Workspace.CurrentCamera` points at.
fn current_camera(dm: &DataModel) -> Option<metallic_engine::metallic_core::Id> {
    let workspace = dm.service(dm.registry().class_id("Workspace")?)?;
    match dm.get_by_name(workspace, "CurrentCamera").ok()? {
        Value::Ref(camera) => camera,
        _ => None,
    }
}
