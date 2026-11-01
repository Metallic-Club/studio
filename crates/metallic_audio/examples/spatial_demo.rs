//! Three generated tones circle the listener on the default output device.
//!
//! `cargo run -p metallic_audio --example spatial_demo -- [seconds] [--mute]` (default 12 s;
//! `--mute` runs everything with the master volume at 0, a silent smoke test of the device path).
//! Listen on headphones: a low hum orbits slowly at head height, a mid tone orbits the other
//! way and passes behind you (darker), a high chirp flies past fast (Doppler). Every fourth
//! second the occlusion hook "puts a wall up" and everything is muffled.
//!
//! On a machine without an output device (CI, servers) it prints a note and exits with
//! success, so it never needs audio hardware to build or run.

use metallic_audio::{
    AudioEngine, AudioError, Effect, Emitter, EngineConfig, GroupId, Listener, ReverbParams,
    RollOffMode, SoundAsset, SoundProperties, Vec3,
};
use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn tone(
    freq: f32,
    seconds: f32,
    rate: u32,
    shape: impl Fn(f32) -> f32,
) -> Result<SoundAsset, AudioError> {
    let frames = (seconds * rate as f32) as usize;
    let samples = (0..frames)
        .map(|n| {
            let t = n as f32 / rate as f32;
            shape(t) * (TAU * freq * t).sin()
        })
        .collect();
    SoundAsset::from_mono(samples, rate)
}

fn main() -> Result<(), AudioError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds: f32 = args.iter().find_map(|s| s.parse().ok()).unwrap_or(12.0);
    let mute = args.iter().any(|a| a == "--mute");
    let mut engine = match AudioEngine::open_device(EngineConfig::default()) {
        Ok(engine) => engine,
        Err(e) => {
            println!("no audio device ({e}); nothing to play.");
            return Ok(());
        }
    };
    println!(
        "playing on '{}' at {} Hz for {seconds} s",
        engine.backend_name(),
        engine.sample_rate()
    );
    if mute {
        engine.set_group_volume(GroupId::MASTER, 0.0)?;
    }

    let rate = 48_000;
    // Whole numbers of cycles per loop so the loops are seamless.
    let hum = engine.insert_asset(tone(110.0, 1.0, rate, |_| 0.6)?);
    let mid = engine.insert_asset(tone(440.0, 1.0, rate, |t| {
        0.4 * (0.6 + 0.4 * (TAU * 2.0 * t).sin())
    })?);
    let chirp = engine.insert_asset(tone(1_760.0, 0.25, rate, |t| {
        0.3 * (1.0 - 4.0 * t).max(0.0)
    })?);

    let props = |emitter| SoundProperties {
        playing: true,
        looped: true,
        volume: 1.0,
        roll_off_mode: RollOffMode::InverseTapered,
        roll_off_min_distance: 6.0,
        roll_off_max_distance: 120.0,
        emitter: Some(emitter),
        ..SoundProperties::default()
    };
    let start = Emitter::at(Vec3::new(10.0, 0.0, 0.0));
    let sounds = [
        engine.create_sound(hum, props(start))?,
        engine.create_sound(mid, props(start))?,
        engine.create_sound(chirp, props(start))?,
    ];
    engine.add_effect(
        GroupId::SFX,
        Effect::Reverb(ReverbParams {
            decay_time: 1.2,
            wet_level: -10.0,
            dry_level: 0.0,
            ..ReverbParams::default()
        }),
        0,
    )?;
    engine.set_listener(Listener::default());

    let walled = Arc::new(AtomicBool::new(false));
    let hook_walled = Arc::clone(&walled);
    engine.set_occlusion_hook(move |_, _| {
        if hook_walled.load(Ordering::Relaxed) {
            1.0
        } else {
            0.0
        }
    });

    let began = Instant::now();
    let mut last = began;
    while began.elapsed().as_secs_f32() < seconds {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;
        let t = began.elapsed().as_secs_f32();
        walled.store((t as u32) % 4 == 3, Ordering::Relaxed);

        // (radius, angular speed rad/s, height) per sound.
        let orbits = [
            (10.0f32, 0.6f32, 0.0f32),
            (18.0, -0.9, 2.0),
            (35.0, 2.4, 6.0),
        ];
        for (sound, (radius, speed, height)) in sounds.iter().zip(orbits) {
            let a = t * speed;
            let position = Vec3::new(a.cos() * radius, height, a.sin() * radius);
            let velocity = Vec3::new(-a.sin(), 0.0, a.cos()) * radius * speed;
            engine.set_emitter(*sound, Some(Emitter { position, velocity }))?;
        }
        engine.update(dt);
        if let Some(err) = engine.backend_error() {
            println!("device reported: {err}");
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    for sound in sounds {
        engine.stop(sound)?;
    }
    std::thread::sleep(Duration::from_millis(100));
    let stats = engine.stats();
    println!(
        "rendered {} frames, {} voices at the end",
        stats.frames_rendered, stats.real_voices
    );
    Ok(())
}
