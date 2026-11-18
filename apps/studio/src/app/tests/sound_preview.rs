//! #831: a double-click on a sound in the Assets panel opens its preview: its waveform on a timeline, Play and Pause,
//! a click on the timeline to move, and its facts over it.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

fn editor<'a>(harness: &'a mut Harness<'_, Test>) -> &'a mut EditorState {
    harness.state_mut().studio.editor_mut().unwrap()
}

/// A WAV of two seconds of a quiet tone at 8 kHz, mono.
fn wav() -> Vec<u8> {
    let (rate, frames) = (8000u32, 16_000u32);
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + frames * 2).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(frames * 2).to_le_bytes());
    for frame in 0..frames {
        let sample = if frame % 20 < 10 { 4000i16 } else { -4000 };
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[test]
fn a_sound_opens_in_a_preview_that_plays_and_seeks() {
    let mut harness = editing(settings_file("sound-preview", None), "Sounds Game");
    harness.set_size(egui::vec2(1400.0, 1000.0));
    let editor_state = editor(&mut harness);
    if !editor_state.project.contains("Assets/beep.wav") {
        editor_state.project.create_member("Assets/beep.wav", wav()).unwrap();
    }
    let id = editor_state.project.assets().id("Assets/beep.wav").unwrap();
    editor_state.browser.open_folder("Assets");
    editor_state.browser.query = "beep".into();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Tab, "Assets").click();
    settle(&mut harness);

    // A double-click, timed as `double_click`'s are (#662).
    let time = harness.ctx.input(|input| input.time) + 1.0;
    harness.input_mut().time = Some(time);
    harness.get_by_role_and_label(Role::GridCell, "beep").click();
    harness.step();
    harness.input_mut().time = Some(time + 0.05);
    harness.get_by_role_and_label(Role::GridCell, "beep").click();
    settle(&mut harness);
    harness.input_mut().time = None;

    harness.get_by_role_and_label(Role::Tab, "beep.wav");
    harness.get_by_label("Assets/beep.wav");
    harness.get_by_label_contains("0:02.00  \u{2022}  8000 Hz  \u{2022}  Mono  \u{2022}  WAV");
    harness.get_by_label("No sound plays it.");

    // Play, then Pause.
    harness.get_by_role_and_label(Role::Button, "Play the sound").click();
    settle(&mut harness);
    assert!(editor(&mut harness).sounds.is_playing(id));
    harness.get_by_role_and_label(Role::Button, "Pause the sound").click();
    settle(&mut harness);
    assert!(!editor(&mut harness).sounds.is_playing(id));

    // A click in the middle of the timeline moves there.
    let timeline = harness
        .get_by_role_and_label(Role::Slider, "Time 0:00.00 of 0:02.00")
        .rect();
    left_click(
        &mut harness,
        egui::Rect::from_center_size(timeline.center(), egui::Vec2::ZERO),
    );
    let place = editor(&mut harness).sounds.place(id);
    assert!((place - 1.0).abs() < 0.05, "{place}");
    harness.get_by_role_and_label(Role::Slider, "Time 0:01.00 of 0:02.00");
}
