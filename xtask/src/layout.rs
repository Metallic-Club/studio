//! `cargo xtask check-layout`: enforces the crate boundaries from `docs/adr/0001-project-layout.md`, and that
//! nothing on the frame path waits for the GPU (ADR 0008 §9, [`GPU_WAIT_ALLOWED`]).
//!
//! The rules are pure functions over package lists and file contents, so they are tested without running cargo.
//! [`run`] is the thin shell that feeds them `cargo metadata` and the source tree.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// Which workspace packages each workspace package may depend on (normal and build dependencies).
///
/// This table mirrors "Who depends on whom" in `docs/adr/0001-project-layout.md`. Change both together: a new
/// crate goes into the ADR first, then here.
const ALLOWED_WORKSPACE_DEPS: &[(&str, &[&str])] = &[
    ("metallic_core", &[]),
    ("metallic_platform", &[]),
    ("metallic_classes", &["metallic_core"]),
    ("metallic_script", &["metallic_core"]),
    ("metallic_render", &["metallic_core"]),
    ("metallic_physics", &["metallic_core"]),
    ("metallic_project", &["metallic_core"]),
    ("metallic_net", &["metallic_core"]),
    ("metallic_luau", &["metallic_core", "metallic_script"]),
    (
        "metallic_engine",
        &[
            "metallic_core",
            "metallic_classes",
            "metallic_script",
            "metallic_project",
            "metallic_platform",
            "metallic_render",
            "metallic_physics",
            "metallic_luau",
            "metallic_net",
        ],
    ),
    ("metallic_player", &["metallic_engine"]),
    ("metallic_studio", &["metallic_engine", "metallic_player"]),
    ("metallic_launcher", &["metallic_player"]),
    // Host tooling that never ships: the API dump (#48) reads the class set through `metallic_script`'s dump builder.
    ("xtask", &["metallic_core", "metallic_classes", "metallic_script"]),
];

/// The only crates.io crates `metallic_core` may depend on (ADR 0001 rule 2: the object model stays pure).
const CORE_ALLOWED_EXTERNAL: &[&str] = &["glam", "serde", "thiserror"];

/// The package the external allow-list applies to.
const CORE_PACKAGE: &str = "metallic_core";

/// Top-level folders, relative to the workspace root, where `cfg(target_os ...)` is allowed (ADR 0001 rule 5).
/// `xtask` is a host-only tool that never ships, and its own rule text would otherwise match itself.
const TARGET_OS_ALLOWED_DIRS: &[&str] = &["crates/platform", "apps", "xtask"];

/// The source folders of the frame path, where nothing may wait for the GPU (ADR 0008 §9): the renderer and the
/// engine that drives it. Their tests may wait (they read frames back); the scan stops at a file's `#[cfg(test)]`.
const FRAME_PATH_DIRS: &[&str] = &["crates/render/src/", "crates/engine/src/"];

/// Calls that wait for the GPU, or start a readback that only a wait completes: `device.poll` with
/// `PollType::Wait` (or `wait_indefinitely`), `pollster::block_on` of a wgpu future, and `map_async`.
const GPU_WAITS: &[&str] = &["PollType::Wait", "wait_indefinitely", "block_on(", ".map_async("];

/// The uses of [`GPU_WAITS`] the frame path may make, each with why it never stalls a frame: `(file, call, why)`.
pub(crate) const GPU_WAIT_ALLOWED: &[(&str, &str, &str)] = &[
    (
        "crates/render/src/context.rs",
        "block_on(",
        "the adapter and device requests, once at setup, before any frame",
    ),
    (
        "crates/render/src/timing.rs",
        ".map_async(",
        "the timestamp readback: mapped asynchronously and collected frames later with PollType::Poll, never waited on",
    ),
];

/// The subset of `cargo metadata` output the checks need.
#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_root: PathBuf,
}

/// One workspace package and the dependencies its manifest declares.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Package {
    /// Package name, e.g. `metallic_core`.
    pub(crate) name: String,
    /// Every declared dependency, of every kind and for every target.
    pub(crate) dependencies: Vec<Dependency>,
}

/// One dependency as declared in a manifest.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Dependency {
    /// The depended-on package's real name (not a `package = ...` rename).
    pub(crate) name: String,
    /// `None` for a normal dependency.
    pub(crate) kind: Option<DepKind>,
}

/// A non-normal dependency kind, as `cargo metadata` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DepKind {
    /// `[dev-dependencies]`: exempt from the layout rules (tests may use anything).
    Dev,
    /// `[build-dependencies]`: checked like normal dependencies.
    Build,
}

