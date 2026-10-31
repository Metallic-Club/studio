//! The building tools, the colour popover and the Input Actions panel wired into the editor
//! (#319): each tool family commits one undo step and undo restores the world exactly; the
//! Input Actions panel records into the editor's history and follows rebuilt ids.
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_render::glam::DVec3;
use metallic_studio::actions::ToolbarGroup;
use metallic_studio::commands::Command;
use metallic_studio::editor::{EditorState, Tool};
use metallic_studio::editor_tools::arrange::{ArrangeSettings, ArrayMode, run_align, run_array};
use metallic_studio::editor_tools::{Modal, Popover};
use metallic_studio::editor_ui::EditorUi;
use metallic_studio::output::OutputPanel;
use metallic_studio::toolbar::MORE_LABEL;
use metallic_studio::tools::arrange::{Axis, Edge};
use metallic_studio::tools::select::SelectMode;

struct Scene {
    base: InstanceId,
    a: InstanceId,
    b: InstanceId,
    c: InstanceId,
}

fn part(world: &mut World, parent: InstanceId, name: &str, pos: Vec3, size: Vec3) -> InstanceId {
    let p = world.create("Part", Some(parent)).unwrap();
    world.set(p, "Name", Value::String(name.into())).unwrap();
    world.set(p, "Size", Value::Vector3(size)).unwrap();
    world.set(p, "Position", Value::Vector3(pos)).unwrap();
    world.set(p, "Anchored", Value::Bool(true)).unwrap();
    p
}

fn editor() -> (EditorUi, Scene) {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let base = part(
        &mut world,
        ws,
        "Baseplate",
        Vec3::new(0.0, -0.5, 0.0),
        Vec3::new(64.0, 1.0, 64.0),
    );
    let a = part(
        &mut world,
        ws,
        "A",
        Vec3::new(-8.0, 2.0, 0.0),
        Vec3::new(4.0, 4.0, 4.0),
    );
    let b = part(
        &mut world,
        ws,
        "B",
        Vec3::new(6.0, 1.0, 3.0),
        Vec3::new(2.0, 2.0, 2.0),
    );
    world
        .set(b, "Orientation", Value::Vector3(Vec3::new(0.0, 30.0, 0.0)))
        .unwrap();
    let c = part(
        &mut world,
        ws,
        "C",
        Vec3::new(12.0, 1.0, -6.0),
        Vec3::new(2.0, 2.0, 2.0),
    );
    let output = OutputPanel::new();
    let mut state = EditorState::new(
        world,
        std::path::Path::new("."),
        std::path::Path::new("unused.mscene"),
        "Test",
        output.log.sink(),
    );
    state.can_save = false;
    (EditorUi::new(state, output), Scene { base, a, b, c })
}

/// The panels use the bundled font families, which the theme installs for the next pass: the
/// first pass only installs them.
fn harness(ui: EditorUi) -> Harness<'static, EditorUi> {
    harness_sized(ui, egui::vec2(1600.0, 900.0))
}

fn harness_sized(ui: EditorUi, size: egui::Vec2) -> Harness<'static, EditorUi> {
    let mut themed = false;
    let mut h = Harness::builder().with_size(size).build_ui_state(
        move |ui, editor: &mut EditorUi| {
            if !themed {
                metallic_studio::theme::apply(ui.ctx(), metallic_studio::theme::ThemeId::Graphite);
                themed = true;
                ui.ctx().request_repaint();
                return;
            }
            editor.ui(ui, None);
        },
        ui,
    );
    h.run_steps(3);
    h
}

fn button(h: &mut Harness<'static, EditorUi>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    });
    h.run_steps(2);
}

fn click(h: &mut Harness<'static, EditorUi>, pos: Pos2) {
    h.hover_at(pos);
    h.run_steps(2);
    button(h, pos, true, Modifiers::NONE);
    button(h, pos, false, Modifiers::NONE);
}

fn screen(h: &Harness<'static, EditorUi>, p: DVec3) -> Pos2 {
    h.state().viewport.screen_pos(p).expect("in view")
}

