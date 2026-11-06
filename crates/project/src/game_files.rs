//! [`GameFiles`]: one way to read a game, whether from its source project or from its build.

use std::path::Path;

use metallic_core::DataModel;

use crate::asset_index::AssetIndex;
use crate::assets::AssetReader;
use crate::manifest::Manifest;
use crate::project::ProjectError;
use crate::scene::{LoadedScene, load_scene_bytes};

/// A game's files, read by member path: an open `.mtproj` ([`Project`](crate::Project), the source) or a `game.pak`
/// ([`Pak`](crate::Pak), the build). The engine loads a game through this, so it runs the same from either.
pub trait GameFiles {
    /// The file the game is read from.
    fn path(&self) -> &Path;

    /// The project settings (`metallic.toml`).
    fn manifest(&self) -> &Manifest;

    /// Every member's path, sorted.
    fn members(&self) -> Vec<&str>;

    /// The contents of `member`.
    fn read(&mut self, member: &str) -> Result<Vec<u8>, ProjectError>;

    /// The asset index: a stable id for every asset member.
    fn assets(&self) -> &AssetIndex;

    /// A reader of the game's assets by id that works on any thread ([`AssetReader`]), for decoding meshes and images
    /// off the frame.
    fn asset_reader(&self) -> AssetReader;

    /// [`GameFiles::read`] for a text member (a script, a scene), which must be UTF-8.
    fn read_text(&mut self, member: &str) -> Result<String, ProjectError> {
        String::from_utf8(self.read(member)?).map_err(|_| ProjectError::NotText {
            path: self.path().to_path_buf(),
            member: member.to_owned(),
        })
    }

    /// Loads the start scene ([`Manifest::start_scene`]) into `dm`, which must be a whole game (see
    /// [`load_scene_bytes`]). On an error `dm` is unchanged.
    fn load_start_scene(&mut self, dm: &mut DataModel) -> Result<LoadedScene, ProjectError> {
        let scene = self.manifest().start_scene.clone();
        let bytes = self.read(&scene).map_err(|error| match error {
            ProjectError::MissingMember { path, member } => ProjectError::MissingStartScene { path, scene: member },
            other => other,
        })?;
        load_scene_bytes(dm, &bytes).map_err(|source| ProjectError::Scene {
            path: self.path().to_path_buf(),
            scene,
            source,
        })
    }
}
