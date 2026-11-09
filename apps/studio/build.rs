//! Builds Studio's shipped icons into the executable (#402): every file of `apps/studio/icons` (the Lucide SVGs,
//! `icons.json` and Lucide's licence) is listed in `$OUT_DIR/shipped_icons.rs` as its path in the folder and its bytes
//! (`include_bytes!`), which `theme::icons::builtin::SHIPPED` includes. So Studio has its icons wherever its executable
//! goes, with nothing beside it.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=icons");
    let (Some(out), Some(manifest)) = (std::env::var_os("OUT_DIR"), std::env::var_os("CARGO_MANIFEST_DIR")) else {
        return;
    };
    let root = Path::new(&manifest).join("icons");
    let mut files = Vec::new();
    if let Err(error) = list(&root, "", &mut files) {
        println!("cargo:warning=icons: could not read {}: {error}", root.display());
    }
    files.sort();
    let mut code = String::from("&[\n");
    for (path, full) in &files {
        // Debug quoting escapes the backslashes of a Windows path.
        let _ = writeln!(code, "    ({path:?}, include_bytes!({full:?})),");
    }
    code.push_str("]\n");
    let target = Path::new(&out).join("shipped_icons.rs");
    if let Err(error) = std::fs::write(&target, code) {
        println!("cargo:warning=icons: could not write {}: {error}", target.display());
    }
}

/// Every file under `dir`, as its path from the icons folder (`/` between folders, `prefix` before it) and its full
/// path.
fn list(dir: &Path, prefix: &str, files: &mut Vec<(String, String)>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}{name}");
        if entry.file_type()?.is_dir() {
            list(&entry.path(), &format!("{path}/"), files)?;
        } else {
            files.push((path, entry.path().to_string_lossy().into_owned()));
        }
    }
    Ok(())
}
