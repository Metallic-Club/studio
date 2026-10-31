//! Breakpoints kept between Studio sessions, per place, in Studio's settings folder
//! (`breakpoints.toml` in [`crate::recent::config_dir`], `%APPDATA%\MetallicStudio` on Windows).
//!
//! A place is its scene file; a script is its full name in the tree
//! (`ServerScriptService.Checkpoints`), since instance ids change on every load. When two scripts
//! share a full name, the first in tree order gets the breakpoints.
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use metallic_core::{InstanceId, World};
use metallic_script::Breakpoint;
use serde::{Deserialize, Serialize};

/// Breakpoints by script and line.
pub type Breakpoints = BTreeMap<InstanceId, BTreeMap<u32, Breakpoint>>;

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    place: Vec<Place>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Place {
    scene: String,
    #[serde(default)]
    break_on_error: bool,
    #[serde(default)]
    script: Vec<ScriptLines>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ScriptLines {
    path: String,
    #[serde(default)]
    breakpoint: Vec<Saved>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    log: Option<String>,
}

/// One place's entry in the breakpoints file.
#[derive(Debug, Clone)]
pub struct BreakpointStore {
    file: PathBuf,
    scene: String,
}

impl BreakpointStore {
    /// The entry for the place `scene` in `file`.
    pub fn new(file: PathBuf, scene: &Path) -> BreakpointStore {
        BreakpointStore {
            file,
            scene: scene.to_string_lossy().replace('\\', "/"),
        }
    }

    fn read(&self) -> StoreFile {
        let Ok(text) = fs::read_to_string(&self.file) else {
            return StoreFile::default();
        };
        toml::from_str(&text).unwrap_or_else(|e| {
            log::warn!("ignoring unreadable breakpoints file: {}", e.message());
            StoreFile::default()
        })
    }

    /// The saved breakpoints of scripts in `world`, and break-on-error.
    pub fn load(&self, world: &World) -> (Breakpoints, bool) {
        let file = self.read();
        let Some(place) = file.place.into_iter().find(|p| p.scene == self.scene) else {
            return (Breakpoints::new(), false);
        };
        let scripts = scripts_by_path(world);
        let mut out = Breakpoints::new();
        for s in place.script {
            let Some(&id) = scripts.get(&s.path) else {
                continue;
            };
            let lines = out.entry(id).or_default();
            for b in s.breakpoint {
                let kind = match (b.condition, b.log) {
                    (_, Some(m)) => Breakpoint::Log(m),
                    (Some(c), None) => Breakpoint::Condition(c),
                    (None, None) => Breakpoint::Plain,
                };
                lines.insert(b.line.max(1), kind);
            }
        }
        out.retain(|_, l| !l.is_empty());
        (out, place.break_on_error)
    }

    /// Writes this place's breakpoints (scripts no longer in `world` are dropped). Other places
    /// in the file stay as they are.
    pub fn save(
        &self,
        world: &World,
        breakpoints: &Breakpoints,
        break_on_error: bool,
    ) -> std::io::Result<()> {
        let mut file = self.read();
        file.place.retain(|p| p.scene != self.scene);
        let script: Vec<ScriptLines> = breakpoints
            .iter()
            .filter(|(id, _)| world.parent(**id).is_some())
            .map(|(&id, lines)| ScriptLines {
                path: world.full_name(id),
                breakpoint: lines
                    .iter()
                    .map(|(&line, kind)| {
                        let (condition, log) = match kind {
                            Breakpoint::Plain => (None, None),
                            Breakpoint::Condition(c) => (Some(c.clone()), None),
                            Breakpoint::Log(m) => (None, Some(m.clone())),
                        };
                        Saved {
                            line,
                            condition,
                            log,
                        }
                    })
                    .collect(),
            })
            .collect();
        if !script.is_empty() || break_on_error {
            file.place.push(Place {
                scene: self.scene.clone(),
                break_on_error,
                script,
            });
        }
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(&file).map_err(std::io::Error::other)?;
        fs::write(&self.file, text)
    }
}

/// Every script (an instance with `Source`) by full name; the first in tree order wins.
fn scripts_by_path(world: &World) -> HashMap<String, InstanceId> {
    let mut out = HashMap::new();
    let mut stack: Vec<InstanceId> = world.roots().iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if world.get_ref(id, "Source").is_ok() {
            out.entry(world.full_name(id)).or_insert(id);
        }
        stack.extend(world.children(id).iter().rev());
    }
    out
}

