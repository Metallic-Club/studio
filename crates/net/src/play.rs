//! Multiplayer playtesting (ADR 0011 §8): a game server replicating its world to clients, each client a player with
//! a character it moves.
//!
//! - **The server** runs the game (its `Script`s) and owns the world. Everything a save writes but the server-only
//!   services' subtrees replicates to every client through scene sync ([`Scope::Replicated`]), in the server's
//!   order. Clients never change the server's world but by moving their own character.
//! - **Joining**: `Hello` (versions, the registry's fingerprint, the player's name) → the server makes the player a
//!   character in the Workspace → `Welcome` (the player's number, their character, the client scripts' sources, and
//!   the whole replicated world) → live scene operations.
//! - **Characters**: a part per player, named for them. The client moves its own and sends where it is
//!   ([`Move`](ClientMsg::Move)) 20 times a second; the server checks the move, places the character and replicates
//!   it to the others; the client never takes the server's word for its own character's place.
//! - **Leaving or crashing**: the connection closes, the character goes, the others see it go. A client that crashes
//!   never takes the server down.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, DataModel, Id, Value};

use crate::codec::{CodecError, Reader, Writer};
use crate::collab::host::PeerId;
use crate::collab::scene::{self, ClientScene, HostOut, HostScene, Key, NetId, Record, SceneSync, Scope, To};
use crate::identity::{UserId, clean_name, unique_name};
use crate::transport::{Channel, Connection, Event, Limits, Listener, Waker};

/// The version of the play protocol.
pub const PLAY_PROTOCOL: u32 = 1;
/// How often a client sends where its character is, and the server collects its world.
pub const RATE: Duration = Duration::from_millis(50);
/// The most players a server takes.
pub const MAX_PLAYERS: usize = 32;
/// The farthest a character may move in one update, in metres: farther is a teleport the server refuses.
pub const MAX_STEP: f32 = 25.0;
/// The longest script source a client gets.
const MAX_SOURCE: usize = 4 << 20;

/// The characters' colours, in join order.
const COLOURS: [[f32; 3]; 6] = [
    [0.23, 0.51, 0.96],
    [0.06, 0.64, 0.50],
    [0.92, 0.70, 0.03],
    [0.94, 0.27, 0.27],
    [0.66, 0.33, 0.97],
    [0.93, 0.28, 0.60],
];

/// A message from a client.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMsg {
    /// The first message.
    Hello {
        /// [`PLAY_PROTOCOL`].
        protocol: u32,
        /// The engine version.
        engine: String,
        /// The class registry's fingerprint.
        fingerprint: u64,
        /// The player's name.
        name: String,
    },
    /// Where the player's character is now.
    Move([f32; 7]),
    /// The client's world drifted from the server's: it asks for all of it.
    Resync,
}

/// A message from the server.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerMsg {
    /// The player is in.
    Welcome(Welcome),
    /// They cannot join, and why.
    Refused(String),
    /// The game ended, and why.
    Ended(String),
    /// A scene message of `kind` (scene sync's).
    Scene(u8, Vec<u8>),
}

/// What a joining client gets.
#[derive(Clone, Debug, PartialEq)]
pub struct Welcome {
    /// The player's number.
    pub player: u32,
    /// Their name as the server has it (unique).
    pub name: String,
    /// Their character.
    pub character: NetId,
    /// The sources of the scripts a client runs (`LocalScript`s and `ModuleScript`s), by their file.
    pub sources: Vec<(String, String)>,
    /// The replicated world.
    pub world: Vec<Record>,
}

