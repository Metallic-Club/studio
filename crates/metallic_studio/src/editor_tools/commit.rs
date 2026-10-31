//! Turning a tool's result into one undo step.
//!
//! The world is changed first (property writes, copies made with `World::clone_subtree`), then
//! the whole result is recorded as one `Batch`: `SetProperty` for each write, `Create` (with
//! its id filled) for a new Model or folder and `Duplicate` (with its copies filled) for the
//! copies. Undo parks the new instances and puts the old values back; redo unparks the very same
//! instances. If any write fails half way, everything done so far is taken back and nothing is
//! recorded.
use std::collections::HashMap;

use metallic_core::{CoreError, InstanceId, Value, Vec3, World};

use crate::commands::{Command, Placement, placement};
use crate::editor::EditorState;
use crate::tools::arrange::{CopiedPart, Plan};

/// What a commit made, for selecting the result.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Committed {
    /// Copies (roots of the copied subtrees), in plan order.
    pub copies: Vec<InstanceId>,
    /// The new Model holding the copies, when the plan groups them.
    pub group: Option<InstanceId>,
}

/// A new instance a commit made: class, id and the properties it was created with.
type Created = (String, InstanceId, Vec<(String, Value)>);

/// Collects what a commit did, so a failure can take it all back.
#[derive(Default)]
struct Undoable {
    sets: Vec<(InstanceId, &'static str, Value, Value)>,
    created: Vec<Created>,
    copies: Vec<(InstanceId, Placement)>,
}

impl Undoable {
    fn set(
        &mut self,
        world: &mut World,
        id: InstanceId,
        prop: &'static str,
        value: Value,
    ) -> Result<(), CoreError> {
        let old = world.get(id, prop)?;
        world.set(id, prop, value.clone())?;
        self.sets.push((id, prop, old, value));
        Ok(())
    }

    fn rollback(self, world: &mut World) {
        for (_, p) in self.copies.iter().rev() {
            world.destroy(p.id);
        }
        for (_, id, _) in self.created.iter().rev() {
            world.destroy(*id);
        }
        for (id, prop, old, _) in self.sets.into_iter().rev() {
            let _ = world.set(id, prop, old);
        }
    }