/// Every live instance under Workspace with the properties tools change.
fn snapshot(state: &EditorState) -> Vec<(InstanceId, String, Vec<Value>)> {
    let mut out = Vec::new();
    let mut stack = vec![state.world.root("Workspace").unwrap()];
    while let Some(id) = stack.pop() {
        if !state.is_live(id) {
            continue;
        }
        let props = ["Name", "Position", "Orientation", "Size", "Color"]
            .iter()
            .filter_map(|p| state.world.get(id, p).ok())
            .collect();
        out.push((
            id,
            state.world.class_name(id).unwrap_or("").to_string(),
            props,
        ));
        stack.extend(state.world.children(id).iter().copied());
    }
    out.sort_by_key(|e| e.0.to_bits());
    out
}

fn live_input_actions(state: &EditorState) -> Vec<InstanceId> {
    let mut out = Vec::new();
    let mut stack = vec![state.world.root("ReplicatedStorage").unwrap()];
    while let Some(id) = stack.pop() {
        if state.world.class_name(id) == Some("InputAction") && state.is_live(id) {
            out.push(id);
        }
        stack.extend(state.world.children(id).iter().copied());
    }
    out
}

#[test]
fn input_actions_panel_records_into_editor_undo_and_follows_new_ids() {
    let (ui, _) = editor();
    let mut h = harness(ui);
    h.state_mut().run_action("view.input_actions");
    h.run_steps(4);
    assert!(h.state().input_actions.is_open());

    // Create an action in the panel.
    h.get_by_label("Add").click();
    h.run_steps(3);
    h.get_by_label("Empty action").click();
    h.run_steps(4);
    let made = live_input_actions(&h.state().state);
    assert_eq!(made.len(), 1, "the panel added one action");
    let a0 = made[0];
    {
        let ui = h.state();
        assert_eq!(ui.input_actions.panel().selected_instance(), Some(a0));
        assert!(
            !ui.input_actions.panel().owns_history(),
            "the editor owns undo"
        );
        assert_eq!(ui.state.history.len(), 1);
        assert!(matches!(
            ui.state.history.peek_undo(),
            Some(Command::Step(_))
        ));
    }

    // Undo in the editor: the action is gone.
    h.state_mut().run_action("edit.undo");
    h.run_steps(3);
    assert!(live_input_actions(&h.state().state).is_empty());

    // Redo rebuilds it with a new id.
    h.state_mut().run_action("edit.redo");
    h.run_steps(3);
    let a1 = live_input_actions(&h.state().state)[0];
    assert_ne!(a1, a0, "rebuilt with a new id");
    {
        let ui = h.state_mut();
        let Value::String(name) = ui.state.world.get(a1, "Name").unwrap() else {
            panic!()
        };
        ui.input_actions
            .panel_mut()
            .select_action(&ui.state.world, &name);
    }
    h.run_steps(2);

    // Delete in the panel, then undo in the editor.
    h.get_by_label("Delete (Delete)").click();
    h.run_steps(4);
    assert!(live_input_actions(&h.state().state).is_empty());
    assert_eq!(h.state().state.history.len(), 2);
    h.state_mut().run_action("edit.undo");
    h.run_steps(3);
    let a2 = live_input_actions(&h.state().state)[0];
    assert_ne!(a2, a1);

    // Select it and hide the panel; redo the delete and undo it again from the editor. The
    // panel's selection (and its other ids) moves to the rebuilt action's new id.
    {
        let ui = h.state_mut();
        let name = match ui.state.world.get(a2, "Name").unwrap() {
            Value::String(s) => s,
            _ => panic!(),
        };
        ui.input_actions
            .panel_mut()
            .select_action(&ui.state.world, &name);
        assert_eq!(ui.input_actions.panel().selected_instance(), Some(a2));
        ui.input_actions.set_open(false);
    }
    h.run_steps(2);
    h.state_mut().run_action("edit.redo");
    h.run_steps(2);
    assert!(live_input_actions(&h.state().state).is_empty());
    h.state_mut().run_action("edit.undo");
    h.run_steps(2);
    let a3 = live_input_actions(&h.state().state)[0];
    assert_ne!(a3, a2);
    assert_eq!(
        h.state().input_actions.panel().selected_instance(),
        Some(a3),
        "the selection followed the remap"
    );
    h.state_mut().run_action("view.input_actions");
    h.run_steps(3);
    assert_eq!(
        h.state().input_actions.panel().selected_instance(),
        Some(a3)
    );
    // Undo everything: no action is left, and the history agrees.
    h.state_mut().run_action("edit.undo");
    h.run_steps(2);
    assert!(live_input_actions(&h.state().state).is_empty());
    assert!(h.state().state.history.is_empty());
}

