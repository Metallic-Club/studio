//! Multiplayer playtesting (ADR 0011 §8): Team Create Play and Multiple Clients, on top of Play in Studio
//! (ADR 0010).
//!
//! - **The server** is a Player of the scene's copy in the server's context (its `Script`s run), with a
//!   [`GameServer`] replicating its world. The panels and the Server view show and edit it, as they do the game in
//!   Play Solo.
//! - **Team Create Play**: the host's Studio runs the server and a client of its own, shown in the Game tab; every
//!   participant's Studio joins it as a player of its own ([`EditorState::play_joined`]).
//! - **Multiple Clients**: the server, and N client processes (the Player app, `--connect`), Player1 to PlayerN,
//!   that join it by themselves. Stopping ends the server, closes the clients and waits for them to go; a client that
//!   crashes is only noted, never the editor's concern.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use metallic_engine::metallic_classes;
use metallic_engine::metallic_net::play::{Connecting, GameServer, Welcome};
use metallic_engine::metallic_net::registry_fingerprint;
use metallic_engine::metallic_net::transport::Waker;
use metallic_engine::metallic_project::{load_scene, save_scene};
use metallic_engine::metallic_render::settings::GraphicsSettings;
use metallic_engine::{RunContext, RunState, project_sources};
use metallic_player::Player;
use metallic_player::remote::RemoteGame;

use super::play::{GameView, PlaySession};
use super::{EditorState, History, Selection};
use crate::logging::{LogBuffer, ScriptOutput};
use crate::panels::explorer::ExplorerState;

/// How a session plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayKind {
    /// Team Create Play, hosted here: the server and this Studio's own player.
    TeamCreate,
    /// Multiple Clients: the server and `clients` Player processes.
    Clients(u8),
    /// A player of a Team Create Play hosted by someone else.
    Joined,
}

/// A client process of Multiple Clients.
pub trait ClientProcess {
    /// Its player's name.
    fn name(&self) -> &str;
    /// Whether it still runs.
    fn running(&mut self) -> bool;
    /// Ends it and waits for it to go.
    fn stop(&mut self);
}

/// Starts client `name` of the game served on localhost `port`.
pub type Launcher = Arc<dyn Fn(&str, u16) -> Result<Box<dyn ClientProcess>, String> + Send + Sync>;

/// A Player process.
struct PlayerProcess {
    name: String,
    child: Child,
}

impl ClientProcess for PlayerProcess {
    fn name(&self) -> &str {
        &self.name
    }

    fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn stop(&mut self) {
        // The process is ours: ended as it is, and waited for so nothing is left behind.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PlayerProcess {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.stop();
        }
    }
}

/// Where the Player app is: `METALLIC_PLAYER` if set, else `metallic_player` beside Studio's own executable.
pub fn player_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("METALLIC_PLAYER") {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    let name = format!("metallic_player{}", std::env::consts::EXE_SUFFIX);
    let beside = exe.with_file_name(&name);
    if beside.is_file() {
        return Some(beside);
    }
    // A test or a tool in `target/<profile>/deps` has the Player one folder up.
    exe.parent()?
        .parent()
        .map(|up| up.join(name))
        .filter(|path| path.is_file())
}

