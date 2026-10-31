//! Building core (#213, #214, #215): gestures through the preview overlay are one undo step,
//! the one-shot building commands, pivots, and body drags over surfaces.
use std::time::Instant;

use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_render::glam::DVec3;
use metallic_studio::commands::Command;
use metallic_studio::editor::{EditorState, Snapping, Tool};
use metallic_studio::gizmo::{DragInput, DragModifiers, GizmoDrag, GizmoFrame, Handle};
use metallic_studio::output::OutputLog;
use metallic_studio::pivot::{Pivot, Rigid};
use metallic_studio::transform::Ray;

fn state_with(parts: &[(Vec3, Vec3)]) -> (EditorState, Vec<InstanceId>, OutputLog) {
    let log = OutputLog::new(64);
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let ids = parts
        .iter()
        .map(|&(pos, size)| {
            let p = world.create("Part", Some(ws)).unwrap();
            world.set(p, "Position", Value::Vector3(pos)).unwrap();
            world.set(p, "Size", Value::Vector3(size)).unwrap();
            p
        })
        .collect();
    let dir = std::env::temp_dir();
    let state = EditorState::new(world, &dir, &dir.join("main.mscene"), "Test", log.sink());
    (state, ids, log)
}

fn pos(s: &EditorState, id: InstanceId) -> Vec3 {
    match s.world.get(id, "Position").unwrap() {
        Value::Vector3(v) => v,
        v => panic!("{v:?}"),
    }
}

fn orientation(s: &EditorState, id: InstanceId) -> Vec3 {
    match s.world.get(id, "Orientation").unwrap() {
        Value::Vector3(v) => v,
        v => panic!("{v:?}"),
    }
}

fn close(a: Vec3, b: Vec3) -> bool {
    (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.z - b.z).abs() < 1e-6
}

fn ray_down(x: f64, z: f64) -> Ray {
    Ray {
        origin: DVec3::new(x, 50.0, z),
        dir: DVec3::NEG_Y,
    }
}

fn frame(origin: DVec3, tool: Tool) -> GizmoFrame {
    GizmoFrame {
        origin,
        axes: [DVec3::X, DVec3::Y, DVec3::Z],
        length: 5.0,
        tool,
    }
}

fn drag_frames(s: &mut EditorState, d: &mut GizmoDrag, frames: usize, to: f64) {
    let snapping = s.snapping;
    for i in 1..=frames {
        let x = to * i as f64 / frames as f64;
        d.update(&DragInput::simple(
            ray_down(x, 0.0),
            &snapping,
            DragModifiers::default(),
        ));
        d.apply(&mut s.world, &mut s.preview);
    }
}

#[test]
fn a_500_part_drag_over_60_frames_is_one_step_and_esc_changes_nothing() {
    let parts: Vec<_> = (0..500)
        .map(|i| {
            (
                Vec3::new(f64::from(i % 25) * 2.0, 0.0, f64::from(i / 25) * 2.0),
                Vec3::new(1.0, 1.0, 1.0),
            )
        })
        .collect();
    let (mut s, ids, _log) = state_with(&parts);
    let before: Vec<Vec3> = ids.iter().map(|&id| pos(&s, id)).collect();
    s.selection.set(ids.clone());

    let f = frame(DVec3::ZERO, Tool::Move);
    let mut d = GizmoDrag::begin(&s.world, f, Handle::Axis(0), &ray_down(0.0, 0.0), &ids).unwrap();
    drag_frames(&mut s, &mut d, 60, 6.0);
    assert!(s.finish_drag(d));
    assert_eq!(s.history.len(), 1, "60 frames, one step");
    assert_eq!(
        s.history.undo_name(&s.world).as_deref(),
        Some("Move 500 parts")
    );
    for (&id, b) in ids.iter().zip(&before) {
        assert!(close(pos(&s, id), Vec3::new(b.x + 6.0, b.y, b.z)));
    }
    s.undo();
    for (&id, b) in ids.iter().zip(&before) {
        assert!(close(pos(&s, id), *b), "one undo restores all");
    }

    let mut d = GizmoDrag::begin(&s.world, f, Handle::Axis(0), &ray_down(0.0, 0.0), &ids).unwrap();
    drag_frames(&mut s, &mut d, 60, 9.0);
    s.cancel_drag(d);
    for (&id, b) in ids.iter().zip(&before) {
        assert!(close(pos(&s, id), *b), "Esc leaves the world as it was");
    }
    assert!(!s.preview.is_active());
    assert_eq!(
        s.history.redo_name(&s.world).as_deref(),
        Some("Move 500 parts")
    );
}