#[test]
fn colour_popover_from_properties_previews_live_and_commits_one_step() {
    let (ui, sc) = editor();
    let mut h = harness(ui);
    h.state_mut().state.tool = Tool::Select;
    h.state_mut().state.selection.set([sc.b, sc.c]);
    h.run_steps(3);
    let before = snapshot(&h.state().state);
    let old = h.state().state.world.get(sc.b, "Color").unwrap();

    h.get_by_label("Color swatch").click();
    h.run_steps(6);
    assert!(h.state().colour.is_open(), "the swatch opens the popover");

    // Drag across the saturation/value square: a live preview, no undo step yet.
    let centre = h.state().colour.popover().wheel_centre();
    h.hover_at(centre);
    h.run_steps(2);
    button(&mut h, centre, true, Modifiers::NONE);
    let to = centre + egui::vec2(40.0, -40.0);
    h.hover_at(to);
    h.run_steps(3);
    {
        let s = &h.state().state;
        assert_ne!(s.world.get(sc.b, "Color").unwrap(), old, "previewing");
        assert_eq!(
            s.world.get(sc.b, "Color").unwrap(),
            s.world.get(sc.c, "Color").unwrap(),
            "every target previews"
        );
        assert!(s.history.is_empty(), "no step while dragging");
    }
    button(&mut h, to, false, Modifiers::NONE);
    h.run_steps(2);
    {
        let s = &h.state().state;
        assert_eq!(s.history.len(), 1, "one drag, one undo step");
        let name = s.history.undo_name(&s.world).unwrap();
        assert!(name.starts_with("Colour: #"), "{name}");
        assert!(name.ends_with("(2 parts)"), "{name}");
    }
    h.state_mut().run_action("edit.undo");
    h.run_steps(2);
    assert_eq!(snapshot(&h.state().state), before, "undo restores exactly");
}

#[test]
fn arrange_ops_are_one_undo_step_each_and_undo_restores_exactly() {
    let (mut ui, sc) = editor();
    ui.state.selection.set([sc.a, sc.b, sc.c]);
    let before = snapshot(&ui.state);
    let settings = ArrangeSettings::default();

    // Align.
    run_align(&mut ui.state, &settings, Axis::Y, Edge::Max);
    assert_eq!(ui.state.history.len(), 1);
    assert_ne!(snapshot(&ui.state), before);
    ui.run_action("edit.undo");
    assert_eq!(snapshot(&ui.state), before);

    // Distribute through its action and popover settings.
    metallic_studio::editor_tools::arrange::run_distribute(&mut ui.state, &settings, Axis::X);
    assert_eq!(ui.state.history.len(), 1);
    ui.run_action("edit.undo");
    assert_eq!(snapshot(&ui.state), before);

    // Mirror in place and mirror copy (Ctrl+M, Ctrl+Shift+M through the registry).
    ui.run_action("build.mirror");
    assert_eq!(ui.state.history.len(), 1);
    ui.run_action("edit.undo");
    assert_eq!(snapshot(&ui.state), before);
    ui.run_action("build.mirror_copy");
    assert_eq!(ui.state.history.len(), 1);
    let copies = ui.state.selection.ids().to_vec();
    assert_eq!(copies.len(), 3, "the copies are selected");
    ui.run_action("edit.undo");
    assert_eq!(snapshot(&ui.state), before);
    ui.run_action("edit.redo");
    assert!(
        copies.iter().all(|&c| ui.state.is_live(c)),
        "redo brings the same copies back"
    );
    ui.run_action("edit.undo");
    ui.state.selection.set([sc.a, sc.b, sc.c]);

    // Arrays: linear (grouped), radial, grid and path, each one step.
    let mut s = ArrangeSettings::default();
    for mode in [
        ArrayMode::Linear,
        ArrayMode::Radial,
        ArrayMode::Grid,
        ArrayMode::Path,
    ] {
        s.mode = mode;
        s.path.points = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 5.0),
            DVec3::new(20.0, 0.0, 0.0),
        ];
        let steps = ui.state.history.len();
        assert!(run_array(&mut ui.state, &mut s), "{mode:?}");
        assert_eq!(ui.state.history.len(), steps + 1, "{mode:?}: one step");
        ui.run_action("edit.undo");
        assert_eq!(
            snapshot(&ui.state),
            before,
            "{mode:?}: undo restores exactly"
        );
    }
    // The linear array grouped its copies under a Model "Array".
    s.mode = ArrayMode::Linear;
    s.linear.count = 2;
    assert!(run_array(&mut ui.state, &mut s));
    let ws = ui.state.world.root("Workspace").unwrap();
    let array = ui
        .state
        .world
        .find_first_child(ws, "Array")
        .expect("grouped");
    assert_eq!(ui.state.world.children(array).len(), 6);
    ui.run_action("edit.undo");

    // The copy cap is an error in Output, not a panic or a step.
    s.linear.count = 5000;
    assert!(!run_array(&mut ui.state, &mut s));
    assert!(ui.state.history.is_empty());
    assert_eq!(snapshot(&ui.state), before);

    // A opens the Align popover only when it applies.
    ui.state.selection.set([sc.a]);
    assert!(!ui.actions.get("build.align").unwrap().is_enabled(&ui.state));
    ui.state.selection.set([sc.a, sc.b]);
    ui.run_action("build.align");
    assert_eq!(ui.tools.state().popover, Some(Popover::Align));
    let _ = sc.base;
}

