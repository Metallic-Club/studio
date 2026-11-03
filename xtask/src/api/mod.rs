//! `cargo xtask api-dump`: writes the scripting API as `api/api-dump.json` and `api/metallic.d.luau`.
//!
//! The JSON is [`ApiDump`] (built by `metallic_script`, so other tools can reuse it) serialized here, because
//! `serde_json` is reserved for host tools and stays out of the engine. The Luau definitions are generated from the
//! same dump by [`luau::definitions`]. With `--check` nothing is written: both files are generated in memory and
//! compared with the committed ones, and any difference (naming the file and the first differing line) fails, so CI
//! catches a class member changed without regenerating.

mod luau;

use std::fs;
use std::path::{Path, PathBuf};

use metallic_script::ApiDump;

/// The dump, relative to the workspace root.
const JSON_PATH: &str = "api/api-dump.json";

/// The generated Luau definitions, relative to the workspace root.
const LUAU_PATH: &str = "api/metallic.d.luau";

/// Runs the command with the arguments after `api-dump`. `Ok(false)` means `--check` found a stale file.
pub(crate) fn run(args: &[String]) -> Result<bool, String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => {
            return Err(format!(
                "unexpected arguments {args:?}; usage: cargo xtask api-dump [--check]"
            ));
        }
    };
    let files = generate()?;
    let root = workspace_root()?;

    if !check {
        for (path, contents) in &files {
            let full = root.join(path);
            if let Some(dir) = full.parent() {
                fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
            }
            fs::write(&full, contents).map_err(|e| format!("could not write {path}: {e}"))?;
            println!("wrote {path}");
        }
        return Ok(true);
    }

    let mut fresh = true;
    for (path, generated) in &files {
        let committed = match fs::read_to_string(root.join(path)) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("api-dump: {path} could not be read ({e}); run `cargo xtask api-dump`");
                fresh = false;
                continue;
            }
        };
        if let Some(diff) = first_difference(&committed, generated) {
            eprintln!("api-dump: {path} is stale; run `cargo xtask api-dump` and commit the result\n{diff}");
            fresh = false;
        }
    }
    if fresh {
        println!("api-dump: {JSON_PATH} and {LUAU_PATH} are up to date");
    }
    Ok(fresh)
}

/// Both files, as `(path relative to the workspace root, contents)`.
fn generate() -> Result<[(&'static str, String); 2], String> {
    let registry = metallic_classes::shared_registry().map_err(|e| format!("the class registry is invalid: {e}"))?;
    let dump = ApiDump::from_registry(&registry);
    Ok([(JSON_PATH, to_json(&dump)?), (LUAU_PATH, luau::definitions(&dump))])
}

/// The dump as pretty JSON: two-space indent, fields in declaration order, `\n` line endings and a final newline.
fn to_json(dump: &ApiDump) -> Result<String, String> {
    let mut json = serde_json::to_string_pretty(dump).map_err(|e| format!("could not serialize the dump: {e}"))?;
    json.push('\n');
    Ok(json)
}

/// The workspace root: the folder above `xtask/`.
fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask has no parent folder".to_owned())
}

/// Where `committed` and `generated` first differ, as lines for the error message; `None` when they are equal.
/// Line endings are compared as `\n`, so a checkout with CRLF endings is not stale.
fn first_difference(committed: &str, generated: &str) -> Option<String> {
    let committed = committed.replace("\r\n", "\n");
    if committed == generated {
        return None;
    }
    let mut old = committed.split('\n');
    let mut new = generated.split('\n');
    let mut line = 1;
    loop {
        match (old.next(), new.next()) {
            (Some(a), Some(b)) if a == b => line += 1,
            (a, b) => {
                let show = |l: Option<&str>| l.map_or_else(|| "(end of file)".to_owned(), |l| format!("`{l}`"));
                return Some(format!(
                    "  first difference at line {line}:\n    committed: {}\n    generated: {}",
                    show(a),
                    show(b)
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic() {
        let first = generate().unwrap();
        let second = generate().unwrap();
        assert_eq!(first, second);
        for (path, contents) in &first {
            assert!(contents.ends_with('\n') && !contents.ends_with("\n\n"), "{path}");
            assert!(!contents.contains('\r'), "{path}");
        }
    }

    #[test]
    fn every_registry_class_is_in_both_files() {
        let registry = metallic_classes::shared_registry().unwrap();
        let [(_, json), (_, luau)] = generate().unwrap();
        let dump: ApiDump = serde_json::from_str(&json).unwrap();
        assert_eq!(dump, ApiDump::from_registry(&registry), "the JSON reads back unchanged");
        let names: Vec<&str> = dump.classes.iter().map(|c| c.name.as_str()).collect();
        let expected: Vec<&str> = registry.classes().iter().map(|c| c.name()).collect();
        assert_eq!(names, expected);
        for class in registry.classes() {
            let declared = format!("declare extern type {} ", class.name());
            assert!(
                luau.contains(&declared),
                "{} missing from the definitions",
                class.name()
            );
        }
        for desc in registry.enums() {
            assert!(luau.contains(&format!("declare extern type Enum{} extends EnumItem", desc.name)));
        }
    }

    #[test]
    fn json_has_the_format_version_first() {
        let [(_, json), _] = generate().unwrap();
        assert!(json.starts_with(&format!("{{\n  \"format\": {},\n", metallic_script::api_dump::FORMAT)));
    }

    #[test]
    fn first_difference_names_the_line() {
        assert_eq!(first_difference("a\nb\n", "a\nb\n"), None);
        assert_eq!(first_difference("a\r\nb\r\n", "a\nb\n"), None);
        let diff = first_difference("a\nb\nc\n", "a\nB\nc\n").unwrap();
        assert!(diff.contains("line 2"), "{diff}");
        assert!(
            diff.contains("committed: `b`") && diff.contains("generated: `B`"),
            "{diff}"
        );
        let shorter = first_difference("a\nb", "a\nb\nc").unwrap();
        assert!(
            shorter.contains("line 3") && shorter.contains("(end of file)"),
            "{shorter}"
        );
    }
}
