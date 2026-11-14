//! `team_create.toml` in Studio's config folder: Team Create's choices and its projects (ADR 0011).
//!
//! ```toml
//! version = 1
//! port = 7350                  # the port hosting listens on
//! address = "192.168.1.20:7350" # the last address joined
//! clients = 2                  # Multiple Clients' count
//!
//! [[project]]
//! id = "3f2a..."               # the project's Team Create id (32 hex digits)
//! name = "Castle"
//! owner = "Ada"                # who hosts it
//! role = "owner"               # or "participant"
//! path = "C:\\...\\Castle.mtproj" # the owner's own file (owners only)
//! host = "192.168.1.20"        # where it was last hosted
//! port = 7350
//! last_used = 1762430400       # seconds since 1970-01-01 UTC
//! ```
//!
//! A missing file is the defaults; a write goes to a temporary file renamed over this one, so a crash never leaves
//! half a file; a damaged file is kept aside as `.bak` and Studio starts from the defaults.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use metallic_engine::metallic_net::collab::protocol::{DEFAULT_PORT, ProjectId};
use toml::{Table, Value};

use crate::settings::store::{backup_path, write_atomically};

/// The file's name in Studio's config folder.
pub const FILE: &str = "team_create.toml";

/// The format version written.
const VERSION: i64 = 1;

/// How many clients Multiple Clients starts unless the user picks another number.
pub const DEFAULT_CLIENTS: u8 = 2;
/// The most clients Multiple Clients starts.
pub const MAX_CLIENTS: u8 = 8;

/// What the user is to a Team Create project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// They host it: the project is their own file.
    Owner,
    /// They joined it on someone else's Studio.
    Participant,
}

/// A Team Create project on the projects page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamProject {
    /// Its Team Create id.
    pub id: ProjectId,
    /// Its name.
    pub name: String,
    /// Who hosts it.
    pub owner: String,
    /// What the user is to it.
    pub role: Role,
    /// The project's file, for its owner.
    pub path: Option<PathBuf>,
    /// The address it was last hosted at (as typed, or the host's own address for the owner).
    pub host: String,
    /// The port it was last hosted on.
    pub port: u16,
    /// When it was last hosted or joined, in seconds since 1970-01-01 UTC.
    pub last_used: u64,
}

/// Team Create's saved state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamStore {
    path: Option<PathBuf>,
    /// The port hosting listens on.
    pub port: u16,
    /// The last address joined.
    pub address: String,
    /// Multiple Clients' count.
    pub clients: u8,
    /// The projects, most recently used first.
    pub projects: Vec<TeamProject>,
}

impl Default for TeamStore {
    fn default() -> Self {
        Self {
            path: None,
            port: DEFAULT_PORT,
            address: String::new(),
            clients: DEFAULT_CLIENTS,
            projects: Vec::new(),
        }
    }
}

