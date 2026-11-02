//! Text scene files (RON).
//!
//! Layout: one block per instance, one line per non-default property, children nested:
//!
//! ```text
//! (
//!     version: 1,
//!     roots: [
//!         (
//!             class: "Part",
//!             props: [
//!                 ("Position", Vector3((x: 1.0, y: 2.0, z: 3.0))),
//!             ],
//!         ),
//!     ],
//! )
//! ```
//! Instances are written in tree order. Built-in service instances are marked `builtin: true`
//! and are matched with the ones `World::new_with_services` already made when loading.
use crate::cframe::CFrame;
use crate::classes::{all_properties, default_value};
use crate::value::{Color3, UDim2, Value, Vec3};
use crate::{CoreError, InstanceId, World};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const VERSION: u32 = 1;

/// Maximum instance nesting in a scene file, counting each root as depth one.
///
/// Saving and loading enforce the same limit so a successfully saved tree can be read back
/// without exceeding the RON parser's recursion budget.
pub const MAX_SCENE_DEPTH: usize = 32;

/// Property value as stored in a file: instance references become tree-order indices.
#[derive(Serialize, Deserialize)]
enum SceneValue {
    Nil,
    Bool(bool),
    Number(f64),
    String(String),
    Vector3(#[serde(with = "SceneVec3")] Vec3),
    Color3(#[serde(with = "SceneColor3")] Color3),
    UDim2(#[serde(with = "SceneUDim2")] UDim2),
    Ref(Option<usize>),
    CFrame(CFrame),
}

// Scene-specific strict representations keep the public value types' Serde
// behavior unchanged for callers using formats other than scene files.
#[derive(Serialize, Deserialize)]
#[serde(remote = "Vec3", deny_unknown_fields)]
struct SceneVec3 {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Color3", deny_unknown_fields)]
struct SceneColor3 {
    r: f64,
    g: f64,
    b: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "UDim2", deny_unknown_fields)]
struct SceneUDim2 {
    x_scale: f64,
    x_offset: f64,
    y_scale: f64,
    y_offset: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneFile {
    version: u32,
    roots: Vec<SceneNode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneNode {
    class: String,
    #[serde(default)]
    builtin: bool,
    #[serde(default)]
    props: Vec<(String, SceneValue)>,
    /// Attributes, sorted by name. Absent in files written before attributes existed.
    #[serde(default)]
    attributes: Vec<(String, SceneValue)>,
    /// Tags, sorted. Absent in files written before tags existed.
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    children: Vec<SceneNode>,
}

fn to_scene(v: Value, index_of: &dyn Fn(InstanceId) -> Option<usize>) -> SceneValue {
    match v {
        Value::Nil => SceneValue::Nil,
        Value::Bool(b) => SceneValue::Bool(b),
        Value::Number(n) => SceneValue::Number(n),
        Value::String(s) => SceneValue::String(s),
        Value::Vector3(x) => SceneValue::Vector3(x),
        Value::Color3(x) => SceneValue::Color3(x),
        Value::UDim2(x) => SceneValue::UDim2(x),
        Value::Ref(r) => SceneValue::Ref(r.and_then(index_of)),
        Value::CFrame(cf) => SceneValue::CFrame(*cf),
    }
}

fn preorder(world: &World, out: &mut Vec<InstanceId>) -> Result<(), CoreError> {
    let standard = World::new_with_services();
    let mut builtin_parents = HashMap::new();
    let mut builtin_classes = Vec::new();
    let mut builtins: Vec<_> = standard.roots().iter().rev().copied().collect();
    while let Some(id) = builtins.pop() {
        builtin_classes.push(standard.class_name(id));
        builtin_parents.insert(
            standard.class_name(id),
            standard
                .parent(id)
                .and_then(|parent| standard.class_name(parent)),
        );
        builtins.extend(standard.children(id).iter().rev().copied());
    }
    let mut seen_builtins = HashSet::new();
    let mut pending: Vec<_> = world.roots().iter().rev().map(|&id| (id, 1)).collect();
    while let Some((id, depth)) = pending.pop() {
        if depth > MAX_SCENE_DEPTH {
            return Err(CoreError::Scene(format!(
                "scene depth exceeds {MAX_SCENE_DEPTH} instances"
            )));
        }
        if world.is_builtin(id) {
            if !seen_builtins.insert(world.class_name(id)) {
                return Err(CoreError::Scene(format!(
                    "duplicate builtin `{}`",
                    world.class_name(id).unwrap_or("unknown")
                )));
            }
            let parent = world.parent(id);
            let parent_class = parent.and_then(|parent| world.class_name(parent));
            if parent.is_some_and(|parent| !world.is_builtin(parent))
                || builtin_parents.get(&world.class_name(id)) != Some(&parent_class)
            {
                return Err(CoreError::Scene(format!(
                    "builtin `{}` has moved from its standard location",
                    world.class_name(id).unwrap_or("unknown")
                )));
            }
        }
        out.push(id);
        pending.extend(world.children(id).iter().rev().map(|&id| (id, depth + 1)));
    }
    for class in builtin_classes {
        if !seen_builtins.contains(&class) {
            return Err(CoreError::Scene(format!(
                "missing builtin `{}` required by scene version {VERSION}",
                class.unwrap_or("unknown")
            )));
        }
    }
    Ok(())
}

fn write_node(
    world: &World,
    id: InstanceId,
    depth: usize,
    index_of: &dyn Fn(InstanceId) -> Option<usize>,
    out: &mut String,
) -> Result<(), CoreError> {
    let pad = "    ".repeat(depth);
    let class = world.class_name(id).ok_or(CoreError::InvalidInstance)?;
    let err = |e: ron::Error| CoreError::Scene(e.to_string());
    let _ = writeln!(out, "{pad}(");
    let _ = writeln!(
        out,
        "{pad}    class: {},",
        ron::to_string(class).map_err(err)?
    );
    if world.is_builtin(id) {
        let _ = writeln!(out, "{pad}    builtin: true,");
    }
    let mut lines = Vec::new();
    for def in all_properties(class) {
        let v = world.get(id, def.name)?;
        if default_value(class, def.name).as_ref() == Some(&v) {
            continue;
        }
        let name = ron::to_string(def.name).map_err(err)?;
        let val = ron::to_string(&to_scene(v, index_of)).map_err(err)?;
        lines.push(format!("{pad}        ({name}, {val}),"));
    }
    if !lines.is_empty() {
        let _ = writeln!(out, "{pad}    props: [");
        for l in lines {
            let _ = writeln!(out, "{l}");
        }
        let _ = writeln!(out, "{pad}    ],");
    }
    let tags: Vec<String> = world.tags(id).map(str::to_string).collect();
    if !tags.is_empty() {
        let list: Vec<String> = tags
            .iter()
            .map(|t| ron::to_string(t).map_err(err))
            .collect::<Result<_, _>>()?;
        let _ = writeln!(out, "{pad}    tags: [{}],", list.join(", "));
    }
    let attributes: Vec<String> = world
        .attributes(id)
        .map(|(name, v)| {
            let name = ron::to_string(name).map_err(err)?;
            let val = ron::to_string(&to_scene(v.clone(), index_of)).map_err(err)?;
            Ok(format!("{pad}        ({name}, {val}),"))
        })
        .collect::<Result<_, CoreError>>()?;
    if !attributes.is_empty() {
        let _ = writeln!(out, "{pad}    attributes: [");
        for l in attributes {
            let _ = writeln!(out, "{l}");
        }
        let _ = writeln!(out, "{pad}    ],");
    }
    let kids = world.children(id);
    if !kids.is_empty() {
        let _ = writeln!(out, "{pad}    children: [");
        for &k in kids {
            write_node(world, k, depth + 2, index_of, out)?;
        }
        let _ = writeln!(out, "{pad}    ],");
    }
    let _ = writeln!(out, "{pad}),");
    Ok(())
}

/// Writes the world to `path` in tree order, non-default properties only.
///
/// The complete scene is encoded before a sibling temporary file is written and synced, then
/// renamed over the destination. Failure before replacement leaves an existing scene untouched
/// and the temporary file is removed.
/// The tree must be no deeper than [`MAX_SCENE_DEPTH`] and must contain every canonical built-in
/// exactly once at its standard location, including `StarterPlayerScripts`. A builtinless
/// [`World::new_empty`] is an in-memory utility world and cannot be saved as a v1 game scene:
/// the v1 loader always creates standard services, so it could not preserve that world.
///
/// Replacement creates a new filesystem entry at `path`: a destination symlink is replaced,
/// other hard links still refer to the old file, and platform-specific metadata (including
/// Windows ACLs and alternate streams) is not preserved. Existing Unix permission bits are
/// copied when destination metadata is available. The temporary file is synced, but the parent
/// directory is not; this is not a power-loss durability guarantee. A process crash may leave
/// a temporary sibling. On Windows, sharing/access-denied rename failures receive three short
/// retries (10, 20, 40 ms); a persistent lock still fails without removing the old destination.
///
/// Errors: [`CoreError::Io`] if temporary creation, writing, syncing, permission copying or
/// replacement fails; [`CoreError::Scene`] for excessive depth, missing/duplicate/moved built-ins
/// or an encoding failure. Preflight/encoding errors occur before any destination write.
pub fn save_scene(world: &World, path: &Path) -> Result<(), CoreError> {
    let mut order = Vec::new();
    preorder(world, &mut order)?;
    let indices: HashMap<_, _> = order
        .into_iter()
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect();
    let index_of = |id: InstanceId| indices.get(&id).copied();
    let mut out = String::new();
    let _ = writeln!(out, "(\n    version: {VERSION},\n    roots: [");
    for &r in world.roots() {
        write_node(world, r, 2, &index_of, &mut out)?;
    }
    out.push_str("    ],\n)\n");
    write_atomic(path, |file| file.write_all(out.as_bytes()))
        .map_err(|e| CoreError::Io(format!("{}: {e}", path.display())))
}

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

struct PendingScene(PathBuf);

impl Drop for PendingScene {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes `path` through a temporary file in the same folder: `write` writes the contents, then
/// the file is flushed, synced and renamed over `path`. On any error the temporary file is
/// removed and `path` is left as it was.
fn write_atomic(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let filename = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "scene destination has no filename",
        )
    })?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (pending, mut file) = {
        let mut opened = None;
        for _ in 0..100 {
            let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
            let temporary_name = format!(".metallic-scene-{}-{sequence}.tmp", std::process::id());
            if filename
                .to_string_lossy()
                .eq_ignore_ascii_case(&temporary_name)
            {
                continue;
            }
            let temporary = parent.join(temporary_name);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => {
                    opened = Some((PendingScene(temporary), file));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        opened.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "cannot create scene temporary file",
            )
        })?
    };
    let written = (|| {
        write(&mut file)?;
        file.flush()?;
        #[cfg(unix)]
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()
    })();
    drop(file);
    written?;
    replace_scene(&pending.0, path)?;
    // Make the rename itself durable where the platform allows syncing a folder.
    #[cfg(unix)]
    if let Ok(d) = File::open(parent) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn replace_scene(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        retry_windows_rename(|| std::fs::rename(from, to))
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

#[cfg(windows)]
fn retry_windows_rename(mut rename: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut result = rename();
    for delay_ms in [10, 20, 40] {
        // ERROR_ACCESS_DENIED / ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION.
        // Permanent ACL/read-only failures have the same access code, so retries
        // are bounded and the final OS error is returned without deleting either file.
        if !matches!(
            result.as_ref().err().and_then(io::Error::raw_os_error),
            Some(5 | 32 | 33)
        ) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        result = rename();
    }
    result
}

struct Loader {
    world: World,
    created: Vec<InstanceId>,
    matched: HashSet<InstanceId>,
    pending: Vec<(InstanceId, String, Option<usize>)>,
}

impl Loader {
    fn add(
        &mut self,
        node: SceneNode,
        parent: Option<InstanceId>,
        depth: usize,
    ) -> Result<(), CoreError> {
        if depth > MAX_SCENE_DEPTH {
            return Err(CoreError::Scene(format!(
                "scene depth exceeds {MAX_SCENE_DEPTH} instances"
            )));
        }
        let existing = if node.builtin {
            let candidates: Vec<InstanceId> = match parent {
                Some(p) => self.world.children(p).to_vec(),
                None => self.world.roots().to_vec(),
            };
            candidates.into_iter().find(|&c| {
                !self.matched.contains(&c)
                    && self.world.is_builtin(c)
                    && self.world.class_name(c) == Some(node.class.as_str())
            })
        } else {
            None
        };
        let id = match existing {
            Some(id) => id,
            None if node.builtin => {
                return Err(CoreError::Scene(format!(
                    "builtin `{}` does not match an unmatched built-in instance at this location",
                    node.class
                )));
            }
            None => self.world.create(&node.class, parent)?,
        };
        self.matched.insert(id);
        self.created.push(id);
        let mut properties = HashSet::new();
        for (name, v) in node.props {
            if !properties.insert(name.clone()) {
                return Err(CoreError::Scene(format!(
                    "duplicate property `{name}` on `{}`",
                    node.class
                )));
            }
            let value = match v {
                SceneValue::Nil => Value::Nil,
                SceneValue::Bool(b) => Value::Bool(b),
                SceneValue::Number(n) => Value::Number(n),
                SceneValue::String(s) => Value::String(s),
                SceneValue::Vector3(x) => Value::Vector3(x),
                SceneValue::Color3(x) => Value::Color3(x),
                SceneValue::UDim2(x) => Value::UDim2(x),
                SceneValue::CFrame(x) => Value::CFrame(Box::new(x)),
                SceneValue::Ref(r) => {
                    self.pending.push((id, name, r));
                    continue;
                }
            };
            self.world.set(id, &name, value)?;
        }
        for (name, v) in node.attributes {
            let value = match v {
                SceneValue::Nil => Value::Nil,
                SceneValue::Bool(b) => Value::Bool(b),
                SceneValue::Number(n) => Value::Number(n),
                SceneValue::String(s) => Value::String(s),
                SceneValue::Vector3(x) => Value::Vector3(x),
                SceneValue::Color3(x) => Value::Color3(x),
                SceneValue::UDim2(x) => Value::UDim2(x),
                SceneValue::CFrame(x) => Value::CFrame(Box::new(x)),
                SceneValue::Ref(_) => {
                    return Err(CoreError::Scene(format!(
                        "attribute `{name}` cannot hold an instance reference"
                    )));
                }
            };
            self.world.set_attribute(id, &name, value)?;
        }
        for tag in node.tags {
            self.world.add_tag(id, &tag)?;
        }
        for c in node.children {
            self.add(c, Some(id), depth + 1)?;
        }
        Ok(())
    }
}

/// Reads a scene file into a new world (built on [`World::new_with_services`]).
///
/// Errors: [`CoreError::Io`] if unreadable, [`CoreError::Scene`] if it does not parse, has an
/// unsupported version, unknown field (including typed value members), duplicate property, unmatched built-in marker, excessive
/// depth or a dangling reference; class/property errors from [`World::create`] and [`World::set`].
/// Every property goes through [`World::set`], so a CFrame with a scaled or skewed rotation is
/// re-orthonormalised (keeping its right axis) and a degenerate one fails the load.
/// Legacy v1 input may omit standard services; they are supplied by the fresh world. The saver
/// requires the complete canonical service set so it cannot silently save a world that changes on load.
pub fn load_scene(path: &Path) -> Result<World, CoreError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| CoreError::Io(format!("{}: {e}", path.display())))?;
    // A node consumes several parser frames (struct, children sequence and sequence entries).
    // Keep RON's own bound enabled while allowing the full documented instance depth.
    let file: SceneFile = ron::Options::default()
        .with_recursion_limit(MAX_SCENE_DEPTH * 4 + 32)
        .from_str(&text)
        .map_err(|e| CoreError::Scene(e.to_string()))?;
    if file.version != VERSION {
        return Err(CoreError::Scene(format!(
            "unsupported scene version {}",
            file.version
        )));
    }
    let mut loader = Loader {
        world: World::new_with_services(),
        created: Vec::new(),
        matched: HashSet::new(),
        pending: Vec::new(),
    };
    for r in file.roots {
        loader.add(r, None, 1)?;
    }
    for (id, prop, index) in std::mem::take(&mut loader.pending) {
        let target =
            match index {
                None => None,
                Some(i) => Some(*loader.created.get(i).ok_or_else(|| {
                    CoreError::Scene(format!("reference to missing instance #{i}"))
                })?),
            };
        loader.world.set(id, &prop, Value::Ref(target))?;
    }
    Ok(loader.world)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory() -> PathBuf {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "metallic_atomic_scene_{}_{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir()
            .join("metallic_core_atomic")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn failed_write_keeps_the_original_file() {
        let d = dir("fail");
        let path = d.join("main.mscene");
        std::fs::write(&path, "original").unwrap();
        let err = write_atomic(&path, |f| {
            f.write_all(b"half a sce")?;
            Err(std::io::Error::other("disk full"))
        });
        assert!(err.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        // No temporary file is left behind.
        let left: Vec<_> = std::fs::read_dir(&d).unwrap().collect();
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn save_replaces_the_file_and_leaves_no_temp() {
        let d = dir("ok");
        let path = d.join("main.mscene");
        std::fs::write(&path, "old").unwrap();
        save_scene(&World::new_with_services(), &path).unwrap();
        assert!(load_scene(&path).is_ok());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    }

    #[test]
    fn save_into_a_missing_folder_fails_cleanly() {
        let d = dir("missing");
        let path = d.join("nope").join("main.mscene");
        assert!(matches!(
            save_scene(&World::new_with_services(), &path),
            Err(CoreError::Io(_))
        ));
    }

    #[test]
    fn partial_write_failure_preserves_destination_and_removes_temporary_file() {
        let directory = test_directory();
        let destination = directory.join("scene.mscene");
        std::fs::write(&destination, "previous complete scene").unwrap();
        let result = write_atomic(&destination, |file| {
            file.write_all(b"partial replacement")?;
            Err(io::Error::other("simulated disk failure"))
        });
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "previous complete scene"
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
    }

    #[test]
    fn rename_failure_preserves_destination_and_removes_temporary_file() {
        let directory = test_directory();
        let destination = directory.join("existing_directory");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("keep.txt"), "keep").unwrap();
        let result = write_atomic(&destination, |file| file.write_all(b"replacement"));
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(destination.join("keep.txt")).unwrap(),
            "keep"
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
    }

    #[test]
    fn successful_save_replaces_destination_and_removes_temporary_file() {
        let directory = test_directory();
        let destination = directory.join("scene.mscene");
        std::fs::write(&destination, "previous").unwrap();
        write_atomic(&destination, |file| file.write_all(b"complete replacement")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "complete replacement"
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
    }

    #[test]
    fn public_save_failure_preserves_destination_and_cleans_temporary_file() {
        let directory = test_directory();
        let destination = directory.join("existing_directory");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("keep.txt"), "keep").unwrap();
        assert!(matches!(
            save_scene(&World::new_with_services(), &destination),
            Err(CoreError::Io(_))
        ));
        assert_eq!(
            std::fs::read_to_string(destination.join("keep.txt")).unwrap(),
            "keep"
        );
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 1);
    }

    #[test]
    fn loader_itself_rejects_depth_33() {
        let mut node = SceneNode {
            class: "Folder".into(),
            builtin: false,
            props: vec![],
            attributes: Default::default(),
            tags: Default::default(),
            children: vec![],
        };
        for _ in 1..=MAX_SCENE_DEPTH {
            node = SceneNode {
                class: "Folder".into(),
                builtin: false,
                props: vec![],
                attributes: Default::default(),
                tags: Default::default(),
                children: vec![node],
            };
        }
        let mut loader = Loader {
            world: World::new_with_services(),
            created: vec![],
            matched: HashSet::new(),
            pending: vec![],
        };
        assert!(
            matches!(loader.add(node, None, 1), Err(CoreError::Scene(ref message)) if message == "scene depth exceeds 32 instances")
        );
    }

    #[cfg(windows)]
    #[test]
    fn transient_windows_share_lock_is_retried_without_deleting_the_destination() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = test_directory();
        let source = directory.join("replacement");
        let destination = directory.join("scene.mscene");
        std::fs::write(&source, "replacement").unwrap();
        std::fs::write(&destination, "previous complete scene").unwrap();
        let mut blocker = Some(
            OpenOptions::new()
                .read(true)
                .share_mode(3)
                .open(&destination)
                .unwrap(),
        );
        let mut calls = 0;
        retry_windows_rename(|| {
            calls += 1;
            let result = std::fs::rename(&source, &destination);
            if calls == 1 {
                assert!(matches!(
                    result.as_ref().err().and_then(io::Error::raw_os_error),
                    Some(5 | 32 | 33)
                ));
                assert_eq!(
                    std::fs::read_to_string(&destination).unwrap(),
                    "previous complete scene"
                );
                drop(blocker.take());
            }
            result
        })
        .unwrap();
        assert!(calls >= 2);
        assert_eq!(std::fs::read_to_string(destination).unwrap(), "replacement");
        assert!(!source.exists());
    }

    #[cfg(windows)]
    #[test]
    fn persistent_windows_share_lock_fails_safely_through_public_save() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = test_directory();
        let destination = directory.join("scene.mscene");
        std::fs::write(&destination, "previous complete scene").unwrap();
        let _blocker = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&destination)
            .unwrap();
        assert!(matches!(
            save_scene(&World::new_with_services(), &destination),
            Err(CoreError::Io(_))
        ));
        assert_eq!(
            std::fs::read_to_string(destination).unwrap(),
            "previous complete scene"
        );
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 1);
    }
}
