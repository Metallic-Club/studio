//! Reversible modelling commands and a simple undo history.
//!
//! [`MeshCommand::run`] performs an [`Op`] once on an [`EditMesh`] while the mesh journals every element it touches;
//! the command keeps that [`ChangeSet`] plus the selection before and after. Undo writes the "before" states back,
//! redo the "after" states, so neither re-runs the operation and both restore the exact same element ids.
//! Studio's `History` (`metallic_studio::commands`) can hold `MeshCommand`s directly next to its world commands;
//! [`History`] here is the stand-alone version used by the demo and tests.

use glam::Vec3;

use crate::error::{ModelError, ModelResult};
use crate::mesh::{ChangeSet, Mesh};
use crate::ops;
use crate::ops::delete::DeleteKind;
use crate::ops::transform::{Axis, Transform};
use crate::primitives::Primitive;
use crate::selection::Selection;
use crate::uv::UnwrapOptions;

/// A mesh being edited together with its selection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditMesh {
    /// The geometry.
    pub mesh: Mesh,
    /// What is selected.
    pub selection: Selection,
}

impl EditMesh {
    /// Starts editing `mesh` with nothing selected (face mode).
    #[must_use]
    pub fn new(mesh: Mesh) -> Self {
        Self { mesh, selection: Selection::default() }
    }
}

/// Every modelling operation, as plain data (so it can be logged, replayed, or sent over Team Create later).
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Replace the selection (selection changes are undoable, as in Blender).
    Select(Selection),
    /// Move, rotate or scale the selected vertices.
    Transform(Transform),
    /// Extrude selected faces and move them along the normal.
    ExtrudeFaces {
        /// Distance along the normal.
        distance: f32,
        /// Each face separately instead of as one region.
        individual: bool,
    },
    /// Extrude selected edges into quads.
    ExtrudeEdges {
        /// Movement of the new edges.
        offset: Vec3,
    },
    /// Extrude selected vertices into wire edges.
    ExtrudeVerts {
        /// Movement of the new vertices.
        offset: Vec3,
    },
    /// Inset selected faces.
    Inset {
        /// Ring width.
        thickness: f32,
        /// Push of the inner part along the normal.
        depth: f32,
        /// Each face separately instead of as one region.
        individual: bool,
    },
    /// Bevel selected edges.
    Bevel {
        /// Distance from the edge to each rail.
        width: f32,
        /// Profile segments (1 = chamfer).
        segments: u32,
    },
    /// Cut loops across the ring through an edge.
    LoopCut {
        /// An edge of the ring to cut across.
        edge: crate::ids::EdgeId,
        /// Number of loops.
        cuts: u32,
    },
    /// Connect two selected edge loops with quads.
    BridgeEdgeLoops,
    /// Merge selected vertices at their centre.
    MergeAtCenter,
    /// Merge selected vertices closer than a distance.
    MergeByDistance {
        /// Weld distance.
        distance: f32,
    },
    /// Dissolve selected vertices.
    DissolveVerts,
    /// Dissolve selected edges.
    DissolveEdges,
    /// Dissolve selected faces into one per connected group.
    DissolveFaces,
    /// Delete selected elements.
    Delete(DeleteKind),
    /// Make a face from the selected loop or vertices.
    Fill,
    /// Fill the selected loop with a quad grid.
    GridFill {
        /// Quads along the first side (default: a quarter of the loop).
        span: Option<u32>,
        /// Rotates where the first side starts.
        offset: u32,
    },
    /// Mirror selected geometry across an axis plane through the origin.
    Mirror {
        /// Mirror axis.
        axis: Axis,
        /// Snap-and-weld distance at the plane.
        merge_distance: f32,
    },
    /// Split selected faces into quads (no smoothing).
    Subdivide {
        /// Repeat count.
        levels: u32,
    },
    /// Catmull-Clark smooth the whole mesh.
    CatmullClark {
        /// Repeat count.
        levels: u32,
    },
    /// Reverse the winding of selected faces.
    FlipNormals,
    /// Make selected faces consistently face outwards.
    RecalculateOutside,
    /// Mark or clear selected edges as sharp.
    MarkSharp(bool),
    /// Mark or clear selected edges as UV seams.
    MarkSeam(bool),
    /// Box-project UVs of selected faces.
    UvBox {
        /// UV units per stud.
        scale: f32,
    },
    /// Planar-project UVs of selected faces.
    UvPlanar {
        /// Projection direction.
        normal: Vec3,
        /// UV units per stud.
        scale: f32,
    },
    /// Unwrap selected faces along seams (LSCM) and pack them.
    UvUnwrap(UnwrapOptions),
    /// Add a starter shape and select it.
    AddPrimitive {
        /// The shape.
        primitive: Primitive,
        /// Where its centre goes.
        center: Vec3,
    },
}

