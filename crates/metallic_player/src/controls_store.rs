//! Loading and saving the player's input overrides for the game being played: one file per game
//! and local profile under the Player's data folder ([`crate::data_dir`]). The file is read once
//! at start and written when the Controls page closes and when the Player exits, never per frame.
use metallic_runtime::input::overrides::{LOCAL_PROFILE, LoadStatus, game_id};
use metallic_runtime::input::{InputRuntime, OverrideStore, PlayerOverrides};

/// The overrides of one game.
pub struct Controls {
    store: Option<OverrideStore>,
    loaded: PlayerOverrides,
    /// What is on disk now (to skip writes that change nothing).
    saved: PlayerOverrides,
}

impl Controls {
    /// No overrides and nothing saved (`--no-overrides`, or no data folder).
    pub fn disabled() -> Self {
        Self {
            store: None,
            loaded: PlayerOverrides::default(),
            saved: PlayerOverrides::default(),
        }
    }

    /// Opens the overrides of the game called `game_name` and reads them; problems are reported
    /// on stderr as warnings and leave the game's own controls in place.
    pub fn open(game_name: &str) -> Self {
        let Some(dir) = crate::data_dir::data_dir() else {
            eprintln!("warning: input: no data folder; control changes will not be saved");
            return Self::disabled();
        };
        let store = OverrideStore::new(&dir, &game_id(game_name), LOCAL_PROFILE);
        let (loaded, status) = store.load();
        if let LoadStatus::Corrupt { reason, moved_to } = &status {
            match moved_to {
                Some(_) => eprintln!(
                    "warning: input: the saved controls could not be read ({reason}); they were moved aside and the game's own controls apply"
                ),
                None => eprintln!(
                    "warning: input: the saved controls could not be read ({reason}); the game's own controls apply"
                ),
            }
        }
        Self {
            store: Some(store),
            saved: loaded.clone(),
            loaded,
        }
    }

    /// Applies the loaded overrides to `rt`.
    pub fn apply(&self, rt: &mut InputRuntime) {
        if self.store.is_some() {
            rt.set_overrides(self.loaded.clone());
        }
    }

    /// Writes `o` when it differs from what is on disk.
    pub fn save(&mut self, o: &PlayerOverrides) {
        let Some(store) = &self.store else { return };
        if *o == self.saved {
            return;
        }
        match store.save(o) {
            Ok(()) => self.saved = o.clone(),
            Err(e) => eprintln!("warning: input: could not save the controls: {e}"),
        }
    }
}
