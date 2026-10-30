//! Mixer behaviour through the headless engine: no audio device needed.

use metallic_audio::{
    AudioBackend, AudioEngine, AudioError, BLOCK, CompressorParams, DistortionParams, EchoParams,
    Effect, Emitter, EngineConfig, EqualizerParams, FilterParams, GroupId, Listener, MixRenderer,
    ReverbParams, RollOffMode, SPEED_OF_SOUND_MPS, SoundAsset, SoundEvent, SoundProperties,
    SpatialSettings, StreamSource, Vec3,
};
use std::f32::consts::TAU;

const RATE: u32 = 48_000;

fn engine() -> AudioEngine {
    AudioEngine::headless(EngineConfig::default()).unwrap()
}

fn engine_with(max_voices: usize) -> AudioEngine {
    AudioEngine::headless(EngineConfig {
        max_voices,
        ..EngineConfig::default()
    })
    .unwrap()
}

fn dc(engine: &mut AudioEngine, value: f32, frames: usize) -> metallic_audio::AssetId {
    engine.insert_asset(SoundAsset::from_mono(vec![value; frames], RATE).unwrap())
}

fn sine(engine: &mut AudioEngine, freq: f32, amp: f32, frames: usize) -> metallic_audio::AssetId {
    let samples = (0..frames)
        .map(|i| amp * (TAU * freq * i as f32 / RATE as f32).sin())
        .collect();
    engine.insert_asset(SoundAsset::from_mono(samples, RATE).unwrap())
}

fn playing(volume: f32) -> SoundProperties {
    SoundProperties {
        volume,
        playing: true,
        looped: true,
        ..SoundProperties::default()
    }
}

fn render(engine: &mut AudioEngine, frames: usize) -> Vec<[f32; 2]> {
    let mut out = vec![0.0; frames * 2];
    engine.render(&mut out).unwrap();
    out.as_chunks::<2>().0.to_vec()
}

fn rms(frames: &[[f32; 2]], ch: usize) -> f32 {
    (frames.iter().map(|f| f[ch] * f[ch]).sum::<f32>() / frames.len() as f32).sqrt()
}

fn assert_all(frames: &[[f32; 2]], expected: [f32; 2], tol: f32) {
    for (i, f) in frames.iter().enumerate() {
        assert!(
            (f[0] - expected[0]).abs() <= tol && (f[1] - expected[1]).abs() <= tol,
            "frame {i}: {f:?} != {expected:?}"
        );
    }
}

#[test]
fn headless_needs_no_device_and_starts_silent() {
    let mut e = engine();
    assert!(e.is_headless());
    assert_eq!(e.backend_name(), "headless");
    assert_eq!(e.sample_rate(), RATE);
    let out = render(&mut e, 1_000);
    assert_all(&out, [0.0, 0.0], 0.0);
    assert!(e.stats().frames_rendered >= 1_000);
    assert_eq!(e.backend_error(), None);
}

#[test]
fn invalid_configs_are_rejected() {
    let bad = [
        EngineConfig {
            sample_rate: 100,
            ..EngineConfig::default()
        },
        EngineConfig {
            max_voices: 0,
            ..EngineConfig::default()
        },
        EngineConfig {
            max_groups: 3,
            ..EngineConfig::default()
        },
    ];
    for config in bad {
        assert!(matches!(
            AudioEngine::headless(config),
            Err(AudioError::InvalidParameter(_))
        ));
    }
}

#[test]
fn group_volumes_multiply_down_to_master() {
    let mut e = engine();
    let a = dc(&mut e, 0.5, 4_800);
    e.create_sound(
        a,
        SoundProperties {
            sound_group: GroupId::MUSIC,
            ..playing(0.5)
        },
    )
    .unwrap();
    e.set_group_volume(GroupId::MUSIC, 0.5).unwrap();
    e.set_group_volume(GroupId::MASTER, 0.8).unwrap();
    // 0.5 sample * 0.5 sound * 0.5 Music * 0.8 Master, unpanned on both channels.
    assert_all(&render(&mut e, 1_000), [0.1, 0.1], 1e-6);
    assert!((e.effective_group_volume(GroupId::MUSIC).unwrap() - 0.4).abs() < 1e-6);

    // A user group nested under Music.
    let quiet = e.create_group(GroupId::MUSIC).unwrap();
    e.set_group_volume(quiet, 0.5).unwrap();
    let b = dc(&mut e, 0.25, 4_800);
    let s = e
        .create_sound(
            b,
            SoundProperties {
                sound_group: quiet,
                ..playing(1.0)
            },
        )
        .unwrap();
    render(&mut e, BLOCK);
    // 0.1 from before + 0.25 * 1 * 0.5 * 0.5 * 0.8 = 0.05.
    assert_all(&render(&mut e, 500), [0.15, 0.15], 1e-6);
    assert_eq!(e.group_parent(quiet).unwrap(), Some(GroupId::MUSIC));
    assert!((e.effective_group_volume(quiet).unwrap() - 0.2).abs() < 1e-6);

    // Removing the group moves its sound to the parent (Music).
    e.remove_group(quiet).unwrap();
    assert_eq!(e.properties(s).unwrap().sound_group, GroupId::MUSIC);
    render(&mut e, BLOCK);
    // 0.25 * 0.5 * 0.8 = 0.1, plus 0.1.
    assert_all(&render(&mut e, 500), [0.2, 0.2], 1e-6);
    assert!(matches!(
        e.set_group_volume(quiet, 1.0),
        Err(AudioError::UnknownGroup(_))
    ));
}

