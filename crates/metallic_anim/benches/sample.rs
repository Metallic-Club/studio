//! Sampling cost: 1,000 characters x 30 joints, each joint with a 16-key rotation curve and the
//! root with a translation curve. Every character plays at its own time offset.
//!
//! Run: `cargo bench -p metallic_anim -j 4`.

use criterion::{Criterion, criterion_group, criterion_main};
use metallic_anim::glam::{Quat, Vec3};
use metallic_anim::{
    Animator, Channel, Clip, ClipBinding, Curve, Joint, JointChannel, Keyframe, LoopMode,
    PlayOptions, Pose, Skeleton, TrackTarget, Transform, sample_clip,
};
use std::hint::black_box;
use std::sync::Arc;

const CHARACTERS: usize = 1_000;
const JOINTS: usize = 30;
const KEYS: usize = 16;

fn skeleton() -> Skeleton {
    let joints = (0..JOINTS)
        .map(|i| Joint {
            name: format!("J{i}"),
            parent: if i == 0 { None } else { Some((i - 1) / 2) },
            rest: Transform::from_translation(Vec3::new(0.0, 0.5, 0.0)),
            length: 0.5,
        })
        .collect();
    Skeleton::new(joints).unwrap_or_else(|e| panic!("bench skeleton: {e}"))
}

fn clip() -> Clip {
    let mut clip = Clip::new("Bench", 1.0);
    clip.loop_mode = LoopMode::Loop;
    for j in 0..JOINTS {
        let keys = (0..KEYS)
            .map(|k| {
                let t = k as f32 / (KEYS - 1) as f32;
                let a = (t * std::f32::consts::TAU + j as f32).sin() * 0.6;
                Keyframe::new(
                    t,
                    Quat::from_euler(metallic_anim::glam::EulerRot::XYZ, a, a * 0.5, -a * 0.25),
                )
            })
            .collect();
        let curve = Curve::from_keys(keys).unwrap_or_else(|e| panic!("bench curve: {e}"));
        clip.tracks.push(metallic_anim::Track::new(
            TrackTarget::joint(format!("J{j}"), JointChannel::Rotation),
            Channel::Quat(curve),
        ));
    }
    let root = Curve::from_keys(
        (0..KEYS)
            .map(|k| {
                Keyframe::new(
                    k as f32 / (KEYS - 1) as f32,
                    Vec3::new(0.0, (k as f32).sin() * 0.1, 0.0),
                )
            })
            .collect(),
    )
    .unwrap_or_else(|e| panic!("bench root curve: {e}"));
    clip.tracks.push(metallic_anim::Track::new(
        TrackTarget::joint("J0", JointChannel::Translation),
        Channel::Vec3(root),
    ));
    clip
}

fn bench(c: &mut Criterion) {
    let s = skeleton();
    let clip = clip();
    let binding = ClipBinding::new(&clip, &s);
    let mut poses: Vec<Pose> = (0..CHARACTERS).map(|_| Pose::rest(&s)).collect();
    let mut model = Vec::with_capacity(JOINTS);
    let mut frame = 0u32;

    c.bench_function("sample 1000 characters x 30 joints", |b| {
        b.iter(|| {
            frame = frame.wrapping_add(1);
            let base = frame as f32 / 60.0;
            for (i, pose) in poses.iter_mut().enumerate() {
                let t = clip.local_time(base + i as f32 * 0.137);
                sample_clip(&clip, &binding, &s, t, pose);
            }
            black_box(&poses);
        });
    });

    c.bench_function("sample + model space, 1000 x 30", |b| {
        b.iter(|| {
            frame = frame.wrapping_add(1);
            let base = frame as f32 / 60.0;
            for (i, pose) in poses.iter_mut().enumerate() {
                let t = clip.local_time(base + i as f32 * 0.137);
                sample_clip(&clip, &binding, &s, t, pose);
                pose.model_space(&s, &mut model);
                black_box(&model);
            }
        });
    });

    // Full Animator path: two tracks crossfading per character.
    let skel = Arc::new(s.clone());
    let clip = Arc::new(clip);
    let mut animators: Vec<Animator> = (0..CHARACTERS)
        .map(|_| {
            let mut a = Animator::new(Arc::clone(&skel));
            let h1 = a.load(Arc::clone(&clip));
            let h2 = a.load(Arc::clone(&clip));
            a.play(h1, PlayOptions::default());
            a.update(0.5);
            a.crossfade(h1, h2, 1.0e6);
            a
        })
        .collect();
    c.bench_function("animator crossfade 1000 x 30", |b| {
        b.iter(|| {
            for (a, pose) in animators.iter_mut().zip(poses.iter_mut()) {
                a.update(1.0 / 60.0);
                a.evaluate(pose);
            }
            black_box(&poses);
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