#[test]
fn committing_a_1000_part_drag_fits_the_budget() {
    let parts: Vec<_> = (0..1000)
        .map(|i| {
            (
                Vec3::new(f64::from(i % 40) * 2.0, 0.0, f64::from(i / 40) * 2.0),
                Vec3::new(1.0, 1.0, 1.0),
            )
        })
        .collect();
    let (mut s, ids, _log) = state_with(&parts);
    let f = frame(DVec3::ZERO, Tool::Move);
    let mut d = GizmoDrag::begin(&s.world, f, Handle::Axis(0), &ray_down(0.0, 0.0), &ids).unwrap();
    drag_frames(&mut s, &mut d, 3, 4.0);
    let t = Instant::now();
    assert!(s.finish_drag(d));
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!("commit of 1,000 parts: {ms:.3} ms");
    // Spec 17: under 10 ms in release; debug builds get room.
    let budget = if cfg!(debug_assertions) { 200.0 } else { 10.0 };
    assert!(ms < budget, "{ms} ms");
}

#[test]
fn alt_drag_duplicate_and_move_is_one_step_and_an_unmoved_copy_is_dropped() {
    let (mut s, ids, _log) = state_with(&[(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 2.0))]);
    let original = ids[0];
    s.selection.set([original]);
    let f = frame(DVec3::ZERO, Tool::Move);

    let mut dup = Command::Duplicate {
        sources: vec![original],
        offset: Vec3::new(0.0, 0.0, 0.0),
        copies: Vec::new(),
    };
    s.history.apply_pending(&mut dup, &mut s.world).unwrap();
    let copy = dup.created()[0];
    let mut d = GizmoDrag::begin(&s.world, f, Handle::Axis(0), &ray_down(0.0, 0.0), &[copy])
        .unwrap()
        .with_pending(dup);
    drag_frames(&mut s, &mut d, 10, 4.0);
    assert!(s.finish_drag(d));
    assert_eq!(s.history.len(), 1);
    assert_eq!(
        s.history.undo_name(&s.world).as_deref(),
        Some("Duplicate and move")
    );
    assert!(close(pos(&s, copy), Vec3::new(4.0, 0.0, 0.0)));
    assert!(close(pos(&s, original), Vec3::new(0.0, 0.0, 0.0)));
    s.undo();
    assert!(s.history.is_parked(copy), "undo removes the copy");
    s.redo();
    assert!(close(pos(&s, copy), Vec3::new(4.0, 0.0, 0.0)));

    // Alt-click without moving: the copy goes away and no step is added.
    let mut dup = Command::Duplicate {
        sources: vec![original],
        offset: Vec3::new(0.0, 0.0, 0.0),
        copies: Vec::new(),
    };
    s.history.apply_pending(&mut dup, &mut s.world).unwrap();
    let copy2 = dup.created()[0];
    let d = GizmoDrag::begin(&s.world, f, Handle::Axis(0), &ray_down(0.0, 0.0), &[copy2])
        .unwrap()
        .with_pending(dup);
    assert!(!s.finish_drag(d));
    assert!(!s.world.is_alive(copy2));
    assert_eq!(s.history.len(), 1);
}

#[test]
fn ctrl_r_turns_about_the_pivot_and_ctrl_t_tilts() {
    let (mut s, ids, _log) = state_with(&[(Vec3::new(2.0, 0.0, 0.0), Vec3::new(4.0, 1.0, 2.0))]);
    let p = ids[0];
    s.selection.set([p]);
    assert!(s.rotate_selection(DVec3::Y, 90.0, "Rotate"));
    assert!(close(orientation(&s, p), Vec3::new(0.0, 90.0, 0.0)));
    assert!(
        close(pos(&s, p), Vec3::new(2.0, 0.0, 0.0)),
        "about its own pivot"
    );
    assert_eq!(
        s.history.undo_name(&s.world).as_deref(),
        Some("Rotate 1 part")
    );

    s.view_right = DVec3::new(0.9, 0.0, -0.3);
    assert_eq!(s.tilt_axis(), DVec3::X);
    let axis = s.tilt_axis();
    assert!(s.rotate_selection(axis, 90.0, "Tilt"));
    assert_eq!(s.history.len(), 2);
    s.undo();
    s.undo();
    assert!(close(orientation(&s, p), Vec3::new(0.0, 0.0, 0.0)));
}

