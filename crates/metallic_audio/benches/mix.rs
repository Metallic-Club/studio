//! Mixing cost: 64 spatial voices for one second of 48 kHz stereo.
//!
//! `cargo bench -p metallic_audio --bench mix`. Real time is 1 s per iteration, so the
//! reported time is also the fraction of one CPU core the mixer needs (10 ms = 1 %).

use criterion::{Criterion, criterion_group, criterion_main};
use metallic_audio::{
    AudioEngine, Effect, Emitter, EngineConfig, GroupId, ReverbParams, RollOffMode, SoundAsset,
    SoundProperties, Vec3,
};
use std::f32::consts::TAU;
use std::hint::black_box;

const RATE: u32 = 48_000;
const VOICES: usize = 64;

fn scene(with_effects: bool) -> AudioEngine {
    let mut engine = AudioEngine::headless(EngineConfig {
        max_voices: VOICES,
        ..EngineConfig::default()
    })
    .unwrap_or_else(|e| panic!("engine: {e}"));
    for i in 0..VOICES {
        // Different lengths, rates and pitches so voices do not move in lockstep.
        let rate = if i % 2 == 0 { 44_100 } else { 48_000 };
        let freq = 110.0 + 37.0 * i as f32;
        let frames = rate as usize / 2 + i * 101;
        let samples = (0..frames)
            .map(|n| 0.2 * (TAU * freq * n as f32 / rate as f32).sin())
            .collect();
        let asset = engine.insert_asset(
            SoundAsset::from_mono(samples, rate).unwrap_or_else(|e| panic!("asset: {e}")),
        );
        let angle = TAU * i as f32 / VOICES as f32;
        let radius = 5.0 + (i % 8) as f32 * 7.0;
        let emitter = Emitter {
            position: Vec3::new(angle.cos() * radius, (i % 3) as f32, angle.sin() * radius),
            velocity: Vec3::new(-angle.sin(), 0.0, angle.cos()) * 20.0,
        };
        let props = SoundProperties {
            playing: true,
            looped: true,
            volume: 0.5,
            playback_speed: 0.9 + (i % 5) as f32 * 0.05,
            roll_off_mode: RollOffMode::InverseTapered,
            roll_off_max_distance: 200.0,
            emitter: Some(emitter),
            ..SoundProperties::default()
        };
        engine
            .create_sound(asset, props)
            .unwrap_or_else(|e| panic!("sound: {e}"));
    }
    if with_effects {
        engine.set_occlusion_hook(|_, e| if e.x > 0.0 { 0.6 } else { 0.0 });
        engine.update(0.0);
        engine
            .add_effect(GroupId::MASTER, Effect::Reverb(ReverbParams::default()), 0)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    engine
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("mix");
    group.sample_size(20);
    let mut buf = vec![0.0f32; RATE as usize * 2];

    let mut plain = scene(false);
    group.bench_function("64_spatial_voices_1s", |b| {
        b.iter(|| {
            plain.render(&mut buf).unwrap_or_else(|e| panic!("{e}"));
            black_box(&buf);
        })
    });

    let mut rich = scene(true);
    group.bench_function("64_spatial_voices_occlusion_reverb_1s", |b| {
        b.iter(|| {
            rich.render(&mut buf).unwrap_or_else(|e| panic!("{e}"));
            black_box(&buf);
        })
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