impl ClientMsg {
    /// The message as a frame's payload.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Hello {
                protocol,
                engine,
                fingerprint,
                name,
            } => {
                w.u8(1);
                w.u32(*protocol);
                w.str(engine);
                w.u64(*fingerprint);
                w.str(name);
            }
            Self::Resync => w.u8(3),
            Self::Move(parts) => {
                w.u8(2);
                for part in parts {
                    w.f32(*part);
                }
            }
        }
        w.finish()
    }

    /// Reads a frame's payload.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader::new(bytes);
        let msg = match r.u8()? {
            1 => Self::Hello {
                protocol: r.u32()?,
                engine: r.str("an engine version", 64)?.to_owned(),
                fingerprint: r.u64()?,
                name: r.str("a name", 256)?.to_owned(),
            },
            2 => {
                let mut parts = [0.0f32; 7];
                for part in &mut parts {
                    *part = r.f32()?;
                }
                Self::Move(parts)
            }
            3 => {
                r.finish()?;
                Self::Resync
            }
            tag => {
                return Err(CodecError::BadTag {
                    what: "a play message",
                    tag,
                });
            }
        };
        r.finish()?;
        Ok(msg)
    }
}

impl ServerMsg {
    /// The message as a frame's payload.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Welcome(welcome) => {
                w.u8(1);
                w.u32(welcome.player);
                w.str(&welcome.name);
                w.u64(welcome.character.0);
                w.len_of(welcome.sources.len());
                for (member, source) in &welcome.sources {
                    w.str(member);
                    w.str(source);
                }
                scene::write_records(&welcome.world, &mut w);
            }
            Self::Refused(reason) => {
                w.u8(2);
                w.str(reason);
            }
            Self::Ended(reason) => {
                w.u8(3);
                w.str(reason);
            }
            Self::Scene(kind, bytes) => {
                w.u8(4);
                w.u8(*kind);
                return [w.finish(), bytes.clone()].concat();
            }
        }
        w.finish()
    }

    /// Reads a frame's payload.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader::new(bytes);
        let msg = match r.u8()? {
            1 => {
                let player = r.u32()?;
                let name = r.str("a name", 256)?.to_owned();
                let character = NetId(r.u64()?);
                let count = r.count("sources", 100_000)?;
                let mut sources = Vec::with_capacity(count);
                for _ in 0..count {
                    let member = r.str("a script's path", crate::collab::protocol::MAX_PATH)?;
                    if !crate::collab::protocol::is_safe_member_path(member) {
                        return Err(CodecError::Invalid("an unsafe script path"));
                    }
                    sources.push((member.to_owned(), r.str("a source", MAX_SOURCE)?.to_owned()));
                }
                let world = scene::read_records(&mut r)?;
                Self::Welcome(Welcome {
                    player,
                    name,
                    character,
                    sources,
                    world,
                })
            }
            2 => Self::Refused(r.str("a reason", 2048)?.to_owned()),
            3 => Self::Ended(r.str("a reason", 2048)?.to_owned()),
            4 => {
                let kind = r.u8()?;
                return Ok(Self::Scene(kind, bytes[2..].to_vec()));
            }
            tag => {
                return Err(CodecError::BadTag {
                    what: "a play message",
                    tag,
                });
            }
        };
        r.finish()?;
        Ok(msg)
    }
}

/// One connection to the server.
struct Peer {
    connection: Connection,
    /// The player, once in: their number, name and character.
    player: Option<(u32, String, Id)>,
}

/// A game server over TCP: the world it replicates, the players and their characters.
pub struct GameServer {
    listener: Listener,
    peers: BTreeMap<PeerId, Peer>,
    next_peer: u64,
    next_player: u32,
    scene: HostScene,
    sources: Vec<(String, String)>,
    engine: String,
    fingerprint: u64,
    last_collect: Instant,
    last_digest: Instant,
    ended: bool,
}

impl std::fmt::Debug for GameServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameServer")
            .field("port", &self.port())
            .field("players", &self.players())
            .finish_non_exhaustive()
    }
}

/// The transport limits a server reads clients with.
fn server_limits() -> Limits {
    Limits::default()
}

/// The transport limits a client reads its server with: the world comes at the start.
fn client_limits() -> Limits {
    Limits {
        max_frame: 64 << 20,
        bytes_per_second: 256 << 20,
        ..Limits::default()
    }
}

