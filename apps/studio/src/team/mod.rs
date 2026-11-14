//! Team Create in Offline Mode (ADR 0011): hosting the open project on a port, joining one by address, and the
//! session's state between frames.
//!
//! - **Host.** [`TeamCreate::host`] listens on the port the menu shows (saved in `team_create.toml`). A participant's
//!   join asks for a snapshot, made at once from the open project ([`snapshot`]): its members as they are in memory,
//!   unsaved edits included, and the start scene saved from the editor's scene as it is this frame. The host's own
//!   file is the project; nothing about the session is written into it.
//! - **Join.** [`TeamCreate::join`] connects in the background. The project arrives in chunks, is written as a
//!   participant's copy in Studio's cache folder ([`Project::from_members`]), and is opened like any project; once
//!   it is open, what the host sent meanwhile is applied and the host is told the participant is ready. The copy is
//!   never the project: Save is the host's.
//! - **Leaving.** Disconnect, End session, closing the project or the host going away all end it cleanly; a
//!   participant whose host vanished is taken back to the projects page with the reason.
//!
//! Nothing here blocks the frame: the transport's threads do the waiting, and [`TeamCreate::update`] only moves what
//! they received.

pub mod live;
pub mod menu;
pub mod page;
pub mod play_menu;
pub mod presence;
pub mod probe;
pub mod store;
pub mod text;
pub mod viewport;

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use metallic_engine::metallic_net::collab::client::{Client, ClientEvent};
use metallic_engine::metallic_net::collab::host::{Host, HostConfig, HostEvent, ProjectSnapshot};
use metallic_engine::metallic_net::collab::protocol::{DEFAULT_PORT, ProjectId, SessionInfo};
use metallic_engine::metallic_net::collab::{ClientLink, HostServer};
use metallic_engine::metallic_net::identity::{Identity, User, UserId};
use metallic_engine::metallic_net::registry_fingerprint;
use metallic_engine::metallic_net::transport::{self, Waker};
use metallic_engine::metallic_project::{Project, SceneIds, save_scene};

use crate::editor::EditorState;
use store::{Role, TeamProject, TeamStore};

/// The engine version peers must share.
pub const ENGINE: &str = env!("CARGO_PKG_VERSION");

/// A hosted session.
pub struct Hosting {
    /// The session over TCP.
    pub server: HostServer,
    /// The project hosted: the host's own file.
    pub project: PathBuf,
    /// This machine's addresses others can join at.
    pub addresses: Vec<IpAddr>,
    /// When hosting started.
    pub started: Instant,
    /// The scene and presence sync, while the hosted project is open.
    pub live: Option<live::HostLive>,
}

/// A join under way.
pub struct Joining {
    /// The session over TCP.
    pub link: ClientLink,
    /// The address as typed: host and port.
    pub host: String,
    /// The port.
    pub port: u16,
    /// Bytes of the project received, of the total, once it is arriving.
    pub progress: Option<(u64, u64)>,
    /// The participant's copy, once written and opening.
    pub copy: Option<PathBuf>,
}

/// A joined session.
pub struct Joined {
    /// The session over TCP.
    pub link: ClientLink,
    /// The host's address.
    pub host: String,
    /// The port.
    pub port: u16,
    /// The participant's copy of the project, open in the editor.
    pub copy: PathBuf,
    /// The scene and presence sync over the copy.
    pub live: Option<live::ClientLive>,
}

/// Where Team Create is.
#[derive(Default)]
pub enum Session {
    /// No session.
    #[default]
    Idle,
    /// Hosting the open project.
    Hosting(Box<Hosting>),
    /// Joining a host.
    Joining(Box<Joining>),
    /// In a host's session.
    Joined(Box<Joined>),
}

/// How Play runs (ADR 0011 §8), chosen from the arrow beside Play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayMode {
    /// The game in Studio alone: Server and Game tabs over one game.
    #[default]
    Solo,
    /// The host runs a shared server; everyone in the session joins it as a player.
    TeamCreate,
    /// A local server and Player processes that join it by themselves.
    Clients,
}