/// One broken rule, printed as a single line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Violation {
    /// A workspace package depends on a workspace package the ADR does not allow.
    ForbiddenWorkspaceDep {
        /// The depending package.
        package: String,
        /// The workspace package it may not depend on.
        dependency: String,
        /// What the table allows instead.
        allowed: &'static [&'static str],
    },
    /// A workspace package that is not in [`ALLOWED_WORKSPACE_DEPS`].
    UnknownPackage {
        /// The package's name.
        package: String,
    },
    /// `metallic_core` depends on a crate outside [`CORE_ALLOWED_EXTERNAL`].
    ForbiddenCoreDep {
        /// The external crate.
        dependency: String,
    },
    /// `cfg(target_os ...)` outside the folders that may contain platform code.
    TargetOsOutsidePlatform {
        /// File path relative to the workspace root, with `/` separators.
        file: String,
        /// 1-based line number.
        line: usize,
    },
    /// A call that waits for the GPU on the frame path, not in [`GPU_WAIT_ALLOWED`].
    GpuWait {
        /// File path relative to the workspace root, with `/` separators.
        file: String,
        /// 1-based line number.
        line: usize,
        /// The call found.
        call: &'static str,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForbiddenWorkspaceDep {
                package,
                dependency,
                allowed,
            } => {
                let allowed = if allowed.is_empty() {
                    "no workspace crates".to_owned()
                } else {
                    allowed.join(", ")
                };
                write!(
                    f,
                    "[crate-deps] `{package}` depends on `{dependency}`, which ADR 0001 does not allow \
                     (`{package}` may depend on: {allowed})"
                )
            }
            Self::UnknownPackage { package } => write!(
                f,
                "[crate-deps] unknown workspace package `{package}`: add it to the table in \
                 xtask/src/layout.rs and to ADR 0001"
            ),
            Self::ForbiddenCoreDep { dependency } => write!(
                f,
                "[core-deps] `{CORE_PACKAGE}` depends on `{dependency}`; it may only use {} (ADR 0001 rule 2)",
                CORE_ALLOWED_EXTERNAL.join(", ")
            ),
            Self::TargetOsOutsidePlatform { file, line } => write!(
                f,
                "[target-os] {file}:{line}: `cfg(target_os ...)` is only allowed in crates/platform and apps/ \
                 (ADR 0001 rule 5)"
            ),
            Self::GpuWait { file, line, call } => write!(
                f,
                "[gpu-wait] {file}:{line}: `{call}` waits for the GPU on the frame path (ADR 0008 §9); read back \
                 asynchronously, or add it to GPU_WAIT_ALLOWED in xtask/src/layout.rs with why it never stalls a frame"
            ),
        }
    }
}

/// Runs every layout check on the workspace that contains the current directory and prints the result.
///
/// Returns `Ok(true)` when the layout is clean, `Ok(false)` when rules are broken, and `Err` when the checks
/// could not run at all (cargo missing, unreadable files).
pub(crate) fn run() -> Result<bool, String> {
    let metadata = cargo_metadata()?;
    let mut violations = check_packages(&metadata.packages);
    violations.extend(scan_sources(&metadata.workspace_root)?);

    if violations.is_empty() {
        println!("layout OK");
        return Ok(true);
    }
    for violation in &violations {
        eprintln!("check-layout: {violation}");
    }
    eprintln!(
        "check-layout: {} violation(s); the rules are in docs/adr/0001-project-layout.md",
        violations.len()
    );
    Ok(false)
}

/// Reads the workspace packages. `--no-deps` lists only workspace members, which is all the rules look at, and
/// needs no network or dependency resolution.
fn cargo_metadata() -> Result<Metadata, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("could not run `cargo metadata`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo metadata` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("could not parse `cargo metadata` output: {e}"))
}

