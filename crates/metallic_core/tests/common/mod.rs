//! Helpers shared by the core integration tests.

use std::path::PathBuf;

/// A temporary folder owned by one test and deleted when it drops.
///
/// Every call makes a new, uniquely named folder, so tests running at the same time (in this
/// process, in another `cargo test`, or in another worktree on the same PC) never share files.
pub struct TestDir {
    dir: tempfile::TempDir,
}

impl TestDir {
    /// A fresh folder whose name starts with `metallic_core-<test>-`.
    pub fn new(test: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("metallic_core-{test}-"))
            .tempdir()
            .expect("create a temporary test folder");
        Self { dir }
    }

    /// A path for `name` inside the folder (the file is not created).
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}