    fn into_commands(self, world: &World) -> Vec<Command> {
        let mut out: Vec<Command> = self
            .sets
            .into_iter()
            .map(|(id, prop, old, new)| Command::set(id, prop, old, new))
            .collect();
        for (class, id, props) in self.created {
            if let Some(parent) = world.parent(id) {
                out.push(Command::Create {
                    class,
                    parent,
                    props,
                    id: Some(id),
                });
            }
        }
        if !self.copies.is_empty() {
            let (sources, copies) = self.copies.into_iter().unzip();
            out.push(Command::Duplicate {
                sources,
                offset: Vec3::new(0.0, 0.0, 0.0),
                copies,
            });
        }
        out
    }
}

/// Pairs every instance of the subtree at `a` with the one at the same place under `b`.
fn pair_subtrees(
    world: &World,
    a: InstanceId,
    b: InstanceId,
    out: &mut HashMap<InstanceId, InstanceId>,
) {
    out.insert(a, b);
    let (ka, kb) = (world.children(a).to_vec(), world.children(b).to_vec());
    for (x, y) in ka.into_iter().zip(kb) {
        pair_subtrees(world, x, y, out);
    }
}

/// Copies `source` under `parent` and places the copy's parts as `parts` says.
pub fn clone_placed(
    world: &mut World,
    source: InstanceId,
    parent: InstanceId,
    parts: &[CopiedPart],
) -> Result<InstanceId, CoreError> {
    let copy = world.clone_subtree(source)?;
    let result = (|| {
        world.set_parent(copy, Some(parent))?;
        let mut pairs = HashMap::new();
        pair_subtrees(world, source, copy, &mut pairs);
        for p in parts {
            let Some(&c) = pairs.get(&p.source_part) else {
                continue;
            };
            world.set(c, "Size", Value::Vector3(p.size))?;
            world.set(c, "Orientation", Value::Vector3(p.orientation))?;
            world.set(c, "Position", Value::Vector3(p.position))?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(copy),
        Err(e) => {
            world.destroy(copy);
            Err(e)
        }
    }
}

/// Finds the child `name` of `parent`, or creates a Folder called `name` there (recorded).
fn folder(
    world: &mut World,
    done: &mut Undoable,
    parent: InstanceId,
    name: &str,
) -> Result<InstanceId, CoreError> {
    if let Some(f) = world.find_first_child(parent, name) {
        return Ok(f);
    }
    let f = world.create("Folder", Some(parent))?;
    let props = vec![("Name".to_string(), Value::String(name.to_string()))];
    if let Err(e) = world.set(f, "Name", props[0].1.clone()) {
        world.destroy(f);
        return Err(e);
    }
    done.created.push(("Folder".into(), f, props));
    Ok(f)
}

/// Resolves a dotted path such as `Workspace.Brush`, creating missing Folders (recorded).
fn folder_path(
    world: &mut World,
    done: &mut Undoable,
    path: &str,
) -> Result<InstanceId, CoreError> {
    let mut names = path.split('.').map(str::trim).filter(|n| !n.is_empty());
    let first = names.next().unwrap_or("Workspace");
    let mut cur = world
        .root(first)
        .or_else(|| world.root("Workspace"))
        .ok_or(CoreError::InvalidInstance)?;
    for name in names {
        cur = folder(world, done, cur, name)?;
    }
    Ok(cur)
}

fn finish(state: &mut EditorState, done: Undoable, name: String) {
    let commands = done.into_commands(&state.world);
    if !commands.is_empty() {
        state.record(Command::Batch { name, commands });
    }
}

/// Applies an arrange [`Plan`] and records it as one undo step named by the plan.
pub fn commit_plan(state: &mut EditorState, plan: &Plan) -> Result<Committed, String> {
    if state.play.is_playing() {
        return Err("Stop the game to edit".into());
    }
    if plan.is_empty() {
        return Err("Nothing to change".into());
    }
    let mut done = Undoable::default();
    let mut made = Committed::default();
    let world = &mut state.world;
    let result = (|| -> Result<(), CoreError> {
        for c in &plan.changes {
            done.set(world, c.id, c.property, c.after.clone())?;
        }
        if let Some(g) = &plan.group {
            let parent = g
                .parent
                .or_else(|| world.root("Workspace"))
                .ok_or(CoreError::InvalidInstance)?;
            let m = world.create("Model", Some(parent))?;
            let props = vec![("Name".to_string(), Value::String(g.name.clone()))];
            world.set(m, "Name", props[0].1.clone())?;
            done.created.push(("Model".into(), m, props));
            made.group = Some(m);
        }
        for copy in &plan.copies {
            let parent = made
                .group
                .or(copy.parent)
                .or_else(|| world.root("Workspace"))
                .ok_or(CoreError::InvalidInstance)?;
            let id = clone_placed(world, copy.source, parent, &copy.parts)?;
            let p = placement(world, id).ok_or(CoreError::InvalidInstance)?;
            done.copies.push((copy.source, p));
            made.copies.push(id);
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            finish(state, done, plan.label.clone());
            Ok(made)
        }
        Err(e) => {
            done.rollback(&mut state.world);
            Err(format!("{}: {e}", plan.label))
        }
    }
}

/// One brush placement, ready to apply: the brush item's instance and its parts' new places.
pub struct Placed {
    /// The instance the brush copies.
    pub source: InstanceId,
    /// Where each of its parts goes.
    pub parts: Vec<CopiedPart>,
}

/// Places brush copies under `folder_path`/`set_name` and records them as one undo step.
pub fn commit_placements(
    state: &mut EditorState,
    folder_path_text: &str,
    set_name: &str,
    placed: &[Placed],
    name: String,
) -> Result<Vec<InstanceId>, String> {
    if state.play.is_playing() {
        return Err("Stop the game to edit".into());
    }
    let mut done = Undoable::default();
    let mut made = Vec::new();
    let world = &mut state.world;
    let result = (|| -> Result<(), CoreError> {
        let base = folder_path(world, &mut done, folder_path_text)?;
        let set_folder = folder(world, &mut done, base, set_name)?;
        for p in placed {
            let id = clone_placed(world, p.source, set_folder, &p.parts)?;
            let pl = placement(world, id).ok_or(CoreError::InvalidInstance)?;
            done.copies.push((p.source, pl));
            made.push(id);
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            finish(state, done, name);
            Ok(made)
        }
        Err(e) => {
            done.rollback(&mut state.world);
            Err(format!("{name}: {e}"))
        }
    }
}

/// The folder a brush set paints into (`Workspace.Brush` then the set's name), if it exists.
pub fn find_set_folder(
    world: &World,
    folder_path_text: &str,
    set_name: &str,
) -> Option<InstanceId> {
    let mut names = folder_path_text
        .split('.')
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let mut cur = world.root(names.next().unwrap_or("Workspace"))?;
    for name in names {
        cur = world.find_first_child(cur, name)?;
    }
    world.find_first_child(cur, set_name)
}