impl GameServer {
    /// A server of the world in `dm` on `listener`, giving clients `sources` (their scripts) and taking only clients
    /// of `engine` with the registry `fingerprint`.
    pub fn new(
        listener: Listener,
        dm: &mut DataModel,
        sources: Vec<(String, String)>,
        engine: &str,
        fingerprint: u64,
    ) -> Result<Self, String> {
        let sync = SceneSync::with_scope(dm, [], UserId::HOST, Scope::Replicated)?;
        let now = Instant::now();
        Ok(Self {
            listener,
            peers: BTreeMap::new(),
            next_peer: 1,
            next_player: 1,
            scene: HostScene::from_sync(sync),
            sources,
            engine: engine.to_owned(),
            fingerprint,
            last_collect: now,
            last_digest: now,
            ended: false,
        })
    }

    /// Listens on `port` (0 for a free one) on every interface, waking the owner through `waker` (see
    /// [`GameServer::new`]).
    pub fn start(
        port: u16,
        dm: &mut DataModel,
        sources: Vec<(String, String)>,
        engine: &str,
        fingerprint: u64,
        waker: Waker,
    ) -> Result<Self, String> {
        let listener = Listener::bind(port, server_limits(), waker)?;
        Self::new(listener, dm, sources, engine, fingerprint)
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.listener.local_addr().port()
    }

    /// The players in the game: number and name.
    pub fn players(&self) -> Vec<(u32, String)> {
        self.peers
            .values()
            .filter_map(|peer| peer.player.as_ref().map(|(n, name, _)| (*n, name.clone())))
            .collect()
    }

    fn send(&self, peer: PeerId, msg: &ServerMsg) {
        if let Some(p) = self.peers.get(&peer) {
            p.connection.send(Channel::Reliable, msg.encode());
        }
    }

    fn send_scene(&self, out: HostOut) {
        for (to, kind, bytes) in out.messages {
            let msg = ServerMsg::Scene(kind, bytes).encode();
            for p in self.peers.values() {
                let mine = match (to, &p.player) {
                    (_, None) => false,
                    (To::All { except }, Some((n, _, _))) => except != Some(UserId(*n + 1)),
                    (To::One(user), Some((n, _, _))) => user == UserId(*n + 1),
                };
                if mine {
                    p.connection.send(Channel::Reliable, msg.clone());
                }
            }
        }
    }

    fn collect(&mut self, dm: &mut DataModel) {
        let mut out = HostOut::default();
        self.scene.collect(dm, &mut out);
        self.send_scene(out);
    }

    fn close(&mut self, peer: PeerId, msg: ServerMsg) {
        self.send(peer, &msg);
        if let Some(mut p) = self.peers.remove(&peer) {
            p.connection.close();
        }
    }

    /// A client's hello: their character is made, and they get the world.
    fn hello(&mut self, dm: &mut DataModel, peer: PeerId, protocol: u32, engine: &str, fingerprint: u64, name: &str) {
        if protocol != PLAY_PROTOCOL || engine != self.engine || fingerprint != self.fingerprint {
            let reason = format!("this Player ({engine}) cannot join a game served by {}", self.engine);
            self.close(peer, ServerMsg::Refused(reason));
            return;
        }
        if self.players().len() >= MAX_PLAYERS {
            self.close(
                peer,
                ServerMsg::Refused(format!("the game is full ({MAX_PLAYERS} players)")),
            );
            return;
        }
        let names: Vec<String> = self.players().into_iter().map(|(_, n)| n).collect();
        let cleaned = clean_name(name);
        let wanted = if cleaned.is_empty() {
            "Player".to_owned()
        } else {
            cleaned
        };
        let name = unique_name(&wanted, names.iter().map(String::as_str));
        let number = self.next_player;
        self.next_player += 1;
        let character = match make_character(dm, &name, number) {
            Ok(character) => character,
            Err(error) => {
                self.close(
                    peer,
                    ServerMsg::Refused(format!("the server could not place you: {error}")),
                );
                return;
            }
        };
        // Everyone else hears of the character first, in order; then the newcomer gets the world with it.
        self.collect(dm);
        let world = self.scene.sync.state(dm);
        let Some(character_net) = self.scene.sync.net(character) else {
            self.close(peer, ServerMsg::Refused("the server could not place you".to_owned()));
            return;
        };
        if let Some(p) = self.peers.get_mut(&peer) {
            p.player = Some((number, name.clone(), character));
        }
        self.send(
            peer,
            &ServerMsg::Welcome(Welcome {
                player: number,
                name: name.clone(),
                character: character_net,
                sources: self.sources.clone(),
                world,
            }),
        );
        log::info!("play: {name} joined (player {number})");
    }