/// Checks the workspace dependency table and `metallic_core`'s external allow-list.
///
/// `packages` must be exactly the workspace members: a dependency counts as a workspace dependency when its name
/// is one of theirs.
pub(crate) fn check_packages(packages: &[Package]) -> Vec<Violation> {
    let workspace: BTreeSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    let mut violations = Vec::new();

    for package in packages {
        // Dev-dependencies are exempt: a test may pull in a higher crate to drive the one under test.
        let checked = package.dependencies.iter().filter(|d| d.kind != Some(DepKind::Dev));

        let Some(allowed) = allowed_workspace_deps(&package.name) else {
            violations.push(Violation::UnknownPackage {
                package: package.name.clone(),
            });
            continue;
        };

        // A crate can list the same dependency twice (normal and build, or per target); report it once.
        let mut seen = BTreeSet::new();
        for dep in checked {
            let name = dep.name.as_str();
            if !seen.insert(name) {
                continue;
            }
            if workspace.contains(name) {
                if !allowed.contains(&name) {
                    violations.push(Violation::ForbiddenWorkspaceDep {
                        package: package.name.clone(),
                        dependency: name.to_owned(),
                        allowed,
                    });
                }
            } else if package.name == CORE_PACKAGE && !CORE_ALLOWED_EXTERNAL.contains(&name) {
                violations.push(Violation::ForbiddenCoreDep {
                    dependency: name.to_owned(),
                });
            }
        }
    }
    violations
}

/// The workspace packages `package` may depend on, or `None` if the table does not know it.
fn allowed_workspace_deps(package: &str) -> Option<&'static [&'static str]> {
    ALLOWED_WORKSPACE_DEPS
        .iter()
        .find(|(name, _)| *name == package)
        .map(|(_, allowed)| *allowed)
}

/// Scans every `.rs` file under `root` that is subject to the `target_os` rule.
fn scan_sources(root: &Path) -> Result<Vec<Violation>, String> {
    let mut files = Vec::new();
    collect_rs_files(root, root, &mut files)?;
    files.sort();

    let mut violations = Vec::new();
    for (path, rel) in files {
        let contents = fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        violations.extend(scan_file(&rel, &contents));
        violations.extend(scan_gpu_waits(&rel, &contents));
    }
    Ok(violations)
}

/// Finds the calls of [`GPU_WAITS`] in a frame-path file ([`FRAME_PATH_DIRS`]) that [`GPU_WAIT_ALLOWED`] does not
/// list. Line comments are skipped, and so is everything from a `#[cfg(test)]` on and whole test-module files
/// (`tests.rs`): tests read frames back.
pub(crate) fn scan_gpu_waits(rel_path: &str, contents: &str) -> Vec<Violation> {
    if !FRAME_PATH_DIRS.iter().any(|dir| rel_path.starts_with(dir)) || rel_path.ends_with("/tests.rs") {
        return Vec::new();
    }
    let mut violations = Vec::new();
    for (i, line) in contents.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with("#[cfg(test)]") {
            break;
        }
        if code.starts_with("//") {
            continue;
        }
        for &call in GPU_WAITS {
            let allowed = GPU_WAIT_ALLOWED
                .iter()
                .any(|&(file, allowed, _)| file == rel_path && allowed == call);
            if code.contains(call) && !allowed {
                violations.push(Violation::GpuWait {
                    file: rel_path.to_owned(),
                    line: i + 1,
                    call,
                });
            }
        }
    }
    violations
}

/// Collects `(absolute path, workspace-relative path)` for each `.rs` file the `target_os` rule applies to.
fn collect_rs_files(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("could not list {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not list {}: {e}", dir.display()))?;
        let path = entry.path();
        let Some(rel) = relative_slash_path(root, &path) else {
            continue;
        };
        let file_type = entry
            .file_type()
            .map_err(|e| format!("could not stat {}: {e}", path.display()))?;
        if file_type.is_dir() {
            if is_scanned_dir(&rel) {
                collect_rs_files(root, &path, out)?;
            }
        } else if file_type.is_file() && rel.ends_with(".rs") {
            out.push((path, rel));
        }
    }
    Ok(())
}

/// `path` relative to `root`, with `/` separators on every OS.
fn relative_slash_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    Some(parts.join("/"))
}

/// Whether the source scan descends into the directory at `rel` (workspace-relative, `/`-separated).
pub(crate) fn is_scanned_dir(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    // Build output, and hidden folders (`.git`, and `.claude/worktrees`, which holds whole other checkouts).
    if name == "target" || name.starts_with('.') {
        return false;
    }
    !TARGET_OS_ALLOWED_DIRS.contains(&rel)
}

/// Finds `cfg(target_os` / `target_os =` in one file's contents. Line comments are skipped so docs can name the
/// rule.
pub(crate) fn scan_file(rel_path: &str, contents: &str) -> Vec<Violation> {
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//") && mentions_target_os(line))
        .map(|(i, _)| Violation::TargetOsOutsidePlatform {
            file: rel_path.to_owned(),
            line: i + 1,
        })
        .collect()
}

