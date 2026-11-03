//! `SourceContainer`: the base of every script class. It points at a source file in the project.
//!
//! ```text
//! Instance ─► SourceContainer ─► BaseScript ─► Script | LocalScript
//!                      └──────► ModuleScript
//! ```
//!
//! The base is `SourceContainer`, not `LuaSourceContainer`, because scripting is language-neutral (ADR 0003):
//! `Source` holds a project-relative path such as `Scripts/Door.luau`, not the code, and the file's extension picks
//! the language. Running scripts and `require` belong to the script host (`metallic_script`); these classes are
//! only data.

use metallic_core::{ClassDesc, PropMeta};

/// The category script classes are listed under in Studio's Insert menu.
pub const CATEGORY: &str = "Scripts";

/// The Properties panel group of the script members.
pub const GROUP: &str = "Script";

// Studio should edit `Source` with a project file picker; core has no such widget yet, so the description says it.
static PROPS: &[PropMeta] = &[PropMeta::string("Source", "").group(GROUP).order(0).desc(
    "The project-relative path of the script's source file, such as \"Scripts/Door.luau\", chosen with a file \
     picker. The file's extension selects the language. Empty means the script has no source and does nothing.",
)];

/// The base of every script class.
pub static CLASS: ClassDesc = ClassDesc::new("SourceContainer", Some("Instance"))
    .describe(
        "The base of every script: an instance that points at a source file in the project. The file's extension \
         selects the language, so a game can mix languages.",
    )
    .props(PROPS);

#[cfg(test)]
mod tests {
    use metallic_core::Value;

    use crate::shared_registry;
    use crate::testing::{game, new_in, workspace};

    /// Each script class, its ancestors from `Object` down, and whether scripts and Studio can create it.
    const CLASSES: [(&str, &[&str], bool); 5] = [
        ("SourceContainer", &["Object", "Instance", "SourceContainer"], false),
        ("BaseScript", &["Object", "Instance", "SourceContainer", "BaseScript"], false),
        (
            "Script",
            &["Object", "Instance", "SourceContainer", "BaseScript", "Script"],
            true,
        ),
        (
            "LocalScript",
            &["Object", "Instance", "SourceContainer", "BaseScript", "LocalScript"],
            true,
        ),
        (
            "ModuleScript",
            &["Object", "Instance", "SourceContainer", "ModuleScript"],
            true,
        ),
    ];

    #[test]
    fn script_classes_resolve_with_their_chain() {
        let registry = shared_registry().unwrap();
        for (name, chain, creatable) in CLASSES {
            let class = registry.class_id(name).unwrap_or_else(|| panic!("{name}"));
            let info = registry.class(class).unwrap();
            let ancestors: Vec<&str> = info
                .ancestors()
                .iter()
                .map(|&c| registry.class(c).unwrap().name())
                .collect();
            assert_eq!(ancestors, chain, "{name}");
            assert_eq!(info.creatable(), creatable, "{name}");
            assert!(registry.prop_id(class, "Source").is_some(), "{name}.Source");
        }
        let module = registry.class_id("ModuleScript").unwrap();
        assert!(registry.prop_id(module, "Enabled").is_none(), "a module runs when required");
        for name in ["Script", "LocalScript", "ModuleScript"] {
            let info = registry.class(registry.class_id(name).unwrap()).unwrap();
            assert_eq!(info.desc().category, super::CATEGORY, "{name}");
        }
    }

    #[test]
    fn source_and_enabled_have_their_defaults() {
        let mut dm = game();
        let ws = workspace(&dm);
        for class in ["Script", "LocalScript", "ModuleScript"] {
            let script = new_in(&mut dm, class, ws, class);
            assert_eq!(
                dm.get_by_name(script, "Source"),
                Ok(Value::String("".into())),
                "{class}"
            );
        }
        for class in ["Script", "LocalScript"] {
            let script = new_in(&mut dm, class, ws, class);
            assert_eq!(dm.get_by_name(script, "Enabled"), Ok(Value::Bool(true)), "{class}");
        }
    }

    #[test]
    fn source_holds_a_path() {
        let mut dm = game();
        let ws = workspace(&dm);
        let script = new_in(&mut dm, "Script", ws, "Door");
        let path = Value::String("Scripts/Door.luau".into());
        dm.set_by_name(script, "Source", path.clone()).unwrap();
        assert_eq!(dm.get_by_name(script, "Source"), Ok(path));
        assert!(dm.set_by_name(script, "Source", Value::Bool(true)).is_err());
    }
}