#[test]
fn group_volume_changes_ramp_over_one_block() {
    let mut e = engine();
    let a = dc(&mut e, 0.5, 4_800);
    e.create_sound(a, playing(1.0)).unwrap();
    render(&mut e, 4 * BLOCK);
    e.set_group_volume(GroupId::SFX, 0.0).unwrap();
    let out = render(&mut e, 2 * BLOCK);
    for pair in out[..BLOCK].windows(2) {
        assert!(pair[1][0] < pair[0][0], "volume ramp must fall steadily");
    }
    assert!(out[0][0] > 0.45 && out[0][0] < 0.5);
    assert_all(&out[BLOCK - 1..], [0.0, 0.0], 1e-6);
}

#[test]
fn built_in_groups_cannot_be_removed() {
    let mut e = engine();
    for g in GroupId::BUILT_IN {
        assert!(e.remove_group(g).is_err());
        assert_eq!(e.group_volume(g).unwrap(), 1.0);
    }
    assert_eq!(e.group_parent(GroupId::MASTER).unwrap(), None);
    assert_eq!(
        e.group_parent(GroupId::VOICE).unwrap(),
        Some(GroupId::MASTER)
    );
}

#[test]
fn voice_limit_keeps_the_highest_priority() {
    let mut e = engine_with(2);
    let loud = dc(&mut e, 0.1, 4_800);
    let mid = dc(&mut e, 0.01, 4_800);
    let quiet = dc(&mut e, 0.001, 4_800);
    let a = e
        .create_sound(
            loud,
            SoundProperties {
                priority: 1,
                ..playing(1.0)
            },
        )
        .unwrap();
    e.create_sound(
        mid,
        SoundProperties {
            priority: 2,
            ..playing(1.0)
        },
    )
    .unwrap();
    e.create_sound(
        quiet,
        SoundProperties {
            priority: 3,
            ..playing(1.0)
        },
    )
    .unwrap();
    // Priority beats loudness: the 0.1 sound has the lowest priority and is the one left out.
    assert_all(&render(&mut e, 4 * BLOCK), [0.011, 0.011], 1e-6);
    let stats = e.stats();
    assert_eq!((stats.real_voices, stats.virtual_voices), (2, 1));
    // The virtual voice keeps its clock.
    assert!(e.time_position(a).unwrap() > 0.0);

    // Raise its priority: it takes a voice back (fading in over one block) and the 0.01 sound,
    // now the lowest priority, becomes virtual.
    e.set_priority(a, 10).unwrap();
    render(&mut e, 2 * BLOCK);
    assert_all(&render(&mut e, BLOCK), [0.101, 0.101], 1e-6);
}

#[test]
fn equal_priority_prefers_the_louder_sound() {
    let mut e = engine_with(1);
    // Loudness here is the sound's effective volume (volume x groups x distance), not the
    // waveform: 0.2 x 0.05 = 0.01 against 0.2 x 1.
    let a = dc(&mut e, 0.2, 4_800);
    e.create_sound(a, playing(0.05)).unwrap();
    let l = e.create_sound(a, playing(1.0)).unwrap();
    assert_all(&render(&mut e, 2 * BLOCK), [0.2, 0.2], 1e-6);
    // When the loud one stops the quiet one gets the voice.
    e.stop(l).unwrap();
    render(&mut e, 2 * BLOCK);
    assert_all(&render(&mut e, BLOCK), [0.01, 0.01], 1e-6);
}

#[test]
fn silent_sounds_do_not_take_voices() {
    let mut e = engine_with(1);
    let far = dc(&mut e, 0.3, 4_800);
    let near = dc(&mut e, 0.05, 4_800);
    // High priority but beyond RollOffMaxDistance: inaudible, so it must not steal the voice.
    e.create_sound(
        far,
        SoundProperties {
            priority: 100,
            emitter: Some(Emitter::at(Vec3::new(0.0, 0.0, -500.0))),
            roll_off_max_distance: 100.0,
            ..playing(1.0)
        },
    )
    .unwrap();
    e.create_sound(near, playing(1.0)).unwrap();
    assert_all(&render(&mut e, 2 * BLOCK), [0.05, 0.05], 1e-6);
}

