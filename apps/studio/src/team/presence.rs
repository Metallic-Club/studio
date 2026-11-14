//! Other users' presence as the panels show it (ADR 0011): their cameras in the viewport, their selections outlined
//! in their colour, and dots in the Explorer. Team Create fills it each frame from the session; the panels only read
//! it, in the editor's own ids. Nothing here is saved.

use std::time::{Duration, Instant};

use metallic_engine::metallic_core::glam::{Quat, Vec3};
use metallic_engine::metallic_core::{CFrame, Id};
use metallic_engine::metallic_net::identity::{User, UserId};

/// How long a remote camera takes to glide to the place it was last sent at: about one presence update.
pub const GLIDE: Duration = Duration::from_millis(120);

/// One other user, as the panels show them.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteUser {
    /// Their id in the session.
    pub id: UserId,
    /// Their name.
    pub name: String,
    /// Their colour.
    pub color: [u8; 3],
    /// Their camera: where it glides from, to, and when it was told to.
    pub camera: Option<(CFrame, CFrame, Instant)>,
    /// What they have selected, in the editor's ids.
    pub selection: Vec<Id>,
    /// The scripts they have open, in the editor's ids.
    pub open: Vec<Id>,
}

impl RemoteUser {
    /// Their camera at `now`, between where it was and where it was last sent.
    pub fn camera_at(&self, now: Instant) -> Option<CFrame> {
        let (from, to, at) = self.camera?;
        let t = (now.saturating_duration_since(at).as_secs_f32() / GLIDE.as_secs_f32()).clamp(0.0, 1.0);
        let position: Vec3 = from.position().lerp(to.position(), t);
        let rotation: Quat = from.rotation().slerp(to.rotation(), t);
        Some(CFrame::from_parts(position, rotation))
    }
}

/// Every other user in the session; empty without one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    /// The users, by id (never this Studio's own).
    pub users: Vec<RemoteUser>,
}

impl Shown {
    /// Whether anyone is shown.
    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    /// Updates `user` (adding them), their camera gliding from where it is now to `camera`.
    pub fn update(&mut self, user: &User, camera: Option<CFrame>, selection: Vec<Id>, open: Vec<Id>, now: Instant) {
        let (id, name, color) = (user.id, user.identity.name.as_str(), user.identity.color);
        let index = match self.users.iter().position(|u| u.id == id) {
            Some(index) => index,
            None => {
                self.users.push(RemoteUser {
                    id,
                    name: name.to_owned(),
                    color,
                    camera: None,
                    selection: Vec::new(),
                    open: Vec::new(),
                });
                self.users.sort_by_key(|u| u.id);
                self.users.iter().position(|u| u.id == id).unwrap_or_default()
            }
        };
        let user = &mut self.users[index];
        name.clone_into(&mut user.name);
        user.color = color;
        user.selection = selection;
        user.open = open;
        user.camera = match (camera, user.camera_at(now)) {
            (Some(to), Some(from)) if user.camera.is_some_and(|(_, last, _)| last != to) => Some((from, to, now)),
            (Some(to), None) => Some((to, to, now)),
            (Some(_), Some(_)) => user.camera,
            (None, _) => None,
        };
    }

    /// Keeps only the users in `ids`.
    pub fn retain(&mut self, ids: &[UserId]) {
        self.users.retain(|u| ids.contains(&u.id));
    }

    /// The colours of the users marking `id` in the Explorer: who has it open (a script, `script`) or selected.
    pub fn dots(&self, id: Id, script: bool) -> Vec<[u8; 3]> {
        self.users
            .iter()
            .filter(|u| {
                if script {
                    u.open.contains(&id)
                } else {
                    u.selection.contains(&id)
                }
            })
            .map(|u| u.color)
            .collect()
    }

    /// Whether a camera is still gliding at `now` (the viewport keeps drawing until it lands).
    pub fn gliding(&self, now: Instant) -> bool {
        self.users.iter().any(|u| {
            u.camera
                .is_some_and(|(from, to, at)| from != to && now.duration_since(at) < GLIDE)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cameras_glide_and_dots_follow_what_is_open_or_selected() {
        let mut shown = Shown::default();
        let start = Instant::now();
        let a = CFrame::new(Vec3::ZERO);
        let b = CFrame::new(Vec3::new(10.0, 0.0, 0.0));
        let dm = metallic_engine::metallic_classes::new_game().unwrap();
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let bob = User {
            id: UserId(2),
            identity: metallic_engine::metallic_net::identity::Identity::new("Bob", [1, 2, 3]),
            host: false,
        };
        shown.update(&bob, Some(a), vec![workspace], vec![], start);
        shown.update(&bob, Some(b), vec![workspace], vec![], start);
        let half = shown.users[0].camera_at(start + GLIDE / 2).unwrap();
        assert!((half.position().x - 5.0).abs() < 0.5, "{half:?}");
        assert_eq!(shown.users[0].camera_at(start + GLIDE * 2), Some(b));
        assert_eq!(shown.dots(workspace, false), [[1, 2, 3]]);
        assert!(shown.dots(workspace, true).is_empty());
        shown.retain(&[]);
        assert!(shown.is_empty());
    }
}
