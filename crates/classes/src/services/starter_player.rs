//! `StarterPlayer`: what each player starts with, as Roblox's, with its two built-in folders:
//!
//! - `StarterPlayerScripts`: `LocalScript`s that run on each player's client when the game starts. Roblox copies
//!   them into each player's `PlayerScripts`; with no `Players` yet, they run where they are.
//! - `StarterCharacterScripts`: scripts Roblox copies into each player's character when it spawns. There are no
//!   characters yet, so nothing in it runs.
//!
//! The folders are made with the service ([`ClassDesc::builtin_children`]): they cannot be moved, deleted or made
//! by scripts.

use metallic_core::ClassDesc;

/// What players start with.
pub static CLASS: ClassDesc = ClassDesc::new("StarterPlayer", Some("Instance"))
    .describe(
        "What each player starts with. Its StarterPlayerScripts run on each player's client; its \
         StarterCharacterScripts go into each player's character.",
    )
    .service(7)
    .persisted()
    // Roblox lists StarterCharacterScripts first (#827).
    .builtin_children(&["StarterCharacterScripts", "StarterPlayerScripts"]);

#[cfg(test)]
mod tests {
    use metallic_core::{DmError, Id};

    use crate::testing::game;

    #[test]
    fn it_is_made_with_its_folders_which_stay() {
        let mut dm = game();
        let class = dm.registry().class_id("StarterPlayer").unwrap();
        let service = dm.service(class).unwrap();
        let folders: Vec<Id> = dm.children(service).unwrap().collect();
        let names: Vec<&str> = folders.iter().map(|&f| dm.class_name(f).unwrap()).collect();
        assert_eq!(names, ["StarterCharacterScripts", "StarterPlayerScripts"]);
        assert_eq!(dm.name(folders[1]), Ok("StarterPlayerScripts"));
        let scripts = folders[1];
        assert!(dm.is_fixed(scripts));
        assert!(matches!(dm.destroy(scripts), Err(DmError::IsBuiltin { .. })));
        assert!(matches!(dm.set_parent(scripts, None), Err(DmError::IsBuiltin { .. })));
        assert!(matches!(dm.clone_tree(scripts), Err(DmError::IsBuiltin { .. })));
        dm.destroy_children(service).unwrap();
        assert_eq!(dm.children(service).unwrap().count(), 2);
        // Scripts cannot make more.
        let folder = dm.registry().class_id("StarterPlayerScripts").unwrap();
        assert!(matches!(dm.create(folder), Err(DmError::NotCreatable { .. })));
    }
}