#[test]
fn looping_wraps_time_position_and_reports_did_loop() {
    let mut e = engine();
    let a = dc(&mut e, 0.2, 1_000);
    let s = e.create_sound(a, playing(1.0)).unwrap();
    assert_eq!(e.time_length(s).unwrap(), Some(1_000.0 / RATE as f64));
    let frames = 40 * BLOCK; // 2,560 frames
    let out = render(&mut e, frames);
    assert_all(&out, [0.2, 0.2], 1e-6);
    let expected = (frames % 1_000) as f64 / RATE as f64;
    assert!((e.time_position(s).unwrap() - expected).abs() < 1e-9);
    e.update(0.0);
    assert_eq!(
        e.drain_events(),
        vec![SoundEvent::DidLoop { sound: s, count: 2 }]
    );
    assert!(e.is_playing(s).unwrap());
}

#[test]
fn a_sound_ends_and_reports_it() {
    let mut e = engine();
    let a = dc(&mut e, 0.2, 1_000);
    let s = e
        .create_sound(
            a,
            SoundProperties {
                looped: false,
                ..playing(1.0)
            },
        )
        .unwrap();
    assert!(e.is_playing(s).unwrap());
    let out = render(&mut e, 17 * BLOCK);
    assert_all(&out[..1_000], [0.2, 0.2], 1e-6);
    assert_all(&out[1_000..], [0.0, 0.0], 0.0);
    e.update(0.0);
    assert_eq!(e.drain_events(), vec![SoundEvent::Ended(s)]);
    assert!(!e.is_playing(s).unwrap());
    assert_eq!(e.time_position(s).unwrap(), 0.0);
    // Play() starts it again.
    e.play(s).unwrap();
    assert!(e.is_playing(s).unwrap());
    assert_all(&render(&mut e, BLOCK), [0.2, 0.2], 1e-6);
}

fn ramp_asset(e: &mut AudioEngine, frames: usize, rate: u32, step: f32) -> metallic_audio::AssetId {
    e.insert_asset(
        SoundAsset::from_mono((0..frames).map(|i| i as f32 * step).collect(), rate).unwrap(),
    )
}

#[test]
fn time_position_seeks_and_play_starts_there() {
    let mut e = engine();
    let a = ramp_asset(&mut e, 48_000, RATE, 1e-5);
    let s = e
        .create_sound(
            a,
            SoundProperties {
                volume: 1.0,
                ..SoundProperties::default()
            },
        )
        .unwrap();
    e.set_time_position(s, 0.25).unwrap();
    assert_eq!(e.time_position(s).unwrap(), 0.25);
    e.play(s).unwrap();
    let out = render(&mut e, BLOCK);
    for (n, f) in out.iter().enumerate() {
        let expected = (12_000 + n) as f32 * 1e-5;
        assert!(
            (f[0] - expected).abs() < 1e-6,
            "{n}: {} vs {expected}",
            f[0]
        );
    }
    assert!((e.time_position(s).unwrap() - (12_000 + BLOCK) as f64 / RATE as f64).abs() < 1e-9);
    // Seeking past the end clamps to the length.
    e.set_time_position(s, 99.0).unwrap();
    assert_eq!(e.time_position(s).unwrap(), 1.0);
    assert!(e.set_time_position(s, f64::NAN).is_err());
}

#[test]
fn stop_resets_and_pause_resume_keep_position() {
    let mut e = engine();
    let a = ramp_asset(&mut e, 48_000, RATE, 1e-5);
    let s = e
        .create_sound(
            a,
            SoundProperties {
                volume: 1.0,
                playing: true,
                ..SoundProperties::default()
            },
        )
        .unwrap();
    render(&mut e, 10 * BLOCK);
    e.pause(s).unwrap();
    assert!(e.is_paused(s).unwrap() && !e.is_playing(s).unwrap());
    let fade = render(&mut e, 3 * BLOCK);
    // The pause fades out over one block, then silence.
    assert!(fade[0][0] > 0.0 && fade[BLOCK - 1][0].abs() < 1e-3);
    assert_all(&fade[BLOCK..], [0.0, 0.0], 0.0);
    let paused_at = (10 * BLOCK) as f64 / RATE as f64;
    assert!((e.time_position(s).unwrap() - paused_at).abs() < 1e-9);

    e.set_playing(s, true).unwrap(); // Playing = true resumes
    assert!(e.is_playing(s).unwrap());
    let resumed = render(&mut e, BLOCK);
    // Fades in from the paused position.
    let last = resumed[BLOCK - 1][0];
    let expected = (10 * BLOCK + BLOCK - 1) as f32 * 1e-5;
    assert!((last - expected).abs() < 1e-6, "{last} vs {expected}");

    e.stop(s).unwrap();
    assert!(!e.is_playing(s).unwrap() && !e.is_paused(s).unwrap());
    assert_eq!(e.time_position(s).unwrap(), 0.0);
    render(&mut e, 2 * BLOCK);
    assert_eq!(e.time_position(s).unwrap(), 0.0);
    e.play(s).unwrap();
    let restarted = render(&mut e, 2);
    assert!(restarted[0][0].abs() < 1e-7 && (restarted[1][0] - 1e-5).abs() < 1e-7);
}