/// The message kinds of shared play, between Team Create's host and its participants.
pub mod play_kind {
    /// The host started Team Create Play: its game server's port follows.
    pub const STARTED: u8 = 40;
    /// The host stopped it.
    pub const STOPPED: u8 = 41;
}

/// Something Studio has to do for Team Create.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Open the participant's copy at this path.
    Open(PathBuf),
    /// The host started Team Create Play: join its game server at `host:port` as a player.
    JoinPlay {
        /// The host's address.
        host: String,
        /// The game server's port.
        port: u16,
    },
    /// The host stopped Team Create Play.
    StopPlay,
    /// The participant's session is over: close the copy and tell them why.
    Close {
        /// The notice's title.
        title: &'static str,
        /// Why.
        body: String,
    },
}

/// What the projects page's Team Create section asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageAction {
    /// Join this project at the address it was last hosted at.
    Join(ProjectId),
    /// Open the owner's own project file, hosting it again on `port` once it is open when `host`.
    OpenOwned {
        /// The project's file.
        path: PathBuf,
        /// The port it was hosted on.
        port: u16,
        /// Whether to host it again.
        host: bool,
    },
    /// Join the address the form holds (Join by address).
    JoinAddress,
    /// Forget this project.
    Forget(ProjectId),
}

/// Something the menu says until the next action: an error, or why the last session ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// Whether it is an error.
    pub error: bool,
    /// What to say.
    pub text: String,
}

/// The fields the menu edits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    /// The port to host on, as typed.
    pub port: String,
    /// The address to join, as typed.
    pub address: String,
    /// The name to join as; empty for the profile's.
    pub name: String,
}

/// Team Create's state: the saved choices, the session, and what the menu shows.
pub struct TeamCreate {
    /// The saved choices and projects.
    pub store: TeamStore,
    session: Session,
    waker: Waker,
    /// Where participants' copies are written.
    cache: PathBuf,
    /// What the menu says.
    pub note: Option<Note>,
    /// The menu's fields.
    pub form: Form,
    /// Whether each Team Create project is reachable now, for the projects page.
    pub prober: probe::Prober,
    /// What the projects page's Team Create section asked for this frame, for Studio to do.
    pub page_actions: Vec<PageAction>,
    /// The owner's project to host again once it is open (the projects page's Open and host).
    pub host_after_open: Option<PathBuf>,
    /// How Play runs.
    pub play_mode: PlayMode,
}

impl std::fmt::Debug for TeamCreate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeamCreate")
            .field("session", &self.status_line())
            .finish_non_exhaustive()
    }
}

impl TeamCreate {
    /// Team Create with `store`'s choices, waking the window through `waker`, writing participants' copies under
    /// `cache`.
    pub fn new(store: TeamStore, waker: Waker, cache: PathBuf) -> Self {
        let prober_waker = Arc::clone(&waker);
        let form = Form {
            port: store.port.to_string(),
            address: store.address.clone(),
            name: String::new(),
        };
        Self {
            store,
            session: Session::Idle,
            waker,
            cache,
            note: None,
            form,
            prober: probe::Prober::new(Arc::clone(&prober_waker)),
            page_actions: Vec::new(),
            host_after_open: None,
            play_mode: PlayMode::Solo,
        }
    }

    /// Team Create kept in memory, its copies in the system's temporary folder: for tests.
    pub fn in_memory() -> Self {
        Self::new(
            TeamStore::in_memory(),
            transport::no_waker(),
            std::env::temp_dir().join("metallic-team-create"),
        )
    }

    /// Wakes the window through `waker` from now on (the shell's event loop).
    pub fn set_waker(&mut self, waker: Waker) {
        self.prober.set_waker(Arc::clone(&waker));
        self.waker = waker;
    }

    /// Writes participants' copies under `cache` from now on.
    pub fn set_cache(&mut self, cache: PathBuf) {
        self.cache = cache;
    }

    /// The waker the window is woken through, for the play servers and clients a session starts.
    pub fn waker(&self) -> Waker {
        Arc::clone(&self.waker)
    }