/// The launcher of Player processes: `metallic_player --connect 127.0.0.1:<port> --name <name>`.
pub fn player_launcher() -> Launcher {
    Arc::new(|name: &str, port: u16| {
        let path = player_path().ok_or_else(|| {
            "the Player app was not found beside Studio (build it with `cargo build -p metallic_player`, or set \
             METALLIC_PLAYER to it)"
                .to_owned()
        })?;
        let child = Command::new(&path)
            .args(["--connect", &format!("127.0.0.1:{port}"), "--name", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .map_err(|error| format!("{} could not start: {error}", path.display()))?;
        log::info!("play: started {name} (process {})", child.id());
        Ok(Box::new(PlayerProcess {
            name: name.to_owned(),
            child,
        }) as Box<dyn ClientProcess>)
    })
}

/// The network side of a session.
pub struct Net {
    /// How it plays.
    pub kind: PlayKind,
    /// The game server, when this Studio runs it (the session's player is the server then).
    pub server: Option<GameServer>,
    /// The Game tab's game, when it is a client of the session's server.
    pub client: Option<Player<ScriptOutput>>,
    /// Multiple Clients' processes.
    pub processes: Vec<Box<dyn ClientProcess>>,
    /// Why the game ended for this Studio (its server went), once it did.
    pub ended: Option<String>,
}

impl std::fmt::Debug for Net {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Net")
            .field("kind", &self.kind)
            .field("server", &self.server)
            .field("processes", &self.processes.len())
            .finish_non_exhaustive()
    }
}

impl Net {
    /// One frame after the session's own: the server's clients served, this Studio's client run, the processes
    /// watched.
    pub fn tick(&mut self, server_dm: &mut metallic_engine::metallic_core::DataModel, dt: Duration) {
        if let Some(server) = self.server.as_mut() {
            server.tick(server_dm);
        }
        if let Some(client) = self.client.as_mut() {
            client.frame(dt);
            if client.engine().state() == RunState::Stopped && self.ended.is_none() {
                self.ended = Some("the game's server went away".to_owned());
            }
        }
        self.processes.retain_mut(|process| {
            let running = process.running();
            if !running {
                log::warn!("play: {} closed", process.name());
            }
            running
        });
    }

    /// Ends the server (its clients are told), closes the processes and waits for them.
    pub fn stop(&mut self) {
        if let Some(server) = self.server.as_mut() {
            server.end("The game was stopped in Studio.");
        }
        for process in &mut self.processes {
            process.stop();
        }
        self.processes.clear();
        if let Some(client) = self.client.as_mut() {
            client.engine_mut().stop();
        }
    }
}

/// The client scripts' sources (LocalScripts and ModuleScripts), by their file, as a server gives them.
fn client_sources(editor: &mut EditorState) -> Vec<(String, String)> {
    let mut sources = Vec::new();
    for (script, member) in crate::code::script_files(&editor.dm) {
        let client = crate::viewport::pick::is_a(&editor.dm, script, "LocalScript")
            || crate::viewport::pick::is_a(&editor.dm, script, "ModuleScript");
        if client && !sources.iter().any(|(m, _)| *m == member) {
            let text = editor.scripts.text(&mut editor.project, &member).clone();
            sources.push((member, text));
        }
    }
    sources
}

/// This Studio's player of the game at `welcome`, a client in its own data model, its scripts from the server.
fn client_player(
    welcome: &Welcome,
    connecting: Connecting,
    graphics: GraphicsSettings,
    log: Arc<LogBuffer>,
) -> Result<Player<ScriptOutput>, String> {
    let mut game = metallic_classes::new_game().map_err(|error| error.to_string())?;
    let client = connecting.into_client(&mut game, welcome)?;
    let sources = RemoteGame::sources(&welcome.sources);
    let mut player = Player::hosted_in(RunContext::Client, game, sources, graphics, ScriptOutput::new(log))
        .map_err(|error| error.to_string())?;
    player.set_frame_hook(RemoteGame::new(client).hook());
    Ok(player)
}

fn fingerprint() -> Result<u64, String> {
    metallic_classes::shared_registry()
        .map(|registry| registry_fingerprint(&registry))
        .map_err(|error| error.to_string())
}

impl EditorState {
    /// Starts a networked session of `kind` (Team Create Play or Multiple Clients): copies the scene into a server
    /// Player listening on `port` (0: any), then this Studio's own player as `name` (Team Create Play) or the client
    /// processes `launcher` starts (Multiple Clients). Returns the server's port.
    #[allow(clippy::too_many_arguments)]
    pub fn play_server(
        &mut self,
        kind: PlayKind,
        port: u16,
        name: &str,
        graphics: GraphicsSettings,
        log: Arc<LogBuffer>,
        number: u64,
        clear_output: bool,
        launcher: Option<&Launcher>,
        waker: Waker,
    ) -> Result<u16, String> {
        if self.play.is_some() {
            return Err("A game is running already.".to_owned());
        }
        self.output.update();
        if clear_output {
            self.output.clear();
        }
        let saved = save_scene(&self.dm, &mut self.scene_ids).map_err(|error| format!("the scene: {error}"))?;
        let mut game = metallic_classes::new_game().map_err(|error| error.to_string())?;
        let loaded = load_scene(&mut game, &saved.text).map_err(|error| format!("the scene: {error}"))?;
        let sources = project_sources(&mut self.project).map_err(|error| format!("the scripts: {error}"))?;
        let fingerprint = fingerprint()?;
        let client_sources = client_sources(self);
        let mut player = Player::hosted_in(
            RunContext::Server,
            game,
            sources,
            graphics.clone(),
            ScriptOutput::new(Arc::clone(&log)),
        )
        .map_err(|error| error.to_string())?;
        let mut server = GameServer::start(
            port,
            player.engine_mut().data_model_mut(),
            client_sources,
            crate::team::ENGINE,
            fingerprint,
            waker.clone(),
        )?;
        let port = server.port();
        log::info!("play: serving the game on port {port}");
        let mut net = Net {
            kind,
            server: None,
            client: None,
            processes: Vec::new(),
            ended: None,
        };
        match kind {
            PlayKind::TeamCreate => {
                // This Studio's own player joins its own server, here and now.
                let mut connecting =
                    Connecting::start("127.0.0.1", port, name, crate::team::ENGINE, fingerprint, waker);
                let started = Instant::now();
                let welcome = loop {
                    server.tick(player.engine_mut().data_model_mut());
                    if let Some(welcome) = connecting.poll()? {
                        break welcome;
                    }
                    if started.elapsed() > Duration::from_secs(10) {
                        return Err("this Studio's own player could not join its server".to_owned());
                    }
                    std::thread::sleep(Duration::from_millis(2));
                };
                net.client = Some(client_player(&welcome, connecting, graphics, Arc::clone(&log))?);
            }
            PlayKind::Clients(count) => {
                let launcher = launcher.ok_or("there is no way to start the clients")?;
                for n in 1..=count {
                    match launcher(&format!("Player{n}"), port) {
                        Ok(process) => net.processes.push(process),
                        Err(error) => {
                            for process in &mut net.processes {
                                process.stop();
                            }
                            return Err(error);
                        }
                    }
                }
            }
            PlayKind::Joined => return Err("a joined game is not served here".to_owned()),
        }
        net.server = Some(server);
        let dm = player.engine().data_model();
        let explorer = ExplorerState::new(dm);
        self.play = Some(PlaySession {
            player,
            ids: loaded.ids,
            view: GameView::default(),
            number,
            history: History::new(Selection::new()),
            selection: Selection::new(),
            explorer,
            net: Some(net),
        });
        Ok(port)
    }

    /// Starts the session of a Team Create Play someone else hosts: this Studio's player of the game at `welcome`.
    pub fn play_joined(
        &mut self,
        welcome: &Welcome,
        connecting: Connecting,
        graphics: GraphicsSettings,
        log: Arc<LogBuffer>,
        number: u64,
    ) -> Result<(), String> {
        if self.play.is_some() {
            return Err("A game is running already.".to_owned());
        }
        self.output.update();
        let player = client_player(welcome, connecting, graphics, log)?;
        let dm = player.engine().data_model();
        let explorer = ExplorerState::new(dm);
        self.play = Some(PlaySession {
            player,
            ids: metallic_engine::metallic_project::SceneIds::new(),
            view: GameView::default(),
            number,
            history: History::new(Selection::new()),
            selection: Selection::new(),
            explorer,
            net: Some(Net {
                kind: PlayKind::Joined,
                server: None,
                client: None,
                processes: Vec::new(),
                ended: None,
            }),
        });
        Ok(())
    }
}