#[test]
fn playback_speed_and_resampling() {
    let mut e = engine();
    // A 24 kHz ramp played at 48 kHz moves half a source frame per output frame.
    let slow = ramp_asset(&mut e, 24_000, 24_000, 1e-3);
    e.create_sound(slow, playing(1.0)).unwrap();
    let out = render(&mut e, BLOCK);
    for (n, f) in out.iter().enumerate() {
        assert!((f[0] - n as f32 * 0.5e-3).abs() < 1e-6, "{n}");
    }

    let mut e = engine();
    let ramp = ramp_asset(&mut e, 48_000, RATE, 1e-4);
    let s = e
        .create_sound(
            ramp,
            SoundProperties {
                playback_speed: 2.0,
                ..playing(1.0)
            },
        )
        .unwrap();
    let out = render(&mut e, BLOCK);
    for (n, f) in out.iter().enumerate() {
        assert!((f[0] - n as f32 * 2e-4).abs() < 1e-6, "{n}");
    }
    assert!((e.time_position(s).unwrap() - (2 * BLOCK) as f64 / RATE as f64).abs() < 1e-9);
    // Speed 0 holds the position.
    e.set_playback_speed(s, 0.0).unwrap();
    render(&mut e, 2 * BLOCK);
    let t = e.time_position(s).unwrap();
    render(&mut e, 2 * BLOCK);
    assert_eq!(e.time_position(s).unwrap(), t);
    assert!(e.set_playback_speed(s, f32::NAN).is_err());
}

#[test]
fn spatial_sound_pans_and_attenuates() {
    let mut e = engine();
    let a = dc(&mut e, 0.5, 4_800);
    let s = e
        .create_sound(
            a,
            SoundProperties {
                emitter: Some(Emitter::at(Vec3::new(20.0, 0.0, 0.0))),
                roll_off_min_distance: 10.0,
                ..playing(1.0)
            },
        )
        .unwrap();
    // Inverse roll-off at twice the min distance halves it; hard right.
    assert_all(&render(&mut e, 2 * BLOCK), [0.0, 0.25], 1e-6);

    // Turn the listener around: now the sound is on the left.
    e.set_listener(Listener {
        forward: Vec3::Z,
        ..Listener::default()
    });
    render(&mut e, BLOCK);
    assert_all(&render(&mut e, BLOCK), [0.25, 0.0], 1e-6);

    // Linear roll-off, centred in front.
    e.set_listener(Listener::default());
    e.set_emitter(s, Some(Emitter::at(Vec3::new(0.0, 0.0, -60.0))))
        .unwrap();
    e.set_roll_off(s, RollOffMode::Linear, 10.0, 110.0).unwrap();
    render(&mut e, BLOCK);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    assert_all(&render(&mut e, BLOCK), [0.5 * 0.5 * h, 0.5 * 0.5 * h], 1e-6);

    // Non-positional again: full volume, both channels.
    e.set_emitter(s, None).unwrap();
    render(&mut e, BLOCK);
    assert_all(&render(&mut e, BLOCK), [0.5, 0.5], 1e-6);
    assert!(
        e.set_emitter(s, Some(Emitter::at(Vec3::new(f32::NAN, 0.0, 0.0))))
            .is_err()
    );
}

#[test]
fn occlusion_hook_muffles_positional_sounds() {
    let mut e = engine();
    let a = dc(&mut e, 0.5, 48_000);
    e.create_sound(
        a,
        SoundProperties {
            emitter: Some(Emitter::at(Vec3::new(0.0, 0.0, -5.0))),
            ..playing(1.0)
        },
    )
    .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counter = std::sync::Arc::clone(&calls);
    e.set_occlusion_hook(move |listener, emitter| {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(listener, Vec3::ZERO);
        assert_eq!(emitter, Vec3::new(0.0, 0.0, -5.0));
        1.0
    });
    e.update(0.0);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let settings = SpatialSettings::default();
    // DC passes the occlusion low-pass, so after it settles only the occlusion gain remains.
    let out = render(&mut e, 40 * BLOCK);
    let expected = 0.5 * settings.occlusion_gain * h;
    assert_all(&out[20 * BLOCK..], [expected, expected], 1e-4);

    e.clear_occlusion_hook();
    render(&mut e, 20 * BLOCK);
    assert_all(&render(&mut e, BLOCK), [0.5 * h, 0.5 * h], 1e-4);
}

