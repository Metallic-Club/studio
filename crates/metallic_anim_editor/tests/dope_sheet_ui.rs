//! Drives the dope sheet and the demo with simulated mouse and keyboard input (egui_kittest, no
//! GPU: nothing is rendered, only egui's input and layout run).

use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use metallic_anim_editor::theme::{self, Theme};
use metallic_anim_editor::{DemoApp, DopeSheet, DopeSheetLayout, EditorState, KeyRef, wave_clip};

struct Ui {
    state: EditorState,
    rect: Rect,
    fonts: bool,
}

fn harness() -> Harness<'static, Ui> {
    let mut state = EditorState::new(wave_clip().unwrap());
    state.time_view.fit(0.0, 2.0, 1000.0 - 210.0);
    Harness::builder()
        .with_size(vec2(1000.0, 420.0))
        .build_ui_state(
            |ui, s: &mut Ui| {
                if !s.fonts {
                    // Install the shared theme and its bundled fonts, like the demo does.
                    theme::apply(ui.ctx());
                    s.fonts = true;
                }
                s.rect = ui.available_rect_before_wrap();
                DopeSheet::new(Theme::current()).show(ui, &mut s.state);
            },
            Ui {
                state,
                rect: Rect::NOTHING,
                fonts: false,
            },
        )
}

fn key_pos(h: &Harness<'_, Ui>, k: KeyRef) -> Pos2 {
    DopeSheetLayout::new(h.state().rect)
        .key_pos(&h.state().state, k)
        .unwrap()
}

#[test]
fn click_selects_and_drag_moves_a_key() {
    let mut h = harness();
    h.run();
    let k = KeyRef { track: 3, key: 1 };
    let before = h.state().state.key_time(k).unwrap();
    let p = key_pos(&h, k);
    h.hover_at(p);
    h.run();
    h.drag_at(p);
    h.run();
    let px_per_sec = h.state().state.time_view.px_per_sec;
    // Drag right by 6 frames' worth of pixels.
    h.hover_at(p + vec2(6.0 / 30.0 * px_per_sec, 0.0));
    h.run();
    h.drop_at(p + vec2(6.0 / 30.0 * px_per_sec, 0.0));
    h.run();
    let s = &h.state().state;
    assert_eq!(s.selection.len(), 1, "{:?}", s.selection);
    let sel = *s.selection.iter().next().unwrap();
    assert_eq!(sel.track, 3);
    let after = s.key_time(sel).unwrap();
    assert!((after - (before + 0.2)).abs() < 1e-3, "{before} -> {after}");
}

#[test]
fn box_select_picks_keys_inside() {
    let mut h = harness();
    h.run();
    let a = key_pos(&h, KeyRef { track: 0, key: 1 });
    let b = key_pos(&h, KeyRef { track: 1, key: 1 });
    let from = Pos2::new(a.x.min(b.x) - 8.0, a.y - 9.0);
    let to = Pos2::new(a.x.max(b.x) + 8.0, b.y + 9.0);
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    h.drop_at(to);
    h.run();
    let sel = &h.state().state.selection;
    assert!(sel.contains(&KeyRef { track: 0, key: 1 }), "{sel:?}");
    assert!(sel.contains(&KeyRef { track: 1, key: 1 }), "{sel:?}");
    assert!(sel.iter().all(|k| k.track <= 1));
}

#[test]
fn delete_key_removes_selection() {
    let mut h = harness();
    h.run();
    let k = KeyRef { track: 2, key: 1 };
    let count = h.state().state.key_count();
    let p = key_pos(&h, k);
    h.hover_at(p);
    h.run();
    h.drag_at(p);
    h.drop_at(p);
    h.run();
    assert!(h.state().state.selection.contains(&k));
    h.hover_at(p + vec2(0.0, 40.0));
    h.run();
    h.key_press(egui::Key::Delete);
    h.run();
    assert_eq!(h.state().state.key_count(), count - 1);
}

#[test]
fn demo_space_plays_and_k_toggles_record() {
    let mut app = DemoApp::new().unwrap();
    app.state.playing = false;
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut DemoApp| app.ui(ui), app);
    h.run_steps(2);
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert!(h.state().state.playing);
    h.key_press(egui::Key::K);
    h.run_steps(2);
    assert!(h.state().state.recording);
}
