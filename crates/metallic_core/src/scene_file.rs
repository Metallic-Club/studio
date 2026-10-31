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
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

const VERSION: u32 = 1;

/// Property value as stored in a file: instance references become tree-order indices.
#[derive(Serialize, Deserialize)]
enum SceneValue {
    Nil,
    Bool(bool),
    Number(f64),
    String(String),
    Vector3(Vec3),
    Color3(Color3),
    UDim2(UDim2),
    Ref(Option<usize>),
    CFrame(CFrame),
}

#[derive(Deserialize)]
struct SceneFile {
    version: u32,
    roots: Vec<SceneNode>,
}

#[derive(Deserialize)]
struct SceneNode {
    class: String,
    #[serde(default)]
    builtin: bool,
    #[serde(default)]
    props: Vec<(String, SceneValue)>,
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

fn preorder(world: &World, ids: &[InstanceId], out: &mut Vec<InstanceId>) {
    for &id in ids {
        out.push(id);
        preorder(world, world.children(id), out);
    }
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
/// The write is atomic: the text goes to a temporary file in the same folder, which is flushed and
/// synced to disk and then renamed over `path`. If anything fails, `path` keeps its old contents
/// and the temporary file is removed.
///
/// Errors: [`CoreError::Io`] if the file cannot be written, [`CoreError::Scene`] if a value cannot be encoded.
pub fn save_scene(world: &World, path: &Path) -> Result<(), CoreError> {
    let mut order = Vec::new();
    preorder(world, world.roots(), &mut order);
    let index_of = |id: InstanceId| order.iter().position(|&o| o == id);
    let mut out = String::new();
    let _ = writeln!(out, "(\n    version: {VERSION},\n    roots: [");
    for &r in world.roots() {
        write_node(world, r, 2, &index_of, &mut out)?;
    }
    out.push_str("    ],\n)\n");
    write_atomic(path, |f| f.write_all(out.as_bytes()))
        .map_err(|e| CoreError::Io(format!("{}: {e}", path.display())))
}

/// Writes `path` through a temporary file in the same folder: `fill` writes the contents, then
/// the file is flushed, synced and renamed over `path`. On any error the temporary file is
/// removed and `path` is left as it was.
fn write_atomic(
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        fill(&mut file)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        // Make the rename itself durable where the platform allows syncing a folder.
        #[cfg(unix)]
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
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
    fn add(&mut self, node: SceneNode, parent: Option<InstanceId>) -> Result<(), CoreError> {
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
            None => self.world.create(&node.class, parent)?,
        };
        self.matched.insert(id);
        self.created.push(id);
        for (name, v) in node.props {
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
        for c in node.children {
            self.add(c, Some(id))?;
        }
        Ok(())
    }
}

/// Reads a scene file into a new world (built on [`World::new_with_services`]).
///
/// Errors: [`CoreError::Io`] if unreadable, [`CoreError::Scene`] if it does not parse, has an
/// unsupported version or a dangling reference; class/property errors from [`World::create`] and [`World::set`].
/// Every property goes through [`World::set`], so a CFrame with a scaled or skewed rotation is
/// re-orthonormalised (keeping its look vector) and a degenerate one fails the load.
pub fn load_scene(path: &Path) -> Result<World, CoreError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| CoreError::Io(format!("{}: {e}", path.display())))?;
    let file: SceneFile = ron::from_str(&text).map_err(|e| CoreError::Scene(e.to_string()))?;
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
        loader.add(r, None)?;
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
}