#[test]
fn per_frame_occlusion_query_can_borrow_game_state() {
    // Stands in for the physics world the runtime borrows each frame.
    let walls = [Vec3::new(0.0, 0.0, -3.0)];
    let mut e = engine();
    let a = dc(&mut e, 0.5, 48_000);
    let s = e
        .create_sound(
            a,
            SoundProperties {
                emitter: Some(Emitter::at(Vec3::new(0.0, 0.0, -5.0))),
                ..playing(1.0)
            },
        )
        .unwrap();
    let mut queries = 0;
    e.update_with_occlusion(0.0, &mut |from, to| {
        queries += 1;
        let blocked = walls.iter().any(|w| w.z < from.z && w.z > to.z);
        if blocked { 1.0 } else { 0.0 }
    });
    assert_eq!(queries, 1);
    render(&mut e, 40 * BLOCK);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let expected = 0.5 * SpatialSettings::default().occlusion_gain * h;
    assert_all(&render(&mut e, BLOCK), [expected, expected], 1e-4);
    // Stopped sounds are not queried.
    e.stop(s).unwrap();
    e.update_with_occlusion(0.0, &mut |_, _| panic!("no playing positional sounds"));
}

#[test]
fn occlusion_low_pass_removes_high_frequencies() {
    let mut e = engine();
    let a = sine(&mut e, 8_000.0, 0.5, 48_000);
    e.create_sound(
        a,
        SoundProperties {
            emitter: Some(Emitter::at(Vec3::new(0.0, 0.0, -5.0))),
            ..playing(1.0)
        },
    )
    .unwrap();
    let open = rms(&render(&mut e, 4_800), 0);
    e.set_occlusion_hook(|_, _| 1.0);
    e.update(0.0);
    render(&mut e, 960);
    let muffled = rms(&render(&mut e, 4_800), 0);
    // 0.35 gain alone would be -9 dB; the 1.2 kHz low-pass adds well over 10 dB at 8 kHz.
    assert!(
        muffled < open * 0.35 * 0.3,
        "open {open}, muffled {muffled}"
    );
}

#[test]
fn doppler_speeds_up_an_approaching_source() {
    let mut e = engine();
    let a = dc(&mut e, 0.5, 96_000);
    let c = SPEED_OF_SOUND_MPS * SpatialSettings::default().distance_factor;
    let s = e
        .create_sound(
            a,
            SoundProperties {
                emitter: Some(Emitter {
                    position: Vec3::new(0.0, 0.0, -50.0),
                    velocity: Vec3::new(0.0, 0.0, 0.5 * c),
                }),
                roll_off_max_distance: 1e6,
                ..playing(1.0)
            },
        )
        .unwrap();
    render(&mut e, 75 * BLOCK); // 0.1 s
    // Pitch factor c / (c - 0.5c) = 2: twice as much of the file is consumed.
    assert!((e.time_position(s).unwrap() - 0.2).abs() < 1e-6);
}

#[test]
fn output_is_identical_whatever_the_render_call_sizes() {
    fn scene(e: &mut AudioEngine) {
        let a = sine(e, 440.0, 0.3, 20_000);
        let b = sine(e, 1_234.0, 0.3, 7_000);
        e.create_sound(
            a,
            SoundProperties {
                emitter: Some(Emitter::at(Vec3::new(15.0, 2.0, -3.0))),
                ..playing(1.0)
            },
        )
        .unwrap();
        e.create_sound(
            b,
            SoundProperties {
                playback_speed: 1.37,
                ..playing(0.7)
            },
        )
        .unwrap();
        e.add_effect(GroupId::MASTER, Effect::Reverb(ReverbParams::default()), 0)
            .unwrap();
    }
    let mut one = engine();
    let mut many = engine();
    scene(&mut one);
    scene(&mut many);
    let whole = render(&mut one, 9_600);
    let mut pieces = Vec::new();
    for size in [1, 7, 63, 64, 65, 333, 1_000, 2_048].iter().cycle() {
        let left = 9_600 - pieces.len();
        if left == 0 {
            break;
        }
        pieces.extend(render(&mut many, (*size).min(left)));
    }
    assert_eq!(whole, pieces);
    assert!(rms(&whole, 0) > 0.01);
}