    /// Tells everyone in the hosted session that Team Create Play started on `port` (or stopped, with `None`).
    pub fn announce_play(&mut self, port: Option<u16>) {
        if let Session::Hosting(hosting) = &mut self.session {
            let channel = metallic_engine::metallic_net::transport::Channel::Reliable;
            match port {
                Some(port) => {
                    hosting
                        .server
                        .host
                        .broadcast_op(channel, play_kind::STARTED, port.to_le_bytes().to_vec(), None)
                }
                None => hosting
                    .server
                    .host
                    .broadcast_op(channel, play_kind::STOPPED, Vec::new(), None),
            }
            hosting.server.flush();
        }
    }

    /// Uses `store` (read once the splash is done).
    pub fn set_store(&mut self, store: TeamStore) {
        self.form.port = store.port.to_string();
        self.form.address.clone_from(&store.address);
        self.store = store;
    }

    /// The session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The session, to change.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Whether a session is hosted, being joined or joined.
    pub fn is_active(&self) -> bool {
        !matches!(self.session, Session::Idle)
    }

    /// Whether this Studio hosts a session.
    pub fn is_hosting(&self) -> bool {
        matches!(self.session, Session::Hosting(_))
    }

    /// Whether `path` is a participant's copy (open or opening) of a session joined now.
    pub fn is_copy(&self, path: &Path) -> bool {
        match &self.session {
            Session::Joined(joined) => joined.copy == path,
            Session::Joining(joining) => joining.copy.as_deref() == Some(path),
            _ => false,
        }
    }

    /// Whether `path` is in the folder participants' copies are written to: never a project to host.
    pub fn is_in_cache(&self, path: &Path) -> bool {
        path.starts_with(&self.cache)
    }

    /// The users of the session, with whether each has the project open: empty without one.
    pub fn users(&self) -> Vec<(User, bool)> {
        match &self.session {
            Session::Idle => Vec::new(),
            Session::Hosting(hosting) => {
                let host = &hosting.server.host;
                host.users()
                    .into_iter()
                    .map(|user| {
                        let ready = host.is_ready(user.id);
                        (user, ready)
                    })
                    .collect()
            }
            Session::Joining(joining) => with_ready(&joining.link.client),
            Session::Joined(joined) => with_ready(&joined.link.client),
        }
    }

    /// The session's description, once there is one.
    pub fn info(&self) -> Option<&SessionInfo> {
        match &self.session {
            Session::Idle => None,
            Session::Hosting(hosting) => Some(hosting.server.host.info()),
            Session::Joining(joining) => joining.link.client.session(),
            Session::Joined(joined) => joined.link.client.session(),
        }
    }

    /// This user's id in the session.
    pub fn me(&self) -> Option<UserId> {
        match &self.session {
            Session::Idle => None,
            Session::Hosting(_) => Some(UserId::HOST),
            Session::Joining(joining) => joining.link.client.me(),
            Session::Joined(joined) => joined.link.client.me(),
        }
    }

    /// One line on where the session is, as the menu and logs say it.
    pub fn status_line(&self) -> String {
        match &self.session {
            Session::Idle => "Not in a session".to_owned(),
            Session::Hosting(hosting) => format!("Hosting on port {}", hosting.server.port()),
            Session::Joining(joining) => match (&joining.copy, joining.progress) {
                (Some(_), _) => "Opening the project…".to_owned(),
                (None, Some((received, total))) if total > 0 => {
                    format!("Receiving the project… {}%", received * 100 / total)
                }
                (None, _) if joining.link.client.session().is_some() => "Receiving the project…".to_owned(),
                (None, _) => format!("Connecting to {}:{}…", joining.host, joining.port),
            },
            Session::Joined(joined) => format!("Connected to {}:{}", joined.host, joined.port),
        }
    }

