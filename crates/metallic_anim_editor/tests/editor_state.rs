//! Editing operations on `EditorState` (no UI, no GPU).

use metallic_anim::{Channel, Clip, Curve, Interpolation, Keyframe, TangentMode, TrackTarget};
use metallic_anim_editor::theme::{Theme, contrast};
use metallic_anim_editor::{EditorState, KeyRef, wave_clip};

fn f32_track(name: &str, keys: &[(f32, f32)]) -> (TrackTarget, Channel) {
    let curve = Curve::from_keys(keys.iter().map(|&(t, v)| Keyframe::new(t, v)).collect()).unwrap();
    (
        TrackTarget::Property {
            path: name.into(),
            property: "Value".into(),
        },
        Channel::F32(curve),
    )
}

fn state() -> EditorState {
    let a = f32_track("A", &[(0.0, 0.0), (0.5, 1.0), (1.0, 2.0)]);
    let b = f32_track("B", &[(0.0, 5.0), (1.0, 6.0)]);
    let clip = Clip::new("Test", 2.0)
        .with_track(a.0, a.1)
        .with_track(b.0, b.1);
    EditorState::new(clip)
}

fn times(s: &EditorState, track: usize) -> Vec<f32> {
    s.clip.tracks[track].channel.key_times()
}

#[test]
fn move_snaps_to_frames_and_keeps_selection() {
    let mut s = state();
    s.selection.insert(KeyRef { track: 0, key: 1 });
    s.move_selected(0.1234); // 0.6234 s snaps to frame 19 at 30 fps
    let t = times(&s, 0);
    assert!((t[1] - 19.0 / 30.0).abs() < 1e-5, "{t:?}");
    assert_eq!(
        s.selection.iter().copied().collect::<Vec<_>>(),
        vec![KeyRef { track: 0, key: 1 }]
    );
}

#[test]
fn move_without_snap_is_exact() {
    let mut s = state();
    s.snap = false;
    s.selection.insert(KeyRef { track: 0, key: 1 });
    s.move_selected(0.1234);
    assert!((times(&s, 0)[1] - 0.6234).abs() < 1e-6);
}

#[test]
fn moving_past_a_neighbour_reorders_and_follows_the_key() {
    let mut s = state();
    s.selection.insert(KeyRef { track: 0, key: 0 });
    s.move_selected(0.8); // 0.0 -> 0.8, past the key at 0.5
    assert_eq!(times(&s, 0).len(), 3);
    let sel: Vec<KeyRef> = s.selection.iter().copied().collect();
    assert_eq!(sel, vec![KeyRef { track: 0, key: 1 }]);
    assert!((s.key_time(sel[0]).unwrap() - 0.8).abs() < 1e-5);
}

#[test]
fn moved_key_wins_a_collision() {
    let mut s = state();
    s.selection.insert(KeyRef { track: 0, key: 1 }); // value 1.0 at 0.5
    s.move_selected(0.5); // lands on the key at 1.0 (value 2.0)
    assert_eq!(times(&s, 0), vec![0.0, 1.0]);
    assert_eq!(
        s.clip.tracks[0].channel.sample(1.0),
        Some(metallic_anim::ChannelValue::F32(1.0))
    );
    assert_eq!(s.selection.len(), 1);
}

#[test]
fn scale_about_pivot_across_tracks() {
    let mut s = state();
    s.selection = [
        KeyRef { track: 0, key: 1 },
        KeyRef { track: 0, key: 2 },
        KeyRef { track: 1, key: 1 },
    ]
    .into();
    s.scale_selected(0.0, 2.0);
    assert_eq!(times(&s, 0), vec![0.0, 1.0, 2.0]);
    assert_eq!(times(&s, 1), vec![0.0, 2.0]);
    assert_eq!(s.selection.len(), 3);
    assert_eq!(s.selection_span(), Some((1.0, 2.0)));
}

#[test]
fn delete_and_navigate_keys() {
    let mut s = state();
    s.time = 0.2;
    assert_eq!(s.next_key_time(), Some(0.5));
    assert_eq!(s.prev_key_time(), Some(0.0));
    s.selection = [KeyRef { track: 0, key: 1 }, KeyRef { track: 1, key: 0 }].into();
    s.delete_selected();
    assert_eq!(times(&s, 0), vec![0.0, 1.0]);
    assert_eq!(times(&s, 1), vec![1.0]);
    assert!(s.selection.is_empty());
    assert_eq!(s.key_count(), 3);
}