#[test]
fn handles_are_generational_and_limits_hold() {
    let mut e = AudioEngine::headless(EngineConfig {
        max_sounds: 2,
        ..EngineConfig::default()
    })
    .unwrap();
    let a = dc(&mut e, 0.1, 100);
    let s1 = e.create_sound(a, SoundProperties::default()).unwrap();
    let _s2 = e.create_sound(a, SoundProperties::default()).unwrap();
    assert!(matches!(
        e.create_sound(a, SoundProperties::default()),
        Err(AudioError::Limit("max_sounds"))
    ));
    e.remove_sound(s1).unwrap();
    assert!(matches!(e.play(s1), Err(AudioError::UnknownSound(_))));
    let s3 = e.create_sound(a, SoundProperties::default()).unwrap();
    assert_ne!(s1, s3);
    assert!(e.is_playing(s1).is_err());
    assert_eq!(e.sound_count(), 2);
    e.remove_asset(a).unwrap();
    assert!(matches!(
        e.create_sound(a, SoundProperties::default()),
        Err(AudioError::UnknownAsset(_))
    ));
}

#[test]
fn properties_are_clamped() {
    let mut e = engine();
    let a = dc(&mut e, 0.1, 100);
    let s = e
        .create_sound(
            a,
            SoundProperties {
                volume: 50.0,
                playback_speed: -1.0,
                ..SoundProperties::default()
            },
        )
        .unwrap();
    let p = e.properties(s).unwrap();
    assert_eq!((p.volume, p.playback_speed), (10.0, 0.0));
    e.set_volume(s, -3.0).unwrap();
    assert_eq!(e.properties(s).unwrap().volume, 0.0);
    assert!(e.set_volume(s, f32::INFINITY).is_err());
}

// ---------------------------------------------------------------------------------------------
// Effects

fn effect_scene(effect: Option<Effect>, freq: f32, amp: f32, frames: usize) -> Vec<[f32; 2]> {
    let mut e = engine();
    let a = sine(&mut e, freq, amp, 48_000);
    e.create_sound(a, playing(1.0)).unwrap();
    if let Some(effect) = effect {
        e.add_effect(GroupId::SFX, effect, 0).unwrap();
    }
    render(&mut e, frames)
}

#[test]
fn flat_equalizer_is_transparent() {
    let dry = effect_scene(None, 1_000.0, 0.5, 4_800);
    let eq = effect_scene(
        Some(Effect::Equalizer(EqualizerParams::default())),
        1_000.0,
        0.5,
        4_800,
    );
    for (a, b) in dry.iter().zip(eq.iter()) {
        assert!((a[0] - b[0]).abs() < 1e-5);
    }
}

#[test]
fn equalizer_cuts_the_chosen_band() {
    let cut_highs = Effect::Equalizer(EqualizerParams {
        high_gain: -40.0,
        ..EqualizerParams::default()
    });
    let low = effect_scene(Some(cut_highs), 100.0, 0.5, 9_600);
    let high = effect_scene(Some(cut_highs), 15_000.0, 0.5, 9_600);
    assert!(rms(&low[4_800..], 0) > 0.3);
    assert!(rms(&high[4_800..], 0) < 0.02);
}

#[test]
fn low_pass_and_high_pass() {
    let lp = Effect::LowPass(FilterParams {
        cutoff_hz: 1_000.0,
        ..FilterParams::default()
    });
    assert!(rms(&effect_scene(Some(lp), 100.0, 0.5, 9_600)[4_800..], 0) > 0.34);
    assert!(rms(&effect_scene(Some(lp), 10_000.0, 0.5, 9_600)[4_800..], 0) < 0.01);
    let hp = Effect::HighPass(FilterParams {
        cutoff_hz: 1_000.0,
        ..FilterParams::default()
    });
    assert!(rms(&effect_scene(Some(hp), 100.0, 0.5, 9_600)[4_800..], 0) < 0.01);
    assert!(rms(&effect_scene(Some(hp), 10_000.0, 0.5, 9_600)[4_800..], 0) > 0.34);
}

#[test]
fn compressor_reduces_loud_signals() {
    let comp = Effect::Compressor(CompressorParams {
        threshold: -20.0,
        ratio: 10.0,
        attack: 0.001,
        release: 0.05,
        gain_makeup: 0.0,
    });
    let dry = rms(&effect_scene(None, 200.0, 0.9, 9_600)[4_800..], 0);
    let wet = rms(&effect_scene(Some(comp), 200.0, 0.9, 9_600)[4_800..], 0);
    // The peak (-0.9 dB) is ~19 dB over the threshold; ratio 10 removes ~17 dB.
    assert!(wet < dry * 0.25, "dry {dry} wet {wet}");
    assert!(wet > dry * 0.05, "dry {dry} wet {wet}");
}