    /// Starts hosting `editor`'s project as `identity` on the port the form gives.
    pub fn host(&mut self, editor: &mut EditorState, identity: Identity) -> Result<(), String> {
        if self.is_active() {
            return Err("Leave the current session first.".to_owned());
        }
        if self.is_in_cache(editor.path()) {
            return Err("This is a copy of someone else's project: only its owner can host it.".to_owned());
        }
        let port = parse_port(&self.form.port)?;
        let path = editor.path().to_path_buf();
        let project = self.store.id_for_owned(&path);
        let config = HostConfig {
            info: SessionInfo {
                session: random_u64(),
                project,
                project_name: editor.name().to_owned(),
                host_name: identity.name.clone(),
            },
            host: identity.clone(),
            engine: ENGINE.to_owned(),
            fingerprint: registry_fingerprint(editor.dm.registry()),
        };
        let server = HostServer::start(port, Host::new(config), Arc::clone(&self.waker)).map_err(capitalise)?;
        let port = server.port();
        let addresses = transport::local_addresses();
        log::info!("team create: hosting {} on port {port}", editor.name());
        self.store.port = port;
        self.store.remember(TeamProject {
            id: project,
            name: editor.name().to_owned(),
            owner: identity.name,
            role: Role::Owner,
            path: Some(path.clone()),
            host: addresses.first().map(IpAddr::to_string).unwrap_or_default(),
            port,
            last_used: 0,
        });
        self.note = None;
        let live = live::HostLive::new(editor)?;
        self.session = Session::Hosting(Box::new(Hosting {
            server,
            project: path,
            addresses,
            started: Instant::now(),
            live: Some(live),
        }));
        Ok(())
    }

    /// Starts joining the host at `address` (`host:port`, the default port if none) as `identity`, expecting
    /// `project` when the user joined it before.
    pub fn join(&mut self, address: &str, identity: Identity, project: Option<ProjectId>) -> Result<(), String> {
        if self.is_active() {
            return Err("Leave the current session first.".to_owned());
        }
        let (host, port) = transport::parse_address(address, DEFAULT_PORT).map_err(capitalise)?;
        log::info!("team create: joining {host}:{port} as {}", identity.name);
        // The participant's classes are this build's built-in ones: the host must have the same.
        let registry = metallic_engine::metallic_classes::shared_registry()
            .map_err(|error| format!("Studio's classes did not load: {error}"))?;
        let fingerprint = registry_fingerprint(&registry);
        let client = Client::join(identity, project, ENGINE, fingerprint);
        let link = ClientLink::connect(&host, port, client, Arc::clone(&self.waker));
        self.store.address = address.trim().to_owned();
        self.store.save_or_log();
        self.note = None;
        self.session = Session::Joining(Box::new(Joining {
            link,
            host,
            port,
            progress: None,
            copy: None,
        }));
        Ok(())
    }

    /// Ends the hosted session: saves `editor`'s project if it has changes (the session's edits are the host's to
    /// keep), tells everyone, and stops listening.
    pub fn end(&mut self, editor: Option<&mut EditorState>) {
        let Session::Hosting(mut hosting) = std::mem::take(&mut self.session) else {
            return;
        };
        let editor = editor.filter(|editor| editor.path() == hosting.project);
        if let Some(editor) = editor {
            // Everything edited so far is the host's and saved; then the sync lets go of the scene.
            if let Some(mut live) = hosting.live.take() {
                live.flush(editor, &mut hosting.server.host);
                live.release(editor);
            }
            if editor.is_modified()
                && !editor.is_playing()
                && let Err(error) = editor.save()
            {
                log::warn!("team create: could not save at the end of the session: {error}");
                self.note = Some(Note {
                    error: true,
                    text: format!("The session ended, but the project was not saved: {error}"),
                });
            }
        }
        hosting.server.host.end("The host ended the session.");
        hosting.server.flush();
        log::info!("team create: ended the session");
        if self.note.is_none() {
            self.note = Some(Note {
                error: false,
                text: "The session ended.".to_owned(),
            });
        }
    }

    /// Leaves the joined session (or stops joining); `editor`, the participant's copy if it is still open, stops
    /// syncing.
    pub fn leave(&mut self, editor: Option<&mut EditorState>) {
        match std::mem::take(&mut self.session) {
            Session::Joining(mut joining) => joining.link.leave(),
            Session::Joined(mut joined) => {
                if let (Some(live), Some(editor)) = (joined.live.take(), editor)
                    && editor.path() == joined.copy
                {
                    live.release(editor);
                }
                joined.link.leave();
            }
            other => self.session = other,
        }
    }