impl Op {
    /// A short, beginner-readable name for menus and the undo list.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Op::Select(_) => "Select",
            Op::Transform(_) => "Transform",
            Op::ExtrudeFaces { individual: true, .. } => "Extrude faces (individual)",
            Op::ExtrudeFaces { .. } => "Extrude",
            Op::ExtrudeEdges { .. } => "Extrude edges",
            Op::ExtrudeVerts { .. } => "Extrude vertices",
            Op::Inset { .. } => "Inset",
            Op::Bevel { .. } => "Bevel",
            Op::LoopCut { .. } => "Loop cut",
            Op::BridgeEdgeLoops => "Bridge edge loops",
            Op::MergeAtCenter => "Merge at centre",
            Op::MergeByDistance { .. } => "Merge by distance",
            Op::DissolveVerts => "Dissolve vertices",
            Op::DissolveEdges => "Dissolve edges",
            Op::DissolveFaces => "Dissolve faces",
            Op::Delete(_) => "Delete",
            Op::Fill => "Fill",
            Op::GridFill { .. } => "Grid fill",
            Op::Mirror { .. } => "Mirror",
            Op::Subdivide { .. } => "Subdivide",
            Op::CatmullClark { .. } => "Smooth (Catmull-Clark)",
            Op::FlipNormals => "Flip normals",
            Op::RecalculateOutside => "Recalculate outside",
            Op::MarkSharp(true) => "Mark sharp",
            Op::MarkSharp(false) => "Clear sharp",
            Op::MarkSeam(true) => "Mark seam",
            Op::MarkSeam(false) => "Clear seam",
            Op::UvBox { .. } => "Box UVs",
            Op::UvPlanar { .. } => "Planar UVs",
            Op::UvUnwrap(_) => "Unwrap UVs",
            Op::AddPrimitive { .. } => "Add shape",
        }
    }

    /// Performs the operation on `mesh` given the selection; returns the new selection, or `None` to keep the old one.
    fn apply(&self, mesh: &mut Mesh, sel: &Selection) -> ModelResult<Option<Selection>> {
        use crate::uv;
        Ok(match self {
            Op::Select(s) => Some(s.clone()),
            Op::Transform(xf) => {
                ops::transform::transform(mesh, &sel.verts, xf)?;
                None
            }
            Op::ExtrudeFaces { distance, individual } => {
                Some(ops::extrude::extrude_faces(mesh, &sel.faces, *distance, *individual)?)
            }
            Op::ExtrudeEdges { offset } => Some(ops::extrude::extrude_edges(mesh, &sel.edges, *offset)?),
            Op::ExtrudeVerts { offset } => Some(ops::extrude::extrude_verts(mesh, &sel.verts, *offset)?),
            Op::Inset { thickness, depth, individual } => {
                Some(ops::inset::inset(mesh, &sel.faces, *thickness, *depth, *individual)?)
            }
            Op::Bevel { width, segments } => Some(ops::bevel::bevel_edges(mesh, &sel.edges, *width, *segments)?),
            Op::LoopCut { edge, cuts } => Some(ops::loops::loop_cut(mesh, *edge, *cuts)?),
            Op::BridgeEdgeLoops => Some(ops::loops::bridge_edge_loops(mesh, &sel.edges)?),
            Op::MergeAtCenter => Some(ops::merge::merge_at_center(mesh, &sel.verts)?),
            Op::MergeByDistance { distance } => Some(ops::merge::merge_by_distance(mesh, &sel.verts, *distance)?.0),
            Op::DissolveVerts => Some(ops::dissolve::dissolve_verts(mesh, &sel.verts)?),
            Op::DissolveEdges => Some(ops::dissolve::dissolve_edges(mesh, &sel.edges)?),
            Op::DissolveFaces => Some(ops::dissolve::dissolve_faces(mesh, &sel.faces)?),
            Op::Delete(kind) => Some(ops::delete::delete(mesh, sel, *kind)?),
            Op::Fill => Some(ops::fill::fill(mesh, sel)?),
            Op::GridFill { span, offset } => Some(ops::fill::grid_fill(mesh, sel, *span, *offset)?),
            Op::Mirror { axis, merge_distance } => {
                Some(ops::mirror::mirror(mesh, &sel.faces, &sel.verts, *axis, *merge_distance)?)
            }
            Op::Subdivide { levels } => Some(ops::subdivide::subdivide(mesh, &sel.faces, *levels)?),
            Op::CatmullClark { levels } => Some(ops::subdivide::catmull_clark(mesh, *levels)?),
            Op::FlipNormals => {
                ops::normals::flip_normals(mesh, &sel.faces)?;
                None
            }
            Op::RecalculateOutside => {
                let faces = if sel.faces.is_empty() { mesh.face_ids().into_iter().collect() } else { sel.faces.clone() };
                ops::normals::recalculate_outside(mesh, &faces)?;
                None
            }
            Op::MarkSharp(on) => {
                ops::normals::mark_sharp(mesh, &sel.edges, *on)?;
                None
            }
            Op::MarkSeam(on) => {
                ops::normals::mark_seam(mesh, &sel.edges, *on)?;
                None
            }
            Op::UvBox { scale } => {
                uv::box_project(mesh, &sel.faces, *scale)?;
                None
            }
            Op::UvPlanar { normal, scale } => {
                uv::planar_project(mesh, &sel.faces, *normal, *scale)?;
                None
            }
            Op::UvUnwrap(options) => {
                uv::unwrap(mesh, &sel.faces, *options)?;
                None
            }
            Op::AddPrimitive { primitive, center } => {
                let faces = primitive.add_to(mesh, *center)?;
                Some(Selection::from_faces(mesh, faces))
            }
        })
    }
}

