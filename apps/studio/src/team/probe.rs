//! Whether each Team Create project is reachable now (ADR 0011, discovery): a real probe of its host, never a saved
//! flag.
//!
//! [`Prober::refresh`] asks each project's host what it hosts, on a worker thread each, at most once every
//! [`INTERVAL`] unless asked again by hand; an answer naming the same project is **available**, anything else (no
//! answer in [`TIMEOUT`], refused, another project) is **unavailable**, with the reason. Results come back through a
//! channel the page drains each frame; the waker redraws it when one arrives.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use metallic_engine::metallic_net::collab::protocol::ProjectId;
use metallic_engine::metallic_net::collab::{Probed, probe};
use metallic_engine::metallic_net::registry_fingerprint;
use metallic_engine::metallic_net::transport::Waker;

use super::ENGINE;
use super::store::TeamProject;

/// How long a probe waits for an answer.
pub const TIMEOUT: Duration = Duration::from_millis(1500);
/// How often the page probes again while it shows.
pub const INTERVAL: Duration = Duration::from_secs(15);

/// What is known of a project's host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Never asked.
    Unknown,
    /// Asked; no answer yet.
    Checking,
    /// It hosts the project now, with this many people in it.
    Available {
        /// How many people are in the session.
        users: u32,
        /// Who hosts it.
        host: String,
    },
    /// It does not, and why.
    Unavailable(String),
}

impl Reach {
    /// Whether the project can be joined now.
    pub fn is_available(&self) -> bool {
        matches!(self, Reach::Available { .. })
    }
}

/// The probes of the Team Create projects.
pub struct Prober {
    states: HashMap<ProjectId, Reach>,
    asked: Option<Instant>,
    tx: Sender<(ProjectId, Result<Probed, String>)>,
    rx: Receiver<(ProjectId, Result<Probed, String>)>,
    waker: Waker,
}

impl std::fmt::Debug for Prober {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prober")
            .field("states", &self.states)
            .finish_non_exhaustive()
    }
}

impl Prober {
    /// No probes yet, redrawing through `waker` when one answers.
    pub fn new(waker: Waker) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            states: HashMap::new(),
            asked: None,
            tx,
            rx,
            waker,
        }
    }

    /// Redraws through `waker` from now on.
    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = waker;
    }

    /// What is known of project `id`.
    pub fn reach(&self, id: ProjectId) -> &Reach {
        self.states.get(&id).unwrap_or(&Reach::Unknown)
    }

    /// Sets what is known of project `id` (this Studio's own session, which needs no probe).
    pub fn set(&mut self, id: ProjectId, reach: Reach) {
        self.states.insert(id, reach);
    }

    /// Takes the answers that came in. Returns whether any did.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok((id, result)) = self.rx.try_recv() {
            any = true;
            let reach = match result {
                Ok(probed) if probed.session.project == id => Reach::Available {
                    users: probed.users,
                    host: probed.session.host_name,
                },
                Ok(probed) => Reach::Unavailable(format!(
                    "That address hosts another project now ({}).",
                    probed.session.project_name
                )),
                Err(reason) => Reach::Unavailable(reason),
            };
            self.states.insert(id, reach);
        }
        any
    }

    /// Probes every project in `projects` unless they were probed less than [`INTERVAL`] ago (`now`), or `force`.
    /// Projects already being probed are not asked twice.
    pub fn refresh(&mut self, projects: &[TeamProject], now: Instant, force: bool) {
        if !force && self.asked.is_some_and(|at| now.duration_since(at) < INTERVAL) {
            return;
        }
        self.asked = Some(now);
        let Ok(registry) = metallic_engine::metallic_classes::shared_registry() else {
            return;
        };
        let fingerprint = registry_fingerprint(&registry);
        for project in projects {
            if matches!(self.states.get(&project.id), Some(Reach::Checking)) || project.host.is_empty() {
                continue;
            }
            self.states.insert(project.id, Reach::Checking);
            let tx = self.tx.clone();
            let waker = std::sync::Arc::clone(&self.waker);
            let (id, host, port) = (project.id, project.host.clone(), project.port);
            let spawned = std::thread::Builder::new()
                .name("team-probe".to_owned())
                .spawn(move || {
                    let result = probe(&host, port, ENGINE, fingerprint, TIMEOUT);
                    let _ = tx.send((id, result));
                    waker();
                });
            if spawned.is_err() {
                self.states
                    .insert(project.id, Reach::Unavailable("Studio could not check it.".to_owned()));
            }
        }
    }
}