    /// Moves what the network brought since the last frame, and says what Studio has to do about it. `editor` is
    /// the open project, if one is open; `opening` the project opening, if one is; `open` the scripts whose tabs are
    /// open (others see them as open).
    pub fn update(
        &mut self,
        mut editor: Option<&mut EditorState>,
        opening: Option<&Path>,
        open: &[metallic_engine::metallic_core::Id],
    ) -> Vec<Request> {
        let mut requests = Vec::new();
        if matches!(self.session, Session::Joining(_)) {
            if let Some(outcome) = self.join_frame_inner(editor.as_deref_mut(), opening) {
                requests.extend(outcome);
            }
            return requests;
        }
        match &mut self.session {
            Session::Idle | Session::Joining(_) => {}
            Session::Hosting(hosting) => {
                let hosted = editor.as_deref_mut().filter(|editor| editor.path() == hosting.project);
                host_frame(hosting, hosted, open);
            }
            Session::Joined(joined) => {
                joined.link.receive();
                let mut over = None;
                let mut copy = editor.filter(|editor| editor.path() == joined.copy);
                while let Some(event) = joined.link.client.next_event() {
                    match event {
                        ClientEvent::Ended(reason) => over = Some(("The session ended", reason)),
                        ClientEvent::Broken(reason) => {
                            over = Some((
                                "Disconnected from the host",
                                format!("{reason}. Rejoin from Team Create."),
                            ))
                        }
                        ClientEvent::Refused(reason) => over = Some(("Disconnected from the host", reason)),
                        ClientEvent::Op {
                            kind: play_kind::STARTED,
                            bytes,
                            ..
                        } => {
                            if let [a, b] = bytes[..] {
                                requests.push(Request::JoinPlay {
                                    host: joined.host.clone(),
                                    port: u16::from_le_bytes([a, b]),
                                });
                            }
                        }
                        ClientEvent::Op {
                            kind: play_kind::STOPPED,
                            ..
                        } => requests.push(Request::StopPlay),
                        ClientEvent::Op { kind, bytes, .. } => {
                            if let (Some(live), Some(editor)) = (joined.live.as_mut(), copy.as_deref_mut())
                                && !live.receive(editor, &mut joined.link.client, kind, &bytes)
                            {
                                over = Some((
                                    "Disconnected from the host",
                                    "The host sent an edit Studio could not read. Rejoin from Team Create.".to_owned(),
                                ));
                            }
                        }
                        _ => {}
                    }
                }
                if over.is_none()
                    && let (Some(live), Some(editor)) = (joined.live.as_mut(), copy.as_deref_mut())
                {
                    live.tick(editor, &mut joined.link.client, open);
                }
                joined.link.flush();
                let still_open = copy.is_some();
                if let Some((title, body)) = over {
                    if let (Some(live), Some(editor)) = (joined.live.take(), copy) {
                        live.release(editor);
                    }
                    log::info!("team create: {title}: {body}");
                    self.note = Some(Note {
                        error: true,
                        text: body.clone(),
                    });
                    self.session = Session::Idle;
                    requests.push(Request::Close { title, body });
                } else if !still_open && opening != Some(joined.copy.as_path()) {
                    // The participant closed the copy: that is leaving.
                    self.leave(None);
                }
            }
        }
        requests
    }