/// One undoable step: the element changes plus the selection before and after.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshCommand {
    label: String,
    changes: ChangeSet,
    selection_before: Selection,
    selection_after: Selection,
}

impl MeshCommand {
    /// Runs `op` on `edit` and returns the command that undoes/redoes it. On error nothing changes.
    ///
    /// # Errors
    /// Whatever the operation reports.
    pub fn run(edit: &mut EditMesh, op: &Op) -> ModelResult<Self> {
        let op = op.clone();
        Self::run_with(edit, op.label(), move |mesh, sel| op.apply(mesh, sel))
    }

    /// Runs a custom closure as a command (used for operations that need extra context, such as a boolean backend).
    ///
    /// # Errors
    /// Whatever `f` reports.
    pub fn run_with(
        edit: &mut EditMesh,
        label: &str,
        f: impl FnOnce(&mut Mesh, &Selection) -> ModelResult<Option<Selection>>,
    ) -> ModelResult<Self> {
        let before = edit.selection.clone();
        let mut flushed = before.clone();
        flushed.prune(&edit.mesh);
        let (new_sel, changes) = edit.mesh.record(|mesh| f(mesh, &flushed))?;
        let mut after = new_sel.unwrap_or(flushed);
        after.prune(&edit.mesh);
        edit.selection = after.clone();
        Ok(Self { label: label.to_owned(), changes, selection_before: before, selection_after: after })
    }

    /// The name shown in the undo list.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The recorded element changes.
    #[must_use]
    pub fn changes(&self) -> &ChangeSet {
        &self.changes
    }

    /// Whether the command changed nothing at all (no elements, same selection).
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty() && self.selection_before == self.selection_after
    }

    /// Restores the state before the command.
    pub fn undo(&self, edit: &mut EditMesh) {
        edit.mesh.apply_changes(&self.changes, false);
        edit.selection = self.selection_before.clone();
    }

    /// Restores the state after the command.
    pub fn redo(&self, edit: &mut EditMesh) {
        edit.mesh.apply_changes(&self.changes, true);
        edit.selection = self.selection_after.clone();
    }
}

/// A linear undo/redo stack of [`MeshCommand`]s.
#[derive(Debug, Clone, Default)]
pub struct History {
    done: Vec<MeshCommand>,
    undone: Vec<MeshCommand>,
}

impl History {
    /// An empty history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `op` and records it (clearing the redo stack). No-op commands are not recorded.
    ///
    /// # Errors
    /// Whatever the operation reports; the history and mesh are unchanged then.
    pub fn apply(&mut self, edit: &mut EditMesh, op: &Op) -> ModelResult<()> {
        let cmd = MeshCommand::run(edit, op)?;
        self.push(cmd);
        Ok(())
    }

    /// Records an already-applied command.
    pub fn push(&mut self, cmd: MeshCommand) {
        if !cmd.is_noop() {
            self.done.push(cmd);
            self.undone.clear();
        }
    }

    /// Undoes the last command; returns its label.
    pub fn undo(&mut self, edit: &mut EditMesh) -> Option<String> {
        let cmd = self.done.pop()?;
        cmd.undo(edit);
        let label = cmd.label.clone();
        self.undone.push(cmd);
        Some(label)
    }

    /// Redoes the last undone command; returns its label.
    pub fn redo(&mut self, edit: &mut EditMesh) -> Option<String> {
        let cmd = self.undone.pop()?;
        cmd.redo(edit);
        let label = cmd.label.clone();
        self.done.push(cmd);
        Some(label)
    }

    /// Takes the last command off the stack without undoing it (used by interactive previews).
    pub fn pop_last(&mut self) -> Option<MeshCommand> {
        self.done.pop()
    }

    /// Labels of the applied commands, oldest first.
    #[must_use]
    pub fn labels(&self) -> Vec<&str> {
        self.done.iter().map(|c| c.label.as_str()).collect()
    }

    /// Whether there is something to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// Whether there is something to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
}

/// Picks the extrude that fits the selection, as Blender's E key does: faces if any are selected, else edges, else
/// vertices. `distance` is used along the face normal; edges and vertices move by `offset`.
///
/// # Errors
/// [`ModelError::NothingSelected`] for an empty selection.
pub fn contextual_extrude(sel: &Selection, distance: f32, offset: Vec3) -> ModelResult<Op> {
    if !sel.faces.is_empty() {
        Ok(Op::ExtrudeFaces { distance, individual: false })
    } else if !sel.edges.is_empty() {
        Ok(Op::ExtrudeEdges { offset })
    } else if !sel.verts.is_empty() {
        Ok(Op::ExtrudeVerts { offset })
    } else {
        Err(ModelError::NothingSelected("something to extrude"))
    }
}