#[test]
fn end_drops_onto_the_part_below() {
    let (mut s, ids, _log) = state_with(&[
        (Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 1.0, 10.0)),
        (Vec3::new(1.0, 7.0, 1.0), Vec3::new(2.0, 2.0, 2.0)),
    ]);
    s.selection.set([ids[1]]);
    assert!(s.drop_selection());
    assert!(close(pos(&s, ids[1]), Vec3::new(1.0, 1.5, 1.0)));
    assert_eq!(
        s.history.undo_name(&s.world).as_deref(),
        Some("Drop 1 part")
    );
    assert!(!s.drop_selection(), "already resting");
}

#[test]
fn snap_to_grid_moves_the_reference_point_and_rounds_the_turn() {
    // Resting on the ground: y = 0.5 puts the bottom face on y = 0.
    let (mut s, ids, _log) = state_with(&[
        (Vec3::new(0.3, 0.5, 0.5), Vec3::new(1.0, 1.0, 1.0)),
        (Vec3::new(5.3, 0.5, 0.5), Vec3::new(0.5, 1.0, 1.0)),
        (Vec3::new(9.0, 0.5, 0.0), Vec3::new(2.0, 1.0, 2.0)),
    ]);
    s.world
        .set(
            ids[0],
            "Orientation",
            Value::Vector3(Vec3::new(0.0, 7.0, 0.0)),
        )
        .unwrap();
    // A flush 2 x 1 x 2 plate (faces on grid lines) is on the grid.
    s.selection.set([ids[2]]);
    assert!(!s.off_grid(), "flush faces are on the grid (spec 2.2)");
    s.selection.set(ids[..2].to_vec());
    assert!(s.off_grid());
    assert!(s.snap_selection_to_grid());
    // The 1-stud cube's min corner (-0.2) goes to 0: faces on 0 and 1, centre on 0.5.
    assert!(close(pos(&s, ids[0]), Vec3::new(0.5, 0.5, 0.5)));
    assert!(close(orientation(&s, ids[0]), Vec3::new(0.0, 0.0, 0.0)));
    // The 0.5-wide part's min corner (5.05) goes to 5: centre on 5.25.
    assert!(close(pos(&s, ids[1]), Vec3::new(5.25, 0.5, 0.5)));
    assert!(!s.off_grid());
    assert_eq!(s.history.len(), 1);
    assert_eq!(
        s.history.undo_name(&s.world).as_deref(),
        Some("Snap 2 parts to grid")
    );
}

#[test]
fn an_edited_pivot_moves_the_turn_centre_and_resets_in_one_step() {
    let (mut s, ids, _log) = state_with(&[(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0))]);
    let p = ids[0];
    s.selection.set([p]);
    let corner = DVec3::new(1.0, -1.0, 1.0);
    assert!(s.set_pivot(p, Pivot::at(corner)));
    assert_eq!(s.gizmo_pivot().unwrap().position, corner);
    assert!(s.rotate_selection(DVec3::Y, 90.0, "Rotate"));
    // The centre (0,0,0) turns 90 degrees about the corner (1,-1,1): to (0, 0, 2).
    let p_now = pos(&s, p);
    assert!(close(p_now, Vec3::new(0.0, 0.0, 2.0)), "{p_now:?}");
    // The pivot is part-relative, so it turned with the part and stays on the corner.
    assert!((s.gizmo_pivot().unwrap().position - corner).length() < 1e-9);
    assert!(s.reset_pivot());
    assert_eq!(
        s.history.undo_name(&s.world).as_deref(),
        Some("Reset pivot of Part")
    );
    assert!((s.gizmo_pivot().unwrap().position - DVec3::new(0.0, 0.0, 2.0)).length() < 1e-9);
    s.undo();
    assert!((s.gizmo_pivot().unwrap().position - corner).length() < 1e-9);
}