/// Whether a line contains `cfg(target_os` or `target_os` followed by `=` (as in `cfg(any(target_os = ...))`).
fn mentions_target_os(line: &str) -> bool {
    if line.contains("cfg(target_os") {
        return true;
    }
    line.match_indices("target_os").any(|(at, word)| {
        line[at + word.len()..]
            .trim_start()
            .strip_prefix('=')
            .is_some_and(|rest| !rest.starts_with('='))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, deps: &[(&str, Option<DepKind>)]) -> Package {
        Package {
            name: name.to_owned(),
            dependencies: deps
                .iter()
                .map(|&(name, kind)| Dependency {
                    name: name.to_owned(),
                    kind,
                })
                .collect(),
        }
    }

    /// The workspace as it is today, built from the table itself.
    fn current_workspace() -> Vec<Package> {
        ALLOWED_WORKSPACE_DEPS
            .iter()
            .map(|(name, allowed)| pkg(name, &allowed.iter().map(|d| (*d, None)).collect::<Vec<_>>()))
            .collect()
    }

    #[test]
    fn allowed_layout_passes() {
        assert_eq!(check_packages(&current_workspace()), vec![]);
    }

    #[test]
    fn subsystem_depending_on_subsystem_is_reported() {
        let mut packages = current_workspace();
        let render = packages.iter_mut().find(|p| p.name == "metallic_render").unwrap();
        render.dependencies.push(Dependency {
            name: "metallic_project".to_owned(),
            kind: None,
        });

        let violations = check_packages(&packages);
        assert_eq!(
            violations,
            vec![Violation::ForbiddenWorkspaceDep {
                package: "metallic_render".to_owned(),
                dependency: "metallic_project".to_owned(),
                allowed: &["metallic_core"],
            }]
        );
        let line = violations[0].to_string();
        assert!(line.starts_with("[crate-deps]"), "{line}");
        assert!(line.contains("metallic_project"), "{line}");
    }

    #[test]
    fn build_dependencies_are_checked() {
        let packages = vec![
            pkg("metallic_core", &[("metallic_platform", Some(DepKind::Build))]),
            pkg("metallic_platform", &[]),
        ];
        assert_eq!(
            check_packages(&packages),
            vec![Violation::ForbiddenWorkspaceDep {
                package: "metallic_core".to_owned(),
                dependency: "metallic_platform".to_owned(),
                allowed: &[],
            }]
        );
    }

    #[test]
    fn dev_dependencies_are_exempt() {
        let packages = vec![
            pkg(
                "metallic_core",
                &[
                    ("metallic_engine", Some(DepKind::Dev)),
                    ("proptest", Some(DepKind::Dev)),
                ],
            ),
            pkg("metallic_engine", &[]),
        ];
        assert_eq!(check_packages(&packages), vec![]);
    }

    #[test]
    fn duplicate_declarations_are_reported_once() {
        let packages = vec![pkg("metallic_core", &[("wgpu", None), ("wgpu", Some(DepKind::Build))])];
        assert_eq!(
            check_packages(&packages),
            vec![Violation::ForbiddenCoreDep {
                dependency: "wgpu".to_owned()
            }]
        );
    }

    #[test]
    fn unknown_package_is_reported() {
        let packages = vec![
            pkg("metallic_audio", &[("metallic_core", None)]),
            pkg("metallic_core", &[]),
        ];
        let violations = check_packages(&packages);
        assert_eq!(
            violations,
            vec![Violation::UnknownPackage {
                package: "metallic_audio".to_owned()
            }]
        );
        assert!(violations[0].to_string().contains("add it to the table"));
    }

    #[test]
    fn core_external_allow_list() {
        let packages = vec![pkg(
            "metallic_core",
            &[("glam", None), ("serde", None), ("thiserror", None), ("winit", None)],
        )];
        let violations = check_packages(&packages);
        assert_eq!(
            violations,
            vec![Violation::ForbiddenCoreDep {
                dependency: "winit".to_owned()
            }]
        );
        assert!(violations[0].to_string().starts_with("[core-deps]"));
    }

    #[test]
    fn other_crates_may_use_external_crates() {
        let packages = vec![pkg("metallic_render", &[("wgpu", None)])];
        assert_eq!(check_packages(&packages), vec![]);
    }

    #[test]
    fn target_os_is_found_with_line_numbers() {
        let src = "fn a() {}\n#[cfg(target_os = \"linux\")]\nfn b() {}\n#[cfg(any(windows, target_os = \"macos\"))]\n\
                   fn c() { if cfg!(target_os = \"ios\") {} }\n";
        let lines: Vec<_> = scan_file("crates/render/src/lib.rs", src)
            .into_iter()
            .map(|v| match v {
                Violation::TargetOsOutsidePlatform { line, .. } => line,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(lines, vec![2, 4, 5]);
    }

    #[test]
    fn target_os_message_names_file_and_line() {
        let violations = scan_file("crates/core/src/lib.rs", "#[cfg(target_os = \"android\")]");
        assert_eq!(violations.len(), 1);
        let line = violations[0].to_string();
        assert!(line.starts_with("[target-os] crates/core/src/lib.rs:1:"), "{line}");
    }

    #[test]
    fn comments_and_lookalikes_are_ignored() {
        let src = "/// Use `#[cfg(target_os = ...)]` only in metallic_platform.\n\
                   // cfg(target_os\n\
                   let target_os_name = \"x\";\n\
                   let same = target_os == other;\n";
        assert_eq!(scan_file("crates/core/src/lib.rs", src), vec![]);
    }

    #[test]
    fn gpu_waits_on_the_frame_path_are_found() {
        let src = "fn frame(device: &wgpu::Device) {\n\
                   let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });\n\
                   // device.poll(wgpu::PollType::wait_indefinitely()) in a comment is fine\n\
                   let x = pollster::block_on(future);\n\
                   buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});\n\
                   }\n\
                   #[cfg(test)]\n\
                   mod tests { fn read() { device.poll(wgpu::PollType::wait_indefinitely()); } }\n";
        let found: Vec<_> = scan_gpu_waits("crates/render/src/frame.rs", src)
            .into_iter()
            .map(|v| match v {
                Violation::GpuWait { line, call, .. } => (line, call),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(found, vec![(2, "PollType::Wait"), (4, "block_on("), (5, ".map_async(")]);
        let message = scan_gpu_waits("crates/engine/src/x.rs", "pollster::block_on(f);")[0].to_string();
        assert!(message.starts_with("[gpu-wait] crates/engine/src/x.rs:1:"), "{message}");
    }

    #[test]
    fn allowed_gpu_waits_and_other_folders_pass() {
        // Setup's device request, and the asynchronous timestamp readback.
        assert_eq!(
            scan_gpu_waits(
                "crates/render/src/context.rs",
                "pollster::block_on(adapter.request_device(&d))"
            ),
            vec![]
        );
        assert_eq!(
            scan_gpu_waits(
                "crates/render/src/timing.rs",
                "slice.map_async(wgpu::MapMode::Read, f);"
            ),
            vec![]
        );
        // The allowance is per file and per call.
        assert_eq!(
            scan_gpu_waits("crates/render/src/timing.rs", "pollster::block_on(f);").len(),
            1
        );
        // Tests, benches, examples and other crates may wait.
        let wait = "device.poll(wgpu::PollType::wait_indefinitely());";
        assert_eq!(scan_gpu_waits("crates/render/tests/support/mod.rs", wait), vec![]);
        assert_eq!(scan_gpu_waits("crates/render/benches/gpu_scene.rs", wait), vec![]);
        assert_eq!(scan_gpu_waits("crates/render/src/scene/tests.rs", wait), vec![]);
        assert_eq!(scan_gpu_waits("crates/physics/src/lib.rs", wait), vec![]);
        for (file, _, why) in GPU_WAIT_ALLOWED {
            assert!(file.ends_with(".rs") && !why.is_empty());
        }
    }

    #[test]
    fn scanned_dirs() {
        assert!(is_scanned_dir("crates"));
        assert!(is_scanned_dir("crates/render"));
        assert!(is_scanned_dir("crates/platform_extras"));
        assert!(!is_scanned_dir("crates/platform"));
        assert!(!is_scanned_dir("apps"));
        assert!(!is_scanned_dir("xtask"));
        assert!(!is_scanned_dir("target"));
        assert!(!is_scanned_dir("crates/render/target"));
        assert!(!is_scanned_dir(".git"));
        assert!(!is_scanned_dir(".claude"));
    }

    #[test]
    fn table_has_no_unknown_or_self_edges() {
        for (name, allowed) in ALLOWED_WORKSPACE_DEPS {
            for dep in *allowed {
                assert_ne!(name, dep);
                assert!(allowed_workspace_deps(dep).is_some(), "`{name}` allows unknown `{dep}`");
            }
        }
    }
}
