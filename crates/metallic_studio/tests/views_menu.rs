//! #23: View > Debug views switches the viewport's render view and the physics overlays.
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_render::DebugView;
use metallic_render::exposure::ExposureState;
use metallic_runtime::DebugDrawOptions;
use metallic_studio::debug_views::views_menu::{menu, meter_text};
use metallic_studio::viewport::Viewport;

type State = (Viewport, DebugDrawOptions);

fn harness() -> Harness<'static, State> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(500.0, 500.0))
        .build_ui_state(
            |ui, (viewport, physics): &mut State| {
                menu(ui, viewport, physics, false, None);
            },
            (Viewport::new(), DebugDrawOptions::default()),
        );
    h.run_steps(2);
    h
}

#[test]
fn the_menu_sets_the_render_view_and_the_physics_overlays() {
    let mut h = harness();
    h.get_by_label("Debug views").click();
    h.run_steps(2);
    assert!(h.query_by_label("Shows while playing.").is_some());
    h.get_by_label("Wireframe").click();
    h.run_steps(2);
    assert_eq!(h.state().0.debug_view(), DebugView::Wireframe);

    if h.query_by_label("Contacts").is_none() {
        h.get_by_label("Debug views").click();
        h.run_steps(2);
    }
    h.get_by_label("Contacts").click();
    h.run_steps(2);
    assert!(h.state().1.contacts);
    assert!(!h.state().1.shapes);
}

#[test]
fn the_light_meter_reads_ev_and_multiplier() {
    let e = ExposureState {
        multiplier: 1.25,
        ev100: 9.04,
        metered_log2: 0.0,
        target_ev100: 9.0,
    };
    assert_eq!(meter_text(&e), "EV100 9.0, exposure x1.25");
}