    /// A client moved its character to `parts`.
    fn moved(&mut self, dm: &mut DataModel, peer: PeerId, parts: [f32; 7]) {
        let Some((_, _, character)) = self.peers.get(&peer).and_then(|p| p.player.clone()) else {
            return;
        };
        let Some(to) = CFrame::from_array(parts) else {
            return;
        };
        let Ok(prop) = dm.prop_id(character, "CFrame") else {
            return;
        };
        let from = match dm.get_ref(character, prop) {
            Ok(Value::CFrame(from)) => *from,
            _ => return,
        };
        // A step too long is refused: the character stays, and the client is put back by the next update.
        if from.position().distance(to.position()) > MAX_STEP {
            return;
        }
        let _ = dm.set(character, prop, Value::CFrame(to));
    }

    fn left(&mut self, dm: &mut DataModel, peer: PeerId, why: &str) {
        if let Some(p) = self.peers.remove(&peer)
            && let Some((number, name, character)) = p.player
        {
            log::info!("play: {name} (player {number}) left: {why}");
            let _ = dm.destroy(character);
        }
    }

    /// Once a server frame (after the game's own): new clients, their messages, the world's changes to everyone.
    pub fn tick(&mut self, dm: &mut DataModel) {
        while let Some(connection) = self.listener.accept() {
            let peer = PeerId(self.next_peer);
            self.next_peer += 1;
            if self.ended {
                let mut connection = connection;
                connection.send(
                    Channel::Reliable,
                    ServerMsg::Ended("the game ended".to_owned()).encode(),
                );
                connection.close();
                continue;
            }
            self.peers.insert(
                peer,
                Peer {
                    connection,
                    player: None,
                },
            );
        }
        let mut gone = Vec::new();
        let mut messages = Vec::new();
        for (&peer, p) in &mut self.peers {
            while let Some(event) = p.connection.poll() {
                match event {
                    Event::Message(_, bytes) => messages.push((peer, bytes)),
                    Event::Closed(reason) => {
                        gone.push((peer, reason.to_string()));
                        break;
                    }
                    Event::Connected(_) => {}
                }
            }
        }
        for (peer, bytes) in messages {
            match ClientMsg::decode(&bytes) {
                Ok(ClientMsg::Hello {
                    protocol,
                    engine,
                    fingerprint,
                    name,
                }) if self.peers.get(&peer).is_some_and(|p| p.player.is_none()) => {
                    self.hello(dm, peer, protocol, &engine, fingerprint, &name);
                }
                Ok(ClientMsg::Move(parts)) => self.moved(dm, peer, parts),
                Ok(ClientMsg::Resync) => {
                    if let Some((number, _, _)) = self.peers.get(&peer).and_then(|p| p.player.clone()) {
                        let mut out = HostOut::default();
                        let _ = self
                            .scene
                            .receive(dm, UserId(number + 1), scene::kind::RESYNC_ASK, &[], &mut out);
                        self.send_scene(out);
                    }
                }
                // A second hello, or garbage: the client is dropped.
                _ => gone.push((peer, "it broke the protocol".to_owned())),
            }
        }
        for (peer, why) in gone {
            self.left(dm, peer, &why);
        }
        let now = Instant::now();
        if now.duration_since(self.last_collect) >= RATE {
            self.last_collect = now;
            self.collect(dm);
        }
        if now.duration_since(self.last_digest) >= Duration::from_secs(10) {
            self.last_digest = now;
            let mut out = HostOut::default();
            self.scene.digest(dm, &mut out);
            self.send_scene(out);
        }
    }