#[test]
fn a_brush_stroke_is_one_step_and_erase_removes_only_its_own_instances() {
    let (ui, sc) = editor();
    let mut h = harness(ui);
    h.state_mut().state.selection.set([sc.c]);
    h.state_mut().run_action("tool.brush");
    h.run_steps(2);
    assert_eq!(h.state().tools.state().modal, Some(Modal::Brush));
    assert!(
        h.state().tools.state().brush.set.is_some(),
        "B uses the selection"
    );
    let before = snapshot(&h.state().state);

    // Paint: press over the baseplate, hold a few frames, move, release.
    let at = screen(&h, DVec3::new(-16.0, 0.0, 16.0));
    h.hover_at(at);
    h.run_steps(2);
    button(&mut h, at, true, Modifiers::NONE);
    h.hover_at(at + egui::vec2(30.0, 0.0));
    h.run_steps(4);
    button(&mut h, at + egui::vec2(30.0, 0.0), false, Modifiers::NONE);
    let ws = h.state().state.world.root("Workspace").unwrap();
    let painted = {
        let s = &h.state().state;
        assert_eq!(s.history.len(), 1, "one stroke, one step");
        let name = s.history.undo_name(&s.world).unwrap();
        assert!(name.starts_with("Brush C ("), "{name}");
        let brush = s.world.find_first_child(ws, "Brush").expect("brush folder");
        let set = s.world.find_first_child(brush, "C").expect("set folder");
        s.world.children(set).to_vec()
    };
    assert!(!painted.is_empty(), "the stroke placed copies");
    let after_paint = snapshot(&h.state().state);
    h.state_mut().run_action("edit.undo");
    h.run_steps(2);
    assert_eq!(snapshot(&h.state().state), before, "undo restores exactly");
    h.state_mut().run_action("edit.redo");
    h.run_steps(2);
    assert_eq!(
        snapshot(&h.state().state),
        after_paint,
        "redo brings the same copies"
    );

    // Shift-drag erases this brush's copies only, as one step.
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    button(&mut h, at, true, Modifiers::SHIFT);
    h.run_steps(2);
    button(&mut h, at, false, Modifiers::SHIFT);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
    {
        let s = &h.state().state;
        assert_eq!(s.history.len(), 2);
        let name = s.history.undo_name(&s.world).unwrap();
        assert!(name.starts_with("Erase C ("), "{name}");
        assert!(painted.iter().any(|&p| !s.is_live(p)), "erased some");
        assert!(
            s.is_live(sc.base) && s.is_live(sc.c),
            "never what it did not place"
        );
    }
    h.state_mut().run_action("edit.undo");
    h.run_steps(2);
    assert_eq!(snapshot(&h.state().state), after_paint);
}