#[test]
fn changing_the_last_drag_replaces_its_step() {
    let (mut s, ids, _log) = state_with(&[(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0))]);
    let p = ids[0];
    s.selection.set([p]);
    let f = frame(DVec3::ZERO, Tool::Move);
    let mut d = GizmoDrag::begin(&s.world, f, Handle::Axis(0), &ray_down(0.0, 0.0), &ids).unwrap();
    drag_frames(&mut s, &mut d, 5, 2.0);
    let kept = d.clone();
    assert!(s.finish_drag(d));
    assert!(close(pos(&s, p), Vec3::new(2.0, 0.0, 0.0)));
    assert!(s.amend_drag(&kept, 5.0));
    assert!(close(pos(&s, p), Vec3::new(5.0, 0.0, 0.0)));
    assert_eq!(s.history.len(), 1, "the change merges into the same step");
    s.undo();
    assert!(close(pos(&s, p), Vec3::new(0.0, 0.0, 0.0)));
}

#[test]
fn a_body_drag_lands_on_the_surface_under_the_cursor() {
    let (mut s, ids, _log) = state_with(&[
        (Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0)),
        (Vec3::new(5.0, 5.0, 5.0), Vec3::new(2.0, 2.0, 2.0)),
    ]);
    let moving = ids[1];
    s.selection.set([moving]);
    s.picks.sync(&s.world);
    let grab = DVec3::new(5.0, 6.0, 5.0);
    let f = frame(grab, Tool::Select);
    let mut d = GizmoDrag::begin_body(
        &s.world,
        f,
        &ray_down(5.0, 5.0),
        &[moving],
        grab,
        Some(moving),
    )
    .unwrap();
    let snapping = Snapping::default();
    let mut input = DragInput::simple(ray_down(-2.3, 3.4), &snapping, DragModifiers::default());
    input.picks = Some(&s.picks);
    d.update(&input);
    d.apply(&mut s.world, &mut s.preview);
    let p = pos(&s, moving);
    assert!(
        (p.y - 1.5).abs() < 1e-9,
        "bottom touches the floor top: {p:?}"
    );
    assert!((p.x - p.x.round()).abs() < 1e-9 && (p.z - p.z.round()).abs() < 1e-9);
    assert!((p.x - -2.3).abs() <= 0.5 && (p.z - 3.4).abs() <= 0.5);
    assert_eq!(d.surface.and_then(|s| s.part), Some(ids[0]));

    // Off the floor, the drag continues on the plane of the last surface it touched.
    let mut input = DragInput::simple(
        Ray {
            origin: DVec3::new(40.0, 50.0, 0.0),
            dir: DVec3::new(0.0, -1.0, 0.0),
        },
        &snapping,
        DragModifiers::default(),
    );
    input.picks = Some(&s.picks);
    d.update(&input);
    d.apply(&mut s.world, &mut s.preview);
    let p = pos(&s, moving);
    assert!(
        (p.y - 1.5).abs() < 1e-9 && (p.x - 40.0).abs() < 1e-9,
        "{p:?}"
    );

    // R turns it 90 degrees about the surface normal.
    d.turn(d.turn_axis());
    d.update(&input);
    d.apply(&mut s.world, &mut s.preview);
    assert!(close(orientation(&s, moving), Vec3::new(0.0, 90.0, 0.0)));
    assert!(s.finish_drag(d));
    assert_eq!(s.history.len(), 1);
}

fn saved_state(dir: &std::path::Path) -> (EditorState, InstanceId, InstanceId, OutputLog) {
    let log = OutputLog::new(64);
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let model = world.create("Model", Some(ws)).unwrap();
    let part = world.create("Part", Some(model)).unwrap();
    world
        .set(part, "Size", Value::Vector3(Vec3::new(2.0, 2.0, 2.0)))
        .unwrap();
    let scene = dir.join("main.mscene");
    let state = EditorState::new(world, dir, &scene, "Test", log.sink());
    (state, model, part, log)
}

fn pivot_of(world: &World, id: InstanceId) -> DVec3 {
    let cf = metallic_core::transform::get_pivot(world, id).unwrap();
    DVec3::new(cf.position.x, cf.position.y, cf.position.z)
}

