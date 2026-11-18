//! The Assets panel's rendered thumbnails (#681): a material's tile shows a sphere with the material, a scene's tile
//! the scene from its camera. The tiles ask for them ([`Pictures::want`]); the thumbnail renderer
//! (`app/thumbnails.rs`) draws one a frame and hands it back ([`Pictures::done`]). Each is kept with the content of
//! the file it was drawn from ([`Fingerprint`]), so an edited material or a saved scene is drawn again. Nothing is
//! written into the project: the pictures live in memory, made again when Studio opens the project next.

use std::collections::HashMap;

use metallic_engine::metallic_project::{AssetId, Fingerprint};

/// What a thumbnail shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    /// A material, on a sphere.
    Material,
    /// A scene, from its camera.
    Scene,
    /// A mesh's model (#806), whole.
    Mesh,
}

/// A thumbnail asked for: the asset, what it is, the member it is drawn from and that member's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    /// The asset.
    pub id: AssetId,
    /// What it shows.
    pub picture: Picture,
    /// Its member's path.
    pub path: String,
    /// Its member's content when it was asked for.
    pub fingerprint: Fingerprint,
}

/// The rendered thumbnails, and those asked for.
#[derive(Debug, Default)]
pub struct Pictures {
    done: HashMap<AssetId, (Fingerprint, egui::TextureId)>,
    wanted: Vec<Wanted>,
}

impl Pictures {
    /// The thumbnail of `id`, if one was drawn: and whether it shows `fingerprint`'s content (else it is old, and one
    /// is asked for).
    pub fn get(&self, id: AssetId, fingerprint: Fingerprint) -> Option<(egui::TextureId, bool)> {
        self.done
            .get(&id)
            .map(|(drawn, texture)| (*texture, *drawn == fingerprint))
    }

    /// Asks for the thumbnail of `wanted`, once.
    pub fn want(&mut self, wanted: Wanted) {
        if let Some(asked) = self.wanted.iter_mut().find(|asked| asked.id == wanted.id) {
            *asked = wanted;
        } else {
            self.wanted.push(wanted);
        }
    }

    /// The next thumbnail to draw, taken from those asked for.
    pub fn take_next(&mut self) -> Option<Wanted> {
        (!self.wanted.is_empty()).then(|| self.wanted.remove(0))
    }

    /// Keeps `texture` as `id`'s thumbnail, drawn from `fingerprint`'s content.
    pub fn done(&mut self, id: AssetId, fingerprint: Fingerprint, texture: egui::TextureId) {
        self.done.insert(id, (fingerprint, texture));
    }

    /// Whether `id` has a thumbnail's texture already, to draw a newer one into.
    pub fn texture_of(&self, id: AssetId) -> Option<egui::TextureId> {
        self.done.get(&id).map(|(_, texture)| *texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wanted(id: u128, size: u64) -> Wanted {
        Wanted {
            id: AssetId::from_u128(id),
            picture: Picture::Material,
            path: format!("Assets/{id}.material"),
            fingerprint: Fingerprint { size, crc32: 0 },
        }
    }

    #[test]
    fn a_thumbnail_is_asked_for_once_and_is_old_once_its_file_changes() {
        let mut pictures = Pictures::default();
        pictures.want(wanted(1, 10));
        pictures.want(wanted(1, 11));
        pictures.want(wanted(2, 10));
        let first = pictures.take_next().unwrap();
        assert_eq!(
            (first.id, first.fingerprint.size),
            (AssetId::from_u128(1), 11),
            "the newest ask"
        );
        assert_eq!(pictures.take_next().unwrap().id, AssetId::from_u128(2));
        assert_eq!(pictures.take_next(), None);

        let texture = egui::TextureId::User(7);
        pictures.done(first.id, first.fingerprint, texture);
        assert_eq!(pictures.get(first.id, first.fingerprint), Some((texture, true)));
        let edited = Fingerprint { size: 12, crc32: 0 };
        assert_eq!(
            pictures.get(first.id, edited),
            Some((texture, false)),
            "shown until a new one is drawn"
        );
    }
}