    fn join_frame_inner(&mut self, editor: Option<&mut EditorState>, opening: Option<&Path>) -> Option<Vec<Request>> {
        let Session::Joining(joining) = &mut self.session else {
            return None;
        };
        let mut requests = Vec::new();
        joining.link.receive();
        let mut failed = None;
        while let Some(event) = joining.link.client.next_event() {
            match event {
                ClientEvent::Welcomed { session, .. } => {
                    log::info!("team create: welcomed into {}", session.project_name);
                    self.store.remember(TeamProject {
                        id: session.project,
                        name: session.project_name.clone(),
                        owner: session.host_name.clone(),
                        role: Role::Participant,
                        path: None,
                        host: joining.host.clone(),
                        port: joining.port,
                        last_used: 0,
                    });
                }
                ClientEvent::Progress { received, total } => joining.progress = Some((received, total)),
                ClientEvent::Project(project) => {
                    let name = joining
                        .link
                        .client
                        .session()
                        .map_or_else(|| "Project".to_owned(), |s| s.project_name.clone());
                    let id = joining.link.client.session().map_or(ProjectId(0), |s| s.project);
                    let path = copy_path(&self.cache, id, &name);
                    let written = std::fs::create_dir_all(path.parent().unwrap_or(&self.cache))
                        .map_err(|error| error.to_string())
                        .and_then(|()| {
                            Project::from_members(
                                &path,
                                project
                                    .members
                                    .iter()
                                    .map(|(member, bytes)| (member.as_str(), &bytes[..])),
                            )
                            .map(drop)
                            .map_err(|error| error.to_string())
                        });
                    match written {
                        Ok(()) => {
                            joining.copy = Some(path.clone());
                            requests.push(Request::Open(path));
                        }
                        Err(error) => {
                            failed = Some(format!("Could not keep the project on this computer: {error}"));
                        }
                    }
                }
                ClientEvent::Refused(reason) => failed = Some(reason),
                ClientEvent::Ended(reason) => failed = Some(reason),
                ClientEvent::Broken(reason) => failed = Some(capitalise(reason)),
                ClientEvent::Users | ClientEvent::Op { .. } => {}
            }
        }
        if let Some(reason) = failed {
            log::warn!("team create: could not join: {reason}");
            joining.link.leave();
            self.note = Some(Note {
                error: true,
                text: reason.clone(),
            });
            let was_opening = joining.copy.is_some();
            self.session = Session::Idle;
            if was_opening {
                requests.push(Request::Close {
                    title: "Could not join",
                    body: reason,
                });
            }
            return Some(requests);
        }
        joining.link.flush();
        // The copy is open: what was held is applied from now on, and the host hears the participant is ready.
        if let Some(copy) = joining.copy.clone()
            && let Some(editor) = editor.filter(|editor| editor.path() == copy)
        {
            // The sync starts on the copy as it opened (the snapshot), then what the host sent meanwhile applies.
            let me = joining.link.client.me().unwrap_or(UserId(0));
            let live = match live::ClientLive::new(editor, me) {
                Ok(live) => live,
                Err(error) => {
                    joining.link.leave();
                    self.session = Session::Idle;
                    let body = format!("Studio could not follow the scene: {error}");
                    self.note = Some(Note {
                        error: true,
                        text: body.clone(),
                    });
                    requests.push(Request::Close {
                        title: "Could not join",
                        body,
                    });
                    return Some(requests);
                }
            };
            joining.link.client.loaded();
            let Session::Joining(joining) = std::mem::take(&mut self.session) else {
                return Some(requests);
            };
            let Joining { link, host, port, .. } = *joining;
            log::info!("team create: joined {host}:{port}");
            let mut joined = Joined {
                link,
                host,
                port,
                copy,
                live: Some(live),
            };
            // The operations held since the snapshot, in order.
            while let Some(event) = joined.link.client.next_event() {
                if let ClientEvent::Op { kind, bytes, .. } = event
                    && let Some(live) = joined.live.as_mut()
                {
                    live.receive(editor, &mut joined.link.client, kind, &bytes);
                }
            }
            joined.link.flush();
            self.session = Session::Joined(Box::new(joined));
        } else if joining.copy.is_some() && opening.is_none() && requests.is_empty() {
            // The copy failed to open (the projects page says why).
            joining.link.leave();
            self.session = Session::Idle;
        }
        Some(requests)
    }
}