#[test]
fn distortion_stays_bounded_and_adds_harmonics() {
    let dist = Effect::Distortion(DistortionParams { level: 1.0 });
    let out = effect_scene(Some(dist), 200.0, 0.3, 4_800);
    let dry = effect_scene(None, 200.0, 0.3, 4_800);
    assert!(out.iter().all(|f| f[0].abs() <= 1.0));
    // Soft clipping squares the wave up: more energy at the same peak.
    assert!(rms(&out, 0) > rms(&dry, 0) * 2.0);
}

#[test]
fn reverb_leaves_a_tail_and_echo_repeats() {
    let tail = |effect: Option<Effect>| {
        let mut e = engine();
        let a = dc(&mut e, 0.5, 480); // 10 ms burst
        e.create_sound(
            a,
            SoundProperties {
                looped: false,
                ..playing(1.0)
            },
        )
        .unwrap();
        if let Some(effect) = effect {
            e.add_effect(GroupId::MASTER, effect, 0).unwrap();
        }
        render(&mut e, 24_000)
    };
    let dry = tail(None);
    assert_all(&dry[480..], [0.0, 0.0], 0.0);
    let wet = tail(Some(Effect::Reverb(ReverbParams {
        decay_time: 1.0,
        ..ReverbParams::default()
    })));
    assert!(rms(&wet[4_800..9_600], 0) > 1e-3, "reverb tail missing");
    assert!(rms(&wet[4_800..9_600], 1) > 1e-3);

    let echo = tail(Some(Effect::Echo(EchoParams {
        delay: 0.1,
        feedback: 0.5,
        ..EchoParams::default()
    })));
    // First repeat 100 ms later at full wet level, second at half.
    assert!((echo[4_800 + 100][0] - 0.5).abs() < 1e-6);
    assert!((echo[9_600 + 100][0] - 0.25).abs() < 1e-6);
    assert_eq!(echo[3_000][0], 0.0);
}

#[test]
fn effects_can_be_disabled_changed_and_removed() {
    let mut e = engine();
    let a = sine(&mut e, 10_000.0, 0.5, 48_000);
    e.create_sound(a, playing(1.0)).unwrap();
    let lp = e
        .add_effect(
            GroupId::SFX,
            Effect::LowPass(FilterParams {
                cutoff_hz: 500.0,
                q: 0.7,
            }),
            0,
        )
        .unwrap();
    render(&mut e, 4_800);
    assert!(rms(&render(&mut e, 4_800), 0) < 0.01);
    e.set_effect_enabled(lp, false).unwrap();
    assert!(rms(&render(&mut e, 4_800), 0) > 0.34);
    e.set_effect_enabled(lp, true).unwrap();
    // A different kind rebuilds the processor.
    e.set_effect(
        lp,
        Effect::HighPass(FilterParams {
            cutoff_hz: 500.0,
            q: 0.7,
        }),
    )
    .unwrap();
    render(&mut e, 4_800);
    assert!(rms(&render(&mut e, 4_800), 0) > 0.34);
    e.set_effect(
        lp,
        Effect::HighPass(FilterParams {
            cutoff_hz: 18_000.0,
            q: 0.7,
        }),
    )
    .unwrap();
    render(&mut e, 4_800);
    assert!(rms(&render(&mut e, 4_800), 0) < 0.05);
    e.remove_effect(lp).unwrap();
    assert!(rms(&render(&mut e, 4_800), 0) > 0.34);
    assert!(matches!(
        e.remove_effect(lp),
        Err(AudioError::UnknownEffect(_))
    ));
}

#[test]
fn effect_count_is_limited_per_group() {
    let mut e = AudioEngine::headless(EngineConfig {
        max_effects_per_group: 2,
        ..EngineConfig::default()
    })
    .unwrap();
    let d = Effect::Distortion(DistortionParams::default());
    let first = e.add_effect(GroupId::UI, d, 0).unwrap();
    e.add_effect(GroupId::UI, d, 5).unwrap();
    assert!(matches!(
        e.add_effect(GroupId::UI, d, 0),
        Err(AudioError::Limit(_))
    ));
    e.remove_effect(first).unwrap();
    e.add_effect(GroupId::UI, d, 0).unwrap();
}

// ---------------------------------------------------------------------------------------------
// Streams, clocks, backends

/// A stream that counts up: sample n = n * 1e-5, `len` frames long, mono.
struct Counter {
    pos: u64,
    len: u64,
}

impl StreamSource for Counter {
    fn channels(&self) -> u16 {
        1
    }
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn length_frames(&self) -> Option<u64> {
        None
    }
    fn read(&mut self, out: &mut [f32]) -> Result<usize, AudioError> {
        let n = (out.len() as u64).min(self.len - self.pos) as usize;
        for (i, s) in out[..n].iter_mut().enumerate() {
            *s = (self.pos + i as u64) as f32 * 1e-5;
        }
        self.pos += n as u64;
        Ok(n)
    }
    fn seek(&mut self, frame: u64) -> Result<(), AudioError> {
        self.pos = frame.min(self.len);
        Ok(())
    }
}

