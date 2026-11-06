//! Mesh-asset kinds (#189): which mesh kind draws which mesh asset and texture.
//!
//! Draws are grouped by mesh kind (ADR 0008 §4), and a mesh asset's draw also binds its texture, so every pair of a
//! mesh and a texture that parts name (`MeshId`, `TextureId`, through the `AssetMesh` component) is a kind of its
//! own, numbered from [`MeshKind::BUILTIN_COUNT`] up in the order the scene first meets them. A kind keeps its number
//! for the scene's life, even when no part names it any more, so its draw list never moves. The renderer draws a kind
//! once its mesh is here ([`crate::Renderer::add_mesh_asset`]); until then the kind's parts are listed, culled and
//! counted, but draw nothing.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::instance::MeshKind;

/// The most mesh-asset kinds a scene has: parts naming a pair of a mesh and a texture beyond it draw nothing. Each kind
/// has a draw list and a draw.
pub const MAX_MESH_ASSET_KINDS: usize = 4096;

/// The mesh and texture a mesh-asset kind draws: the keys parts name them by (`MeshId`, `TextureId`), the renderer's
/// keys for [`crate::Renderer::add_mesh_asset`] and [`crate::Renderer::add_texture_asset`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MeshAssetKey {
    /// The mesh asset's key (never empty).
    pub mesh: Arc<str>,
    /// The texture asset's key; empty for none.
    pub texture: Arc<str>,
}

/// A revision no other change, in any scene of the process, has had, so a renderer never mistakes one scene's kinds
/// for another's.
fn next_revision() -> u64 {
    static REVISION: AtomicU64 = AtomicU64::new(1);
    REVISION.fetch_add(1, Ordering::Relaxed)
}

/// A scene's mesh-asset kinds.
#[derive(Debug)]
pub(crate) struct AssetKinds {
    /// Kind `BUILTIN_COUNT + i` draws `keys[i]`.
    keys: Vec<MeshAssetKey>,
    /// The kinds of each mesh, by texture.
    by_mesh: HashMap<Arc<str>, Vec<(Arc<str>, MeshKind)>>,
    revision: u64,
}

impl Default for AssetKinds {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            by_mesh: HashMap::new(),
            revision: next_revision(),
        }
    }
}

impl AssetKinds {
    /// The kind that draws `mesh` with `texture`, numbered now when it is new; [`MeshKind::NONE`] (drawn by nothing)
    /// for no mesh, or past [`MAX_MESH_ASSET_KINDS`]. Allocates only for a new pair.
    pub(crate) fn kind(&mut self, mesh: &str, texture: &str) -> MeshKind {
        if mesh.is_empty() {
            return MeshKind::NONE;
        }
        if let Some(&(_, kind)) = self
            .by_mesh
            .get(mesh)
            .and_then(|kinds| kinds.iter().find(|(t, _)| &**t == texture))
        {
            return kind;
        }
        if self.keys.len() >= MAX_MESH_ASSET_KINDS {
            return MeshKind::NONE;
        }
        let kind = MeshKind(MeshKind::BUILTIN_COUNT + self.keys.len() as u16);
        let key = MeshAssetKey {
            mesh: Arc::from(mesh),
            texture: Arc::from(texture),
        };
        self.by_mesh
            .entry(Arc::clone(&key.mesh))
            .or_default()
            .push((Arc::clone(&key.texture), kind));
        self.keys.push(key);
        self.revision = next_revision();
        kind
    }

    /// Every kind's mesh and texture: kind `MeshKind::BUILTIN_COUNT + i` draws the `i`th.
    pub(crate) fn keys(&self) -> &[MeshAssetKey] {
        &self.keys
    }

    /// Changes whenever a kind is added (and differs between scenes).
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_pair_is_one_kind_numbered_after_the_primitives() {
        let mut kinds = AssetKinds::default();
        let start = kinds.revision();
        let a = kinds.kind("mesh-a", "");
        assert_eq!(a, MeshKind(MeshKind::BUILTIN_COUNT));
        let after_a = kinds.revision();
        assert_ne!(after_a, start);
        assert_eq!(kinds.kind("mesh-a", ""), a, "the same pair, the same kind");
        assert_eq!(kinds.revision(), after_a, "nothing new");
        let textured = kinds.kind("mesh-a", "tex");
        let b = kinds.kind("mesh-b", "tex");
        assert_eq!(textured, MeshKind(MeshKind::BUILTIN_COUNT + 1));
        assert_eq!(b, MeshKind(MeshKind::BUILTIN_COUNT + 2));
        assert_eq!(kinds.kind("", "tex"), MeshKind::NONE, "no mesh draws nothing");
        assert_eq!(kinds.keys().len(), 3);
        assert_eq!(&*kinds.keys()[1].texture, "tex");
        assert_ne!(
            AssetKinds::default().revision(),
            kinds.revision(),
            "scenes never share one"
        );
    }

    #[test]
    fn kinds_stop_at_the_limit() {
        let mut kinds = AssetKinds::default();
        for i in 0..MAX_MESH_ASSET_KINDS {
            assert_ne!(kinds.kind(&i.to_string(), ""), MeshKind::NONE);
        }
        assert_eq!(kinds.kind("one more", ""), MeshKind::NONE);
        assert_ne!(kinds.kind("0", ""), MeshKind::NONE, "existing kinds still resolve");
    }
}