/// One frame of hosting: joins answered with a snapshot of `editor` (the hosted project, when it is open).
fn host_frame(
    hosting: &mut Hosting,
    mut editor: Option<&mut EditorState>,
    open: &[metallic_engine::metallic_core::Id],
) {
    let server = &mut hosting.server;
    server.receive();
    while let Some(event) = server.host.next_event() {
        match event {
            HostEvent::JoinRequested { peer, user } => match editor.as_deref_mut() {
                Some(editor) => {
                    // Every edit so far is sent first, so the snapshot and the live stream meet exactly.
                    let ids = match hosting.live.as_mut() {
                        Some(live) => {
                            live.flush(editor, &mut server.host);
                            Some(live.scene_ids())
                        }
                        None => None,
                    };
                    match snapshot(editor, ids) {
                        Ok(snapshot) => {
                            log::info!("team create: {} is joining", user.identity.name);
                            server.host.admit(peer, &snapshot);
                            if let Some(live) = hosting.live.as_mut() {
                                live.joined(user.id, &mut server.host);
                            }
                        }
                        Err(error) => {
                            log::warn!(
                                "team create: could not send the project to {}: {error}",
                                user.identity.name
                            );
                            server
                                .host
                                .refuse(peer, &format!("The host could not send the project: {error}"));
                        }
                    }
                }
                None => server.host.refuse(peer, "The host has no project open."),
            },
            HostEvent::Ready(id) => log::info!("team create: {id} has the project open"),
            HostEvent::Left { user, reason } => {
                log::info!("team create: {} left ({reason})", user.identity.name);
                if let Some(live) = hosting.live.as_mut() {
                    live.left(user.id);
                }
            }
            HostEvent::Op { from, kind, bytes, .. } => {
                if let (Some(live), Some(editor)) = (hosting.live.as_mut(), editor.as_deref_mut())
                    && !live.receive(editor, &mut server.host, from, kind, &bytes)
                {
                    log::warn!("team create: {from} sent an edit that is not valid; dropping them");
                    server.host.drop_user(from, "sent an edit that is not valid");
                }
            }
        }
    }
    if let (Some(live), Some(editor)) = (hosting.live.as_mut(), editor) {
        live.tick(editor, &mut server.host, open);
    }
    server.flush();
}

/// The project as a joining participant gets it: every member as it is in memory, the start scene saved from the
/// editor's scene as it is now.
pub fn snapshot(editor: &mut EditorState, ids: Option<SceneIds>) -> Result<ProjectSnapshot, String> {
    let start_scene = editor.project.manifest().start_scene.clone();
    // The session's net ids as the copy's scene ids (a participant's sync starts from them); the file's own ids
    // otherwise.
    let mut ids = ids.unwrap_or_else(|| editor.scene_ids.clone());
    let scene = save_scene(&editor.dm, &mut ids).map_err(|error| error.to_string())?;
    let names: Vec<String> = editor.project.members().into_iter().map(str::to_owned).collect();
    let mut members = Vec::with_capacity(names.len() + 1);
    for name in names {
        if name == start_scene {
            continue;
        }
        let bytes = editor.project.read(&name).map_err(|error| error.to_string())?;
        members.push((name, Arc::from(bytes)));
    }
    members.push((start_scene.clone(), Arc::from(scene.text.into_bytes())));
    Ok(ProjectSnapshot { members, start_scene })
}

/// Where a participant's copy of project `id`, called `name`, is written under `cache`.
pub fn copy_path(cache: &Path, id: ProjectId, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let safe = safe.trim();
    let file = if safe.is_empty() { "Project" } else { safe };
    cache.join(id.to_hex()).join(format!("{file}.mtproj"))
}

fn with_ready(client: &Client) -> Vec<(User, bool)> {
    client
        .users()
        .iter()
        .map(|user| (user.clone(), client.is_ready(user.id)))
        .collect()
}

/// `text` as a port: 1 to 65535.
pub fn parse_port(text: &str) -> Result<u16, String> {
    match text.trim().parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err(format!(
            "\u{201C}{}\u{201D} is not a port: use a number from 1 to 65535.",
            text.trim()
        )),
    }
}

fn capitalise(text: impl Into<String>) -> String {
    let text = text.into();
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

fn random_u64() -> u64 {
    ProjectId::random().0 as u64
}

#[cfg(test)]
mod tests;