#[test]
fn an_edited_part_pivot_is_one_property_step_saved_and_undone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _model, part, _log) = saved_state(dir.path());
    s.selection.set([part]);
    let corner = DVec3::new(1.0, -1.0, 1.0);
    assert!(s.set_pivot(part, Pivot::at(corner)));
    assert_eq!(s.history.len(), 1);
    assert!(matches!(
        s.history.peek_undo(),
        Some(Command::SetProperties { .. })
    ));
    assert!((pivot_of(&s.world, part) - corner).length() < 1e-9);

    s.save().unwrap();
    let reloaded = metallic_core::load_scene(&dir.path().join("main.mscene")).unwrap();
    let ws = reloaded.root("Workspace").unwrap();
    let m = reloaded.children(ws)[0];
    let p = reloaded.children(m)[0];
    assert!(
        (pivot_of(&reloaded, p) - corner).length() < 1e-9,
        "PivotOffset is saved"
    );

    s.undo();
    assert!(
        pivot_of(&s.world, part).length() < 1e-9,
        "undo restores the centre"
    );
    s.redo();
    assert!(s.reset_pivot());
    assert!(pivot_of(&s.world, part).length() < 1e-9);
}

#[test]
fn a_model_world_pivot_from_properties_is_used_saved_and_moves_with_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, model, part, _log) = saved_state(dir.path());
    s.selection.set([model]);
    let target = metallic_core::CFrame::from_position(Vec3::new(5.0, 1.0, 0.0));
    let mut props = metallic_studio::properties::Properties::new();
    props.set(&mut s, "WorldPivot", Value::CFrame(Box::new(target)));
    assert_eq!(
        s.history.len(),
        1,
        "WorldPivot and WorldPivotSet are one step"
    );
    assert!((pivot_of(&s.world, model) - DVec3::new(5.0, 1.0, 0.0)).length() < 1e-9);

    s.save().unwrap();
    let reloaded = metallic_core::load_scene(&dir.path().join("main.mscene")).unwrap();
    let ws = reloaded.root("Workspace").unwrap();
    let m = reloaded.children(ws)[0];
    assert!((pivot_of(&reloaded, m) - DVec3::new(5.0, 1.0, 0.0)).length() < 1e-9);

    // Moving the Model moves its set pivot in the same step (PivotTo semantics).
    assert!(s.transform_selection(Rigid::translate(DVec3::new(0.0, 0.0, 3.0)), "Move"));
    assert!((pivot_of(&s.world, model) - DVec3::new(5.0, 1.0, 3.0)).length() < 1e-9);
    assert!(close(pos(&s, part), Vec3::new(0.0, 0.0, 3.0)));
    s.undo();
    assert!((pivot_of(&s.world, model) - DVec3::new(5.0, 1.0, 0.0)).length() < 1e-9);
    s.undo();
    assert!(
        (pivot_of(&s.world, model) - DVec3::ZERO).length() < 1e-9,
        "unset again: back to the box centre"
    );
}

#[test]
fn a_gizmo_pivot_drag_is_one_property_step() {
    let (mut s, ids, _log) = state_with(&[(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0))]);
    let p = ids[0];
    s.selection.set([p]);
    let f = frame(DVec3::ZERO, Tool::Move);
    let points = metallic_studio::pivot::candidates(&s.world, p).unwrap();
    let mut d = GizmoDrag::begin_pivot(
        f,
        Handle::Axis(0),
        &ray_down(0.0, 0.0),
        p,
        "Part".into(),
        Pivot::IDENTITY,
        points,
    )
    .unwrap();
    drag_frames(&mut s, &mut d, 10, 3.0);
    assert!(s.finish_drag(d));
    assert_eq!(s.history.len(), 1);
    assert_eq!(
        s.history.undo_name(&s.world).as_deref(),
        Some("Edit pivot of Part")
    );
    assert!((pivot_of(&s.world, p) - DVec3::new(3.0, 0.0, 0.0)).length() < 1e-9);
    assert!(
        close(pos(&s, p), Vec3::new(0.0, 0.0, 0.0)),
        "the part stays"
    );
    s.undo();
    assert!(pivot_of(&s.world, p).length() < 1e-9);
}

#[test]
fn a_world_pivot_that_cannot_be_made_orthonormal_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, model, _part, _log) = saved_state(dir.path());
    s.selection.set([model]);
    let degenerate = metallic_core::CFrame {
        position: Vec3::new(1.0, 2.0, 3.0),
        rotation: [[0.0; 3]; 3],
    };
    let mut props = metallic_studio::properties::Properties::new();
    props.set(&mut s, "WorldPivot", Value::CFrame(Box::new(degenerate)));
    assert!(s.history.is_empty(), "nothing written");
    assert_eq!(
        s.world.get(model, "WorldPivotSet").unwrap(),
        Value::Bool(false)
    );
    assert!(
        s.current_note()
            .unwrap_or("")
            .contains("WorldPivot not set")
    );
}