#[test]
fn selection_modes_select_similar_and_measure_from_the_viewport() {
    let (ui, sc) = editor();
    let mut h = harness(ui);
    let centre = h.state().viewport.rect().center();
    h.hover_at(centre);
    h.run_steps(2);
    // Q selects the Select tool, Q again cycles to Lasso.
    h.key_press(egui::Key::Q);
    h.run_steps(2);
    h.key_press(egui::Key::Q);
    h.run_steps(2);
    assert_eq!(h.state().state.tool, Tool::Select);
    assert_eq!(h.state().tools.state().select_mode, SelectMode::Lasso);

    // A lasso around A's centre selects A only.
    let a = screen(&h, DVec3::new(-8.0, 2.0, 0.0));
    let ring = [(-25.0, -25.0), (25.0, -25.0), (25.0, 25.0), (-25.0, 25.0)];
    let first = a + egui::vec2(ring[0].0, ring[0].1);
    h.hover_at(first);
    h.run_steps(2);
    button(&mut h, first, true, Modifiers::NONE);
    for (x, y) in ring.iter().skip(1) {
        h.hover_at(a + egui::vec2(*x, *y));
        h.run_steps(1);
    }
    button(&mut h, first, false, Modifiers::NONE);
    assert_eq!(h.state().state.selection.ids(), &[sc.a]);
    assert!(
        h.state().state.history.is_empty(),
        "selecting is not an edit"
    );

    // Select similar (Shift+G, then S): B and C share a size.
    h.state_mut().state.selection.set([sc.b]);
    h.hover_at(centre);
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::SHIFT, egui::Key::G);
    h.run_steps(2);
    assert_eq!(h.state().tools.state().popover, Some(Popover::Similar));
    h.key_press(egui::Key::S);
    h.run_steps(2);
    let mut got = h.state().state.selection.ids().to_vec();
    got.sort_by_key(|i| i.to_bits());
    let mut want = vec![sc.b, sc.c];
    want.sort_by_key(|i| i.to_bits());
    assert_eq!(got, want);

    // Measure (Shift+M): two clicks near A's top corners snap to them.
    h.hover_at(centre);
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::SHIFT, egui::Key::M);
    h.run_steps(2);
    assert_eq!(h.state().tools.state().modal, Some(Modal::Measure));
    let p = DVec3::new(-10.0, 4.0, -2.0);
    let q = DVec3::new(-6.0, 4.0, 2.0);
    let top = screen(&h, DVec3::new(-8.0, 4.0, 0.0));
    for corner in [p, q] {
        let s = screen(&h, corner);
        // A few points in from the corner, so the ray hits the part.
        click(&mut h, s + (top - s).normalized() * 4.0);
    }
    let m = *h
        .state()
        .tools
        .state()
        .measure
        .measurements()
        .last()
        .expect("measured");
    assert!(
        (m.distance() - p.distance(q)).abs() < 1e-4,
        "{}",
        m.distance()
    );
    assert!(h.state().state.history.is_empty());
    h.hover_at(centre);
    h.run_steps(1);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().tools.state().measure.measurements().len(), 0);
}

#[test]
fn a_narrow_toolbar_collapses_whole_groups_into_the_more_menu() {
    for width in [1280.0, 1600.0, 2600.0] {
        let (ui, sc) = editor();
        let mut h = harness_sized(ui, egui::vec2(width, 800.0));
        h.state_mut().state.selection.set([sc.a, sc.b]);
        h.run_steps(4);
        let layout = h.state().toolbar_layout().clone();
        let shown: Vec<ToolbarGroup> = layout.shown.iter().map(|(g, _)| *g).collect();
        // Core tools and Play never collapse at these widths.
        assert!(shown.contains(&ToolbarGroup::Tools), "{width}: {shown:?}");
        assert!(shown.contains(&ToolbarGroup::Play), "{width}: {shown:?}");
        // Nothing drawn is cut by the window edge: every group and the chevron end inside.
        let right = layout.available.max.x + 0.5;
        for (g, r) in &layout.shown {
            assert!(
                r.max.x <= right,
                "{width}: {g:?} ends at {} > {right}",
                r.max.x
            );
        }
        if width >= 2600.0 {
            assert!(layout.overflow.is_empty(), "{width}: {:?}", layout.overflow);
            assert!(layout.chevron.is_none());
            continue;
        }
        assert!(
            layout.overflow.contains(&ToolbarGroup::Build),
            "{width}: Build collapses first: {:?}",
            layout.overflow
        );
        let chevron = layout.chevron.expect("a More button");
        assert!(
            chevron.max.x <= right,
            "{width}: chevron at {}",
            chevron.max.x
        );

        // The menu lists the collapsed actions with label and shortcut, and runs them.
        h.get_by_label(MORE_LABEL).click();
        h.run_steps(3);
        h.get_by_label("Align A").click();
        h.run_steps(3);
        assert_eq!(
            h.state().tools.state().popover,
            Some(Popover::Align),
            "{width}: Align ran from the menu"
        );
    }
}

