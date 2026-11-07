//! Puts Studio's built-in icons beside the executable (#291): `apps/studio/icons` (the Lucide SVGs, `icons.json` and
//! Lucide's licence) is copied to `icons` in the folder Cargo builds the executable into, so `cargo run`, the tests
//! and a packaged build all find it where Studio looks (`theme::icons::builtin::folder`).

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=icons");
    let (Some(out), Some(manifest)) = (std::env::var_os("OUT_DIR"), std::env::var_os("CARGO_MANIFEST_DIR")) else {
        return;
    };
    // OUT_DIR is <target>/<profile>/build/<package>-<hash>/out; the executable is built into <target>/<profile>.
    let Some(profile) = Path::new(&out).ancestors().nth(3) else {
        println!("cargo:warning=icons: could not find the executable's folder from OUT_DIR");
        return;
    };
    let from = Path::new(&manifest).join("icons");
    let to = profile.join("icons");
    // A fresh copy, so an icon removed from the source is gone beside the executable too.
    let _ = std::fs::remove_dir_all(&to);
    if let Err(error) = copy(&from, &to) {
        println!(
            "cargo:warning=icons: could not copy {} to {}: {error}",
            from.display(),
            to.display()
        );
    }
}

/// Copies the folder `from` to `to`, sub-folders included.
fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