/// Moves the breakpoints of a script whose text went from `old` to `new` with the lines they
/// were on: lines after the edit shift by the lines added or removed, lines before it stay, and
/// a breakpoint inside the changed lines stays there when the edit kept any lines, else goes.
pub fn shift_lines(
    lines: &BTreeMap<u32, Breakpoint>,
    old: &str,
    new: &str,
) -> BTreeMap<u32, Breakpoint> {
    let a: Vec<&str> = old.split('\n').collect();
    let b: Vec<&str> = new.split('\n').collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    // Changed lines (1-based, inclusive): old `prefix+1 ..= a.len()-suffix`, new
    // `prefix+1 ..= b.len()-suffix`.
    let old_end = (a.len() - suffix) as u32;
    let new_end = (b.len() - suffix) as u32;
    let first = prefix as u32 + 1;
    let mut out = BTreeMap::new();
    for (&line, kind) in lines {
        let moved = if line < first {
            Some(line)
        } else if line > old_end {
            Some((i64::from(line) + i64::from(new_end) - i64::from(old_end)) as u32)
        } else if new_end >= first {
            Some(line.min(new_end))
        } else {
            None
        };
        if let Some(l) = moved {
            out.entry(l).or_insert_with(|| kind.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::Value;

    fn plain(lines: &[u32]) -> BTreeMap<u32, Breakpoint> {
        lines.iter().map(|&l| (l, Breakpoint::Plain)).collect()
    }

    fn keys(m: &BTreeMap<u32, Breakpoint>) -> Vec<u32> {
        m.keys().copied().collect()
    }

    #[test]
    fn breakpoints_move_with_inserted_and_deleted_lines() {
        let old = "a\nb\nc\nd\n";
        // A line inserted above line 2 pushes 2..4 down, line 1 stays.
        let out = shift_lines(&plain(&[1, 2, 4]), old, "a\nnew\nb\nc\nd\n");
        assert_eq!(keys(&out), [1, 3, 5]);
        // Enter at the end of line 2 (a new line 3): only the lines below move.
        let out = shift_lines(&plain(&[2, 3]), old, "a\nb\n\nc\nd\n");
        assert_eq!(keys(&out), [2, 4]);
        // Deleting line 2 removes its breakpoint and pulls the rest up.
        let out = shift_lines(&plain(&[2, 3, 4]), old, "a\nc\nd\n");
        assert_eq!(keys(&out), [2, 3]);
        // Editing inside a line keeps its breakpoint there.
        let out = shift_lines(&plain(&[2]), old, "a\nbb\nc\nd\n");
        assert_eq!(keys(&out), [2]);
        // Joining lines 2 and 3 keeps one breakpoint on the joined line.
        let out = shift_lines(&plain(&[2, 3, 4]), old, "a\nbc\nd\n");
        assert_eq!(keys(&out), [2, 3]);
        // No change, no move.
        assert_eq!(keys(&shift_lines(&plain(&[1, 3]), old, old)), [1, 3]);
    }

    #[test]
    fn saved_breakpoints_come_back_by_script_path() {
        let dir = std::env::temp_dir().join(format!("metallic-bp-{}", std::process::id()));
        let file = dir.join("breakpoints.toml");
        let _ = fs::remove_file(&file);
        let mut world = World::new_with_services();
        let sss = world.root("ServerScriptService").unwrap();
        let script = world.create("Script", Some(sss)).unwrap();
        world
            .set(script, "Name", Value::String("Checkpoints".into()))
            .unwrap();
        let store = BreakpointStore::new(file.clone(), Path::new("game/main.mscene"));
        let mut bps = Breakpoints::new();
        bps.insert(
            script,
            BTreeMap::from([
                (2, Breakpoint::Plain),
                (5, Breakpoint::Condition("hp < 10".into())),
                (7, Breakpoint::Log("hp={hp}".into())),
            ]),
        );
        store.save(&world, &bps, true).unwrap();
        // Another place in the same file is kept apart.
        let other = BreakpointStore::new(file.clone(), Path::new("other/main.mscene"));
        other.save(&world, &Breakpoints::new(), true).unwrap();

        // A fresh load of the same place: new ids, same paths.
        let mut again = World::new_with_services();
        let sss = again.root("ServerScriptService").unwrap();
        let reloaded = again.create("Script", Some(sss)).unwrap();
        again
            .set(reloaded, "Name", Value::String("Checkpoints".into()))
            .unwrap();
        let (loaded, on) = store.load(&again);
        assert!(on);
        assert_eq!(loaded.get(&reloaded), bps.get(&script));
        let (none, on) = BreakpointStore::new(file.clone(), Path::new("x")).load(&again);
        assert!(none.is_empty() && !on);
        let _ = fs::remove_dir_all(&dir);
    }
}
