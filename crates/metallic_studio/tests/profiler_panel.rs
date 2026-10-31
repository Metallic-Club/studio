//! #23: the Profiler panel shows recorded frames and pauses on the one clicked.
use eframe::egui::{self, Event, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_core::World;
use metallic_profiler::{Profiler, Subsystem};
use metallic_studio::debugger::ProfilerPanel;
use metallic_studio::debugger::profiler_view::ScriptOrder;

/// Records `n` frames on this thread: physics, scripts (one named script) and render scopes.
fn record(n: usize, script: u64) {
    Profiler::set_enabled(true);
    for i in 0..n {
        Profiler::begin_frame();
        {
            let _tick = Profiler::scope(Subsystem::Other, "tick");
            let _p = Profiler::scope(Subsystem::Physics, "physics.step");
            std::hint::black_box(i);
        }
        {
            let _s = Profiler::scope(Subsystem::Script, "scripts.step");
            Profiler::charge_script(script, 1_500_000);
            Profiler::charge_script(script + 1, 250_000);
        }
        Profiler::end_frame();
    }
    Profiler::set_enabled(false);
}

fn harness(panel: ProfilerPanel) -> Harness<'static, ProfilerPanel> {
    let mut themed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 300.0))
        .build_ui_state(
            move |ui, panel: &mut ProfilerPanel| {
                if !themed {
                    metallic_studio::theme::apply(
                        ui.ctx(),
                        metallic_studio::theme::ThemeId::Graphite,
                    );
                    themed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                panel.ui(ui, None);
            },
            panel,
        );
    h.run_steps(3);
    h
}

#[test]
fn the_panel_shows_subsystems_scopes_and_named_scripts_and_pauses_on_a_click() {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").expect("workspace");
    let script = world.create("Script", Some(ws)).expect("script");
    world
        .set(
            script,
            "Name",
            metallic_core::Value::String("Checkpoints".into()),
        )
        .expect("name");
    record(40, script.to_bits());

    let mut panel = ProfilerPanel::new();
    panel.learn_names_from(&world);
    let mut h = harness(panel);
    for label in ["Physics", "Scripts", "Workspace.Checkpoints", "Script 0x"] {
        assert!(
            h.query_all_by_label_contains(label).next().is_some(),
            "{label} not shown"
        );
    }
    assert!(h.query_all_by_label_contains("GPU").next().is_some());
    assert!(h.state().selected().is_none());

    // A click on the graph pauses on that frame.
    let graph = h
        .query_by_label_contains("Pause")
        .or_else(|| h.query_by_label_contains("Record"))
        .expect("the record button")
        .rect();
    let pos = egui::pos2(1000.0, graph.bottom() + 40.0);
    h.hover_at(pos);
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(2);
    }
    let selected = h.state().selected().expect("a frame was picked");
    assert!(selected < 40);
    assert!(!Profiler::is_enabled(), "the capture is paused");
    assert!(
        h.query_by_label_contains("Resume").is_none(),
        "not recording, so Record"
    );

    // Record resumes and forgets the pick.
    h.state_mut().set_recording(true);
    h.run_steps(2);
    assert!(Profiler::is_enabled());
    assert!(h.state().selected().is_none());
    h.state_mut().set_order(ScriptOrder::Name);
    h.state_mut().set_recording(false);
    h.run_steps(2);
    assert!(!Profiler::is_enabled());
}