#[test]
fn interpolation_and_tangent_mode_of_selection() {
    let mut s = state();
    s.selection = [KeyRef { track: 0, key: 0 }, KeyRef { track: 1, key: 0 }].into();
    assert_eq!(s.selected_interpolation(), Some(Interpolation::Bezier));
    s.set_selected_interpolation(Interpolation::Constant);
    assert_eq!(s.selected_interpolation(), Some(Interpolation::Constant));
    s.set_selected_tangent_mode(TangentMode::Flat);
    assert_eq!(s.selected_tangent_mode(), Some(TangentMode::Flat));
    s.selection.insert(KeyRef { track: 0, key: 1 });
    assert_eq!(s.selected_interpolation(), None, "mixed selection");
}

#[test]
fn key_at_playhead_shifts_selection_and_reuses_existing_key() {
    let mut s = state();
    s.selection.insert(KeyRef { track: 0, key: 2 });
    s.time = 0.3; // frame 9 at 30 fps
    assert_eq!(s.key_track_at_playhead(0), Some(1));
    assert_eq!(times(&s, 0), vec![0.0, 0.3, 0.5, 1.0]);
    // The old selection followed its key to index 3; the new key is selected too.
    assert!(s.selection.contains(&KeyRef { track: 0, key: 3 }));
    assert!(s.selection.contains(&KeyRef { track: 0, key: 1 }));
    // Keying again at the same time does not add a duplicate.
    assert_eq!(s.key_track_at_playhead(0), Some(1));
    assert_eq!(times(&s, 0).len(), 4);
}

#[test]
fn record_keys_is_explicit() {
    let mut s = state();
    s.time = 1.5;
    assert!(!s.recording, "Record keys starts off");
    assert!(!s.record_value(1, 0, 9.0));
    assert_eq!(times(&s, 1), vec![0.0, 1.0]);
    s.recording = true;
    assert!(s.record_value(1, 0, 9.0));
    assert_eq!(times(&s, 1), vec![0.0, 1.0, 1.5]);
    assert_eq!(
        s.clip.tracks[1].channel.sample(1.5),
        Some(metallic_anim::ChannelValue::F32(9.0))
    );
}

#[test]
fn playback_loops_and_stops() {
    let mut s = state();
    s.playing = true;
    s.tick(2.5);
    assert!((s.time - 0.5).abs() < 1e-5);
    s.looping = false;
    s.tick(5.0);
    assert_eq!(s.time, 2.0);
    assert!(!s.playing);
}

#[test]
fn demo_clips_are_valid() {
    let clip = wave_clip().unwrap();
    clip.validate().unwrap();
    assert!(clip.tracks.len() >= 8);
    metallic_anim_editor::walk_clip()
        .unwrap()
        .validate()
        .unwrap();
}

#[test]
fn theme_text_meets_contrast_on_its_surfaces() {
    let t = Theme::METALLIC;
    let surfaces = [
        ("deep", t.deep),
        ("base", t.base),
        ("panel", t.panel),
        ("raised", t.raised),
    ];
    for (name, text) in [
        ("text", t.text),
        ("text.2", t.text_dim),
        ("icon", t.silver),
        ("accent", t.accent),
    ] {
        for (sname, s) in surfaces {
            let r = contrast(text, s);
            assert!(r >= 4.5, "{name} on {sname}: {r:.2}");
        }
    }
    // text.3 is allowed on deep, base, panel and raised only (not hover/active).
    for (sname, s) in surfaces {
        assert!(contrast(t.text_meta, s) >= 4.0, "text.3 on {sname}");
    }
    assert!(
        contrast(t.on_accent, t.accent) >= 4.5,
        "dark text on the accent"
    );
    assert!(
        contrast(t.on_accent, t.error) >= 4.5,
        "dark text on the Record keys fill"
    );
    assert!(contrast(t.text, t.select) >= 4.5, "text on selected rows");
}