/// Seconds since 1970-01-01 UTC, now.
pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl TeamStore {
    /// The defaults, never saved.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Reads `path`; see the module's documentation for a missing or damaged file.
    pub fn load(path: PathBuf) -> Self {
        let mut store = Self {
            path: Some(path.clone()),
            ..Self::default()
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return store,
            Err(error) => {
                log::warn!("team create: could not read {}: {error}", path.display());
                store.path = None;
                return store;
            }
        };
        match text.parse::<Table>() {
            Ok(table) => store.read(&table),
            Err(error) => {
                if let Some(backup) = backup_path(&path)
                    && std::fs::rename(&path, &backup).is_ok()
                {
                    log::warn!(
                        "team create: {} is damaged ({}); kept as {}",
                        path.display(),
                        error.message(),
                        backup.display()
                    );
                } else {
                    store.path = None;
                }
            }
        }
        store
    }

    fn read(&mut self, table: &Table) {
        let int = |table: &Table, key: &str| table.get(key).and_then(Value::as_integer);
        let text = |table: &Table, key: &str| table.get(key).and_then(Value::as_str).map(str::to_owned);
        if let Some(port) = int(table, "port")
            .and_then(|p| u16::try_from(p).ok())
            .filter(|&p| p != 0)
        {
            self.port = port;
        }
        if let Some(address) = text(table, "address") {
            self.address = address;
        }
        if let Some(clients) = int(table, "clients").and_then(|c| u8::try_from(c).ok()) {
            self.clients = clients.clamp(1, MAX_CLIENTS);
        }
        let Some(projects) = table.get("project").and_then(Value::as_array) else {
            return;
        };
        for entry in projects.iter().filter_map(Value::as_table) {
            let Some(id) = text(entry, "id").and_then(|id| ProjectId::from_hex(&id)) else {
                continue;
            };
            if self.projects.iter().any(|p| p.id == id) {
                continue;
            }
            let role = match text(entry, "role").as_deref() {
                Some("owner") => Role::Owner,
                _ => Role::Participant,
            };
            let path = text(entry, "path").map(PathBuf::from);
            if role == Role::Owner && path.is_none() {
                continue;
            }
            self.projects.push(TeamProject {
                id,
                name: text(entry, "name").unwrap_or_default(),
                owner: text(entry, "owner").unwrap_or_default(),
                role,
                path,
                host: text(entry, "host").unwrap_or_default(),
                port: int(entry, "port")
                    .and_then(|p| u16::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .unwrap_or(DEFAULT_PORT),
                last_used: int(entry, "last_used").and_then(|t| u64::try_from(t).ok()).unwrap_or(0),
            });
        }
        self.projects.sort_by_key(|p| std::cmp::Reverse(p.last_used));
    }

    /// The file's text.
    fn to_text(&self) -> String {
        let mut table = Table::new();
        table.insert("version".into(), Value::Integer(VERSION));
        table.insert("port".into(), Value::Integer(i64::from(self.port)));
        table.insert("address".into(), Value::String(self.address.clone()));
        table.insert("clients".into(), Value::Integer(i64::from(self.clients)));
        let projects = self
            .projects
            .iter()
            .map(|p| {
                let mut entry = Table::new();
                entry.insert("id".into(), Value::String(p.id.to_hex()));
                entry.insert("name".into(), Value::String(p.name.clone()));
                entry.insert("owner".into(), Value::String(p.owner.clone()));
                let role = match p.role {
                    Role::Owner => "owner",
                    Role::Participant => "participant",
                };
                entry.insert("role".into(), Value::String(role.into()));
                if let Some(path) = &p.path {
                    entry.insert("path".into(), Value::String(path.to_string_lossy().into_owned()));
                }
                entry.insert("host".into(), Value::String(p.host.clone()));
                entry.insert("port".into(), Value::Integer(i64::from(p.port)));
                entry.insert(
                    "last_used".into(),
                    Value::Integer(i64::try_from(p.last_used).unwrap_or(i64::MAX)),
                );
                Value::Table(entry)
            })
            .collect();
        table.insert("project".into(), Value::Array(projects));
        format!(
            "# Metallic Studio's Team Create projects and choices. Studio rewrites this file.\n{}",
            toml::to_string(&table).unwrap_or_default()
        )
    }

    /// Writes the file, logging a failure. Does nothing for a store in memory.
    pub fn save_or_log(&self) {
        if let Some(path) = &self.path
            && let Err(error) = write_atomically(path, &self.to_text())
        {
            log::warn!("team create: could not save {}: {error}", path.display());
        }
    }

    /// The Team Create id of the project file `path`, which its owner hosts: the one it was given when first
    /// hosted, or a new one.
    pub fn id_for_owned(&mut self, path: &Path) -> ProjectId {
        self.projects
            .iter()
            .find(|p| p.role == Role::Owner && p.path.as_deref() == Some(path))
            .map_or_else(ProjectId::random, |p| p.id)
    }

    /// Records `project` as used now: added, or updated in place, and moved to the front. Saved.
    pub fn remember(&mut self, mut project: TeamProject) {
        project.last_used = now_seconds();
        self.projects.retain(|p| p.id != project.id);
        self.projects.insert(0, project);
        self.save_or_log();
    }

    /// Forgets the project with `id`. Saved.
    pub fn forget(&mut self, id: ProjectId) {
        self.projects.retain(|p| p.id != id);
        self.save_or_log();
    }

    /// The project with `id`.
    pub fn get(&self, id: ProjectId) -> Option<&TeamProject> {
        self.projects.iter().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("metallic-team-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(FILE);
        let mut store = TeamStore::load(path.clone());
        assert_eq!(store.port, DEFAULT_PORT);
        store.port = 9000;
        store.address = "10.0.0.2:9000".into();
        store.clients = 3;
        store.remember(TeamProject {
            id: ProjectId(5),
            name: "Castle".into(),
            owner: "Ada".into(),
            role: Role::Owner,
            path: Some(PathBuf::from("C:/games/Castle.mtproj")),
            host: "10.0.0.1".into(),
            port: 9000,
            last_used: 0,
        });
        store.remember(TeamProject {
            id: ProjectId(6),
            name: "Moon".into(),
            owner: "Bob".into(),
            role: Role::Participant,
            path: None,
            host: "10.0.0.7".into(),
            port: 7350,
            last_used: 0,
        });
        let again = TeamStore::load(path.clone());
        assert_eq!(again.port, 9000);
        assert_eq!(again.address, "10.0.0.2:9000");
        assert_eq!(again.clients, 3);
        assert_eq!(again.projects.len(), 2);
        assert_eq!(
            again.get(ProjectId(5)).unwrap().path.as_deref(),
            Some(Path::new("C:/games/Castle.mtproj"))
        );
        assert_eq!(again.get(ProjectId(6)).unwrap().role, Role::Participant);
        let mut again = again;
        assert_eq!(again.id_for_owned(Path::new("C:/games/Castle.mtproj")), ProjectId(5));
        again.forget(ProjectId(5));
        assert_eq!(TeamStore::load(path.clone()).projects.len(), 1);
        // A damaged file is set aside, not overwritten.
        std::fs::write(&path, "not [toml").unwrap();
        let fresh = TeamStore::load(path.clone());
        assert!(fresh.projects.is_empty());
        assert!(dir.join(format!("{FILE}.bak")).exists() || std::fs::read_dir(&dir).unwrap().count() >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