    /// Ends the game: tells every client why and closes their connections.
    pub fn end(&mut self, reason: &str) {
        self.ended = true;
        let peers: Vec<PeerId> = self.peers.keys().copied().collect();
        for peer in peers {
            self.close(peer, ServerMsg::Ended(reason.to_owned()));
        }
    }
}

/// Makes player `number`'s character, `name`, in `dm`'s Workspace: an anchored part in the player's colour, placed
/// beside the others'.
fn make_character(dm: &mut DataModel, name: &str, number: u32) -> Result<Id, String> {
    let registry = Arc::clone(dm.registry());
    let part_class = registry.class_id("Part").ok_or("no Part class")?;
    let workspace = registry
        .class_id("Workspace")
        .and_then(|class| dm.service(class))
        .ok_or("no Workspace")?;
    let part = dm.create(part_class).map_err(|e| e.to_string())?;
    let set = |dm: &mut DataModel, prop: &str, value: Value| {
        if let Ok(prop) = dm.prop_id(part, prop) {
            let _ = dm.set(part, prop, value);
        }
    };
    dm.set_name(part, name.into()).map_err(|e| e.to_string())?;
    set(dm, "Size", Value::Vector3(Vec3::new(2.0, 5.0, 1.0)));
    let [r, g, b] = COLOURS[(number as usize - 1) % COLOURS.len()];
    set(dm, "Color", Value::Color3(Color3 { r, g, b }));
    set(dm, "Anchored", Value::Bool(true));
    let x = (number as f32 - 1.0) * 6.0 - 6.0;
    set(dm, "CFrame", Value::CFrame(CFrame::new(Vec3::new(x, 6.0, 10.0))));
    dm.set_parent(part, Some(workspace)).map_err(|e| e.to_string())?;
    Ok(part)
}

/// A client's game: its connection, the replicated world and its own character.
pub struct GameClient {
    connection: Connection,
    scene: ClientScene,
    /// The player's number.
    pub player: u32,
    /// The player's name as the server has it.
    pub name: String,
    character: Option<Id>,
    sent: Option<[f32; 7]>,
    last_move: Instant,
    ended: Option<String>,
}

impl std::fmt::Debug for GameClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameClient")
            .field("player", &self.player)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A connection to a game server under way, before its welcome.
pub struct Connecting {
    connection: Connection,
    started: Instant,
    timeout: Duration,
}

impl std::fmt::Debug for Connecting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connecting").finish_non_exhaustive()
    }
}

impl Connecting {
    /// Starts connecting to the game at `host:port` as `name`, waking the owner through `waker`.
    pub fn start(host: &str, port: u16, name: &str, engine: &str, fingerprint: u64, waker: Waker) -> Self {
        let connection = Connection::connect(host, port, client_limits(), waker);
        connection.send(
            Channel::Reliable,
            ClientMsg::Hello {
                protocol: PLAY_PROTOCOL,
                engine: engine.to_owned(),
                fingerprint,
                name: name.to_owned(),
            }
            .encode(),
        );
        Self {
            connection,
            started: Instant::now(),
            timeout: Duration::from_secs(15),
        }
    }