#[test]
fn custom_stream_sources_play_seek_loop_and_end() {
    let mut e = engine();
    let s = e
        .create_stream_sound(
            Box::new(Counter {
                pos: 0,
                len: 10_000,
            }),
            SoundProperties {
                volume: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(e.time_length(s).unwrap(), None);
    e.set_time_position(s, 5_000.0 / RATE as f64).unwrap();
    e.play(s).unwrap();
    let out = render(&mut e, BLOCK);
    for (n, f) in out.iter().enumerate() {
        assert!((f[0] - (5_000 + n) as f32 * 1e-5).abs() < 1e-6);
    }
    // Runs to the end (length discovered at EOF) and ends.
    render(&mut e, 5_000);
    e.update(0.0);
    assert_eq!(e.drain_events(), vec![SoundEvent::Ended(s)]);
    assert_eq!(e.time_length(s).unwrap(), Some(10_000.0 / RATE as f64));

    // Looped: wraps back to the start through a seek.
    e.set_looped(s, true).unwrap();
    e.set_time_position(s, 9_900.0 / RATE as f64).unwrap();
    e.play(s).unwrap();
    // Commands land on the next block boundary, so find where playback starts.
    let out = render(&mut e, 6 * BLOCK);
    let start = out
        .iter()
        .position(|f| (f[0] - 0.099).abs() < 1e-6)
        .unwrap();
    assert!(start < BLOCK);
    let after_wrap = out[start + 150][0];
    assert!((after_wrap - 50.0 * 1e-5).abs() < 1e-6, "{after_wrap}");
    e.update(0.0);
    assert_eq!(
        e.drain_events(),
        vec![SoundEvent::DidLoop { sound: s, count: 1 }]
    );

    struct Bad;
    impl StreamSource for Bad {
        fn channels(&self) -> u16 {
            6
        }
        fn sample_rate(&self) -> u32 {
            RATE
        }
        fn length_frames(&self) -> Option<u64> {
            None
        }
        fn read(&mut self, _: &mut [f32]) -> Result<usize, AudioError> {
            Ok(0)
        }
        fn seek(&mut self, _: u64) -> Result<(), AudioError> {
            Ok(())
        }
    }
    assert!(
        e.create_stream_sound(Box::new(Bad), SoundProperties::default())
            .is_err()
    );
}

#[test]
fn clocked_headless_engine_advances_with_update() {
    let mut e = AudioEngine::headless_clocked(EngineConfig::default()).unwrap();
    let a = dc(&mut e, 0.1, 4_800); // 0.1 s
    let s = e
        .create_sound(
            a,
            SoundProperties {
                playing: true,
                ..SoundProperties::default()
            },
        )
        .unwrap();
    e.update(0.05);
    assert!(e.is_playing(s).unwrap());
    assert!((e.time_position(s).unwrap() - 0.05).abs() < 0.002);
    e.update(0.06);
    assert_eq!(e.drain_events(), vec![SoundEvent::Ended(s)]);
    assert!(!e.is_playing(s).unwrap());
}

/// A fake device: keeps the renderer so the test can pull from it like a callback would.
struct FakeBackend {
    renderer: std::sync::Arc<std::sync::Mutex<Option<MixRenderer>>>,
}

impl AudioBackend for FakeBackend {
    fn name(&self) -> &str {
        "fake"
    }
    fn sample_rate(&self) -> u32 {
        44_100
    }
    fn start(&mut self, renderer: MixRenderer) -> Result<(), AudioError> {
        *self.renderer.lock().unwrap() = Some(renderer);
        Ok(())
    }
}

#[test]
fn custom_backends_drive_the_mixer() {
    let shared = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut e = AudioEngine::with_backend(
        Box::new(FakeBackend {
            renderer: shared.clone(),
        }),
        EngineConfig::default(),
    )
    .unwrap();
    assert_eq!(e.sample_rate(), 44_100);
    assert_eq!(e.backend_name(), "fake");
    assert!(!e.is_headless());
    assert!(matches!(
        e.render(&mut [0.0; 4]),
        Err(AudioError::NotHeadless)
    ));
    let a = e.insert_asset(SoundAsset::from_mono(vec![0.25; 44_100], 44_100).unwrap());
    e.create_sound(a, playing(1.0)).unwrap();
    let mut out = vec![0.0; 2 * 128];
    let mut guard = shared.lock().unwrap();
    let renderer = guard.as_mut().unwrap();
    assert_eq!(renderer.sample_rate(), 44_100);
    renderer.render(&mut out);
    assert!(out.iter().all(|s| (*s - 0.25).abs() < 1e-6));
}