// --- Review fixes: Esc routing, stroke cancel, one gesture flag, guarded commits, stuck drag.

#[test]
fn esc_reaches_the_colour_popover_then_the_building_popover_then_the_tool() {
    let (ui, sc) = editor();
    let mut h = harness(ui);
    h.state_mut().state.selection.set([sc.b, sc.c]);
    h.state_mut().run_action("tool.measure");
    h.state_mut().run_action("build.align");
    h.run_steps(2);
    let centre = h.state().viewport.rect().center();
    h.hover_at(centre);
    h.run_steps(2);

    // The colour popover is topmost: Esc closes it and nothing else.
    {
        let ui = h.state_mut();
        assert!(ui.colour.open(&ui.state, "Color", Pos2::new(900.0, 200.0)));
    }
    h.run_steps(2);
    h.hover_at(centre);
    h.run_steps(1);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!h.state().colour.is_open(), "Esc closed the colour popover");
    assert_eq!(h.state().tools.state().popover, Some(Popover::Align));
    assert_eq!(h.state().tools.state().modal, Some(Modal::Measure));
    assert_eq!(h.state().state.selection.len(), 2, "the selection stays");

    // Then the Align popover, with the pointer over the viewport.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().tools.state().popover, None, "Esc closed Align");
    assert_eq!(h.state().tools.state().modal, Some(Modal::Measure));
    assert_eq!(h.state().state.selection.len(), 2);

    // Then the tool, and only then the selection.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().tools.state().modal, None, "Esc left Measure");
    assert_eq!(h.state().state.selection.len(), 2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(
        h.state().state.selection.is_empty(),
        "Esc cleared the selection"
    );
}