    /// The welcome, once it came (`Ok(None)` before); an error says why the game cannot be joined.
    pub fn poll(&mut self) -> Result<Option<Welcome>, String> {
        while let Some(event) = self.connection.poll() {
            match event {
                Event::Connected(_) => {}
                Event::Message(_, bytes) => match ServerMsg::decode(&bytes) {
                    Ok(ServerMsg::Welcome(welcome)) => return Ok(Some(welcome)),
                    Ok(ServerMsg::Refused(reason) | ServerMsg::Ended(reason)) => return Err(reason),
                    Ok(ServerMsg::Scene(..)) => {}
                    Err(error) => return Err(format!("the server sent something Player cannot read ({error})")),
                },
                Event::Closed(reason) => return Err(reason.to_string()),
            }
        }
        if self.started.elapsed() > self.timeout {
            return Err("the server did not answer in time".to_owned());
        }
        Ok(None)
    }

    /// Waits for the welcome, at most the timeout. Blocks: for a client that has nothing to draw yet.
    pub fn wait(mut self) -> Result<(Welcome, Self), String> {
        loop {
            if let Some(welcome) = self.poll()? {
                return Ok((welcome, self));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The client's game in `dm` (a new game with its services): the world of `welcome` put in it, its character its
    /// own.
    pub fn into_client(self, dm: &mut DataModel, welcome: &Welcome) -> Result<GameClient, String> {
        let me = UserId(welcome.player + 1);
        let sync = SceneSync::with_scope(dm, [], me, Scope::Replicated)?;
        let mut scene = ClientScene::from_sync(sync);
        scene.sync.reconcile(dm, &welcome.world, |_| false);
        let character = scene.sync.local(welcome.character);
        if let Some(character) = character
            && let Ok(prop) = dm.prop_id(character, "CFrame")
        {
            scene.own(Key::Prop(welcome.character, prop.raw()));
        }
        Ok(GameClient {
            connection: self.connection,
            scene,
            player: welcome.player,
            name: welcome.name.clone(),
            character,
            sent: None,
            last_move: Instant::now(),
            ended: None,
        })
    }
}

impl GameClient {
    /// The player's character in the client's world.
    pub fn character(&self) -> Option<Id> {
        self.character
    }

    /// Why the game ended for this client, once it did (the server ended it, or the connection was lost).
    pub fn ended(&self) -> Option<&str> {
        self.ended.as_deref()
    }

    /// Once a client frame: the server's changes applied, and the character's place sent.
    pub fn tick(&mut self, dm: &mut DataModel) {
        if self.ended.is_some() {
            return;
        }
        while let Some(event) = self.connection.poll() {
            match event {
                Event::Connected(_) => {}
                Event::Message(_, bytes) => match ServerMsg::decode(&bytes) {
                    Ok(ServerMsg::Scene(kind, bytes)) => match self.scene.receive(dm, kind, &bytes) {
                        // The world drifted: the client asks for it again (the one scene message it sends).
                        Ok(Some(_)) => self.connection.send(Channel::Reliable, ClientMsg::Resync.encode()),
                        Ok(None) => {}
                        Err(error) => {
                            self.ended = Some(format!("the server sent something Player cannot read ({error})"));
                            return;
                        }
                    },
                    Ok(ServerMsg::Ended(reason)) => {
                        self.ended = Some(reason);
                        return;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        self.ended = Some(format!("the server sent something Player cannot read ({error})"));
                        return;
                    }
                },
                Event::Closed(reason) => {
                    self.ended = Some(reason.to_string());
                    return;
                }
            }
        }
        // The client's own changes to its world are its own: the server's world is the server's.
        let _ = self.scene.sync.collect(dm);
        if self.last_move.elapsed() >= RATE
            && let Some(character) = self.character
            && let Ok(Value::CFrame(at)) = dm.get_by_name(character, "CFrame")
        {
            self.last_move = Instant::now();
            let parts = at.to_array();
            if self.sent != Some(parts) {
                self.sent = Some(parts);
                self.connection.send(Channel::Reliable, ClientMsg::Move(parts).encode());
            }
        }
    }

    /// Leaves the game.
    pub fn leave(&mut self) {
        self.connection.close();
        self.ended.get_or_insert_with(|| "left".to_owned());
    }
}

#[cfg(test)]
mod tests;