#[test]
fn popover_keys_follow_the_keymap_guards() {
    let (ui, sc) = editor();
    let mut h = harness(ui);
    let centre = h.state().viewport.rect().center();
    h.state_mut().state.selection.set([sc.b]);
    h.state_mut().run_action("select.similar");
    h.hover_at(centre);
    h.run_steps(2);
    // Right mouse held (flying): S is a fly key, not Select similar.
    h.event(Event::PointerButton {
        pos: centre,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(1);
    h.key_press(egui::Key::S);
    h.run_steps(1);
    h.event(Event::PointerButton {
        pos: centre,
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    assert_eq!(
        h.state().state.selection.len(),
        1,
        "no Select similar while flying"
    );
    assert_eq!(h.state().tools.state().popover, Some(Popover::Similar));
    h.key_press(egui::Key::S);
    h.run_steps(2);
    assert_eq!(h.state().state.selection.len(), 2, "S picked Size");

    // Array: Enter makes it (one step) through the keymap.
    h.state_mut().state.selection.set([sc.a]);
    h.state_mut().run_action("build.array");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().state.history.len(), 1);
    assert_eq!(h.state().tools.state().popover, None);
}

/// Presses over the baseplate with the brush on and drags a little; returns where.
fn start_stroke(h: &mut Harness<'static, EditorUi>, sc: &Scene) -> Pos2 {
    h.state_mut().state.selection.set([sc.c]);
    h.state_mut().run_action("tool.brush");
    h.run_steps(2);
    let at = screen(h, DVec3::new(-16.0, 0.0, 16.0));
    h.hover_at(at);
    h.run_steps(2);
    button(h, at, true, Modifiers::NONE);
    h.hover_at(at + egui::vec2(20.0, 0.0));
    h.run_steps(3);
    assert!(h.state().tools.gesture(), "a stroke is held");
    assert!(h.state().state.gesture_active());
    at + egui::vec2(20.0, 0.0)
}

#[test]
fn a_brush_stroke_cancelled_by_esc_tool_switch_or_focus_loss_never_commits() {
    for how in ["esc", "tool", "focus"] {
        let (ui, sc) = editor();
        let mut h = harness(ui);
        let before = snapshot(&h.state().state);
        let at = start_stroke(&mut h, &sc);
        match how {
            "esc" => h.key_press(egui::Key::Escape),
            "tool" => h.state_mut().run_action("tool.move"),
            _ => h.input_mut().focused = false,
        }
        h.run_steps(2);
        h.input_mut().focused = true;
        button(&mut h, at, false, Modifiers::NONE);
        assert!(!h.state().tools.gesture(), "{how}: the stroke ended");
        // Turning the brush on again does not commit the old stroke either.
        if h.state().tools.state().modal != Some(Modal::Brush) {
            h.state_mut().run_action("tool.brush");
        }
        h.run_steps(3);
        let s = &h.state().state;
        assert!(s.history.is_empty(), "{how}: no step");
        assert_eq!(snapshot(s), before, "{how}: the world is unchanged");
    }
}

#[test]
fn save_and_tool_edits_wait_for_a_colour_drag() {
    let (ui, sc) = editor();
    let mut h = harness(ui);
    h.state_mut().state.selection.set([sc.a, sc.b]);
    h.run_steps(2);
    h.get_by_label("Color swatch").click();
    h.run_steps(4);
    let centre = h.state().colour.popover().wheel_centre();
    h.hover_at(centre);
    h.run_steps(2);
    button(&mut h, centre, true, Modifiers::NONE);
    let to = centre + egui::vec2(30.0, -30.0);
    h.hover_at(to);
    h.run_steps(3);
    let mid = snapshot(&h.state().state);
    {
        let ui = h.state_mut();
        assert!(ui.state.gesture_active(), "the colour preview is a gesture");
        assert!(ui.state.save().is_err(), "preview colours are never saved");
        // Tool commits take the same refusal as EditorState::apply.
        assert!(
            !ui.actions.run("build.mirror", &mut ui.state),
            "mirror waits"
        );
        run_align(
            &mut ui.state,
            &ArrangeSettings::default(),
            Axis::Y,
            Edge::Max,
        );
        assert!(ui.state.history.is_empty(), "no step during the drag");
    }
    assert_eq!(snapshot(&h.state().state), mid, "nothing moved");
    // Ctrl+S during the drag does nothing either.
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::S);
    h.run_steps(1);
    button(&mut h, to, false, Modifiers::NONE);
    h.run_steps(2);
    let s = &h.state().state;
    assert!(!s.gesture_active());
    assert_eq!(s.history.len(), 1, "only the colour step");
}

#[test]
fn a_viewport_drag_still_ends_when_a_tool_takes_the_pointer_mid_drag() {
    let (ui, sc) = editor();
    let mut h = harness(ui);
    h.state_mut().state.tool = Tool::Move;
    h.state_mut().state.selection.set([sc.a]);
    h.run_steps(3);
    let pivot = screen(&h, DVec3::new(-8.0, 2.0, 0.0));
    let grab = pivot - egui::vec2(0.0, 54.0);
    h.hover_at(grab);
    h.run_steps(2);
    button(&mut h, grab, true, Modifiers::NONE);
    h.hover_at(grab - egui::vec2(0.0, 60.0));
    h.run_steps(2);
    assert!(h.state().viewport.is_dragging(), "the gizmo drag started");
    // A tool turns pointer capture on mid-drag (any path: a hover over the array handle, ...).
    h.state().tools.state().modal = Some(Modal::Measure);
    h.hover_at(grab - egui::vec2(0.0, 90.0));
    h.run_steps(2);
    button(&mut h, grab - egui::vec2(0.0, 90.0), false, Modifiers::NONE);
    h.run_steps(2);
    {
        let ui = h.state();
        assert!(!ui.viewport.is_dragging(), "the release ended the drag");
        assert!(!ui.state.gesture_active(), "keys are free again");
        assert_eq!(ui.state.history.len(), 1, "the drag is one step");
    }
    // A viewport key works again.
    let centre = h.state().viewport.rect().center();
    h.hover_at(centre);
    h.run_steps(1);
    h.key_press(egui::Key::E);
    h.run_steps(2);
    assert_eq!(h.state().state.tool, Tool::Rotate);
}
