//! Decoder round trips on tiny files generated in the test (no recorded audio in the repo).

use metallic_audio::{
    AudioEngine, AudioError, DecoderStream, EngineConfig, SoundAsset, SoundProperties, StreamSource,
};
use std::io::Cursor;
use std::sync::Arc;

fn wav(
    spec: hound::WavSpec,
    write: impl FnOnce(&mut hound::WavWriter<&mut Cursor<Vec<u8>>>),
) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
        write(&mut writer);
        writer.finalize().unwrap();
    }
    cursor.into_inner()
}

fn tone_i16(frames: usize, channels: usize) -> Vec<i16> {
    (0..frames * channels)
        .map(|i| {
            let frame = (i / channels) as f32;
            let ch = (i % channels) as f32;
            ((frame * 0.05 + ch).sin() * 20_000.0) as i16
        })
        .collect()
}

#[test]
fn wav_16_bit_mono_round_trip() {
    let samples = tone_i16(3_000, 1);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 22_050,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let bytes = wav(spec, |w| {
        samples.iter().for_each(|s| w.write_sample(*s).unwrap())
    });
    let asset = SoundAsset::decode(&bytes, Some("wav")).unwrap();
    assert_eq!(
        (asset.channels(), asset.sample_rate(), asset.frames()),
        (1, 22_050, 3_000)
    );
    for (a, b) in asset.samples().iter().zip(&samples) {
        assert!((a - *b as f32 / 32_768.0).abs() < 1e-6);
    }
    assert!((asset.duration_secs() - 3_000.0 / 22_050.0).abs() < 1e-9);
    // The hint is optional: content is probed.
    assert_eq!(SoundAsset::decode(&bytes, None).unwrap().frames(), 3_000);
}

#[test]
fn wav_float_stereo_is_exact() {
    let samples: Vec<f32> = (0..2_000)
        .map(|i| ((i as f32) * 0.01).sin() * 0.8)
        .collect();
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let bytes = wav(spec, |w| {
        samples.iter().for_each(|s| w.write_sample(*s).unwrap())
    });
    let asset = SoundAsset::decode(&bytes, Some("wav")).unwrap();
    assert_eq!((asset.channels(), asset.frames()), (2, 1_000));
    assert_eq!(asset.samples(), &samples[..]);
}

#[test]
fn wav_24_bit_round_trip() {
    let samples: Vec<i32> = (0..1_500)
        .map(|i| ((i as f32 * 0.03).sin() * 8_000_000.0) as i32)
        .collect();
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44_100,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let bytes = wav(spec, |w| {
        samples.iter().for_each(|s| w.write_sample(*s).unwrap())
    });
    let asset = SoundAsset::decode(&bytes, Some("wav")).unwrap();
    for (a, b) in asset.samples().iter().zip(&samples) {
        assert!((a - *b as f32 / 8_388_608.0).abs() < 1e-6);
    }
}

#[test]
fn six_channel_wav_folds_to_stereo() {
    // Only the centre channel carries signal: it should land equally in both outputs.
    let frames = 500;
    let spec = hound::WavSpec {
        channels: 6,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let bytes = wav(spec, |w| {
        for _ in 0..frames {
            for ch in 0..6 {
                w.write_sample(if ch == 2 { 0.5f32 } else { 0.0 }).unwrap();
            }
        }
    });
    let asset = SoundAsset::decode(&bytes, None).unwrap();
    assert_eq!((asset.channels(), asset.frames()), (2, frames));
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let expected = 0.5 * h / (1.0 + h + h);
    for f in asset.samples().as_chunks::<2>().0 {
        assert!((f[0] - expected).abs() < 1e-6 && (f[1] - expected).abs() < 1e-6);
    }
}

fn flac_bytes(samples: &[i32], channels: usize, rate: usize) -> Vec<u8> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let config = flacenc::config::Encoder::default().into_verified().unwrap();
    let source = flacenc::source::MemSource::from_samples(samples, channels, 16, rate);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size).unwrap();
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).unwrap();
    sink.as_slice().to_vec()
}

#[test]
fn flac_round_trip_is_lossless() {
    // A whole number of 4,096-frame blocks: flacenc counts a short last block in STREAMINFO's
    // minimum block size, which Symphonia's frame sync then rejects (libFLAC files are fine).
    let samples: Vec<i32> = tone_i16(8_192, 2).into_iter().map(i32::from).collect();
    let bytes = flac_bytes(&samples, 2, 44_100);
    let asset = SoundAsset::decode(&bytes, Some("flac")).unwrap();
    assert_eq!(
        (asset.channels(), asset.sample_rate(), asset.frames()),
        (2, 44_100, 8_192)
    );
    for (a, b) in asset.samples().iter().zip(&samples) {
        assert!((a - *b as f32 / 32_768.0).abs() < 1e-6);
    }
}

#[test]
fn streaming_matches_full_decode_in_any_chunk_size() {
    let samples = tone_i16(10_000, 2);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let bytes: Arc<[u8]> = wav(spec, |w| {
        samples.iter().for_each(|s| w.write_sample(*s).unwrap())
    })
    .into();
    let full = SoundAsset::decode(&bytes, Some("wav")).unwrap();

    let mut stream = DecoderStream::from_bytes(Arc::clone(&bytes), Some("wav")).unwrap();
    assert_eq!(
        (
            stream.channels(),
            stream.sample_rate(),
            stream.length_frames()
        ),
        (2, 48_000, Some(10_000))
    );
    let mut streamed = Vec::new();
    for size in [1usize, 3, 100, 777, 4_096].iter().cycle() {
        let mut buf = vec![0.0; size * 2];
        let n = stream.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        streamed.extend_from_slice(&buf[..n * 2]);
    }
    assert_eq!(streamed, full.samples());

    // Seek lands exactly on the requested frame.
    for target in [0u64, 1, 4_321, 9_999] {
        stream.seek(target).unwrap();
        let mut buf = vec![0.0; 8];
        let n = stream.read(&mut buf).unwrap();
        assert!(n >= 1);
        let at = target as usize * 2;
        assert_eq!(&buf[..2], &full.samples()[at..at + 2], "seek to {target}");
    }
}

#[test]
fn streamed_and_decoded_sounds_sound_the_same() {
    let samples = tone_i16(6_000, 1);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 32_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let bytes = wav(spec, |w| {
        samples.iter().for_each(|s| w.write_sample(*s).unwrap())
    });
    let render = |streaming: bool| {
        let mut e = AudioEngine::headless(EngineConfig::default()).unwrap();
        let asset = if streaming {
            e.load_streaming(bytes.clone(), Some("wav")).unwrap()
        } else {
            e.load_asset(&bytes, Some("wav")).unwrap()
        };
        let sound = e
            .create_sound(
                asset,
                SoundProperties {
                    playing: true,
                    volume: 1.0,
                    ..SoundProperties::default()
                },
            )
            .unwrap();
        // WAV states its length, so TimeLength is known before anything is decoded.
        assert_eq!(e.time_length(sound).unwrap(), Some(6_000.0 / 32_000.0));
        e.render_seconds(0.3).unwrap()
    };
    let decoded = render(false);
    let streamed = render(true);
    assert_eq!(decoded, streamed);
    assert!(decoded.iter().any(|s| s.abs() > 0.1));
}

#[test]
fn files_on_disk_decode() {
    let samples = tone_i16(1_000, 1);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let bytes = wav(spec, |w| {
        samples.iter().for_each(|s| w.write_sample(*s).unwrap())
    });
    let dir = std::env::temp_dir().join(format!("metallic_audio_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tone.wav");
    std::fs::write(&path, &bytes).unwrap();
    let asset = SoundAsset::decode_file(&path).unwrap();
    assert_eq!(asset.frames(), 1_000);
    let mut stream = DecoderStream::open(&path).unwrap();
    assert_eq!(stream.read(&mut [0.0; 10]).unwrap(), 10);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(matches!(
        SoundAsset::decode_file(&path),
        Err(AudioError::Io(_))
    ));
}

#[test]
fn garbage_is_a_typed_error() {
    assert!(matches!(
        SoundAsset::decode(b"definitely not audio", None),
        Err(AudioError::Unsupported(_))
    ));
    assert!(matches!(
        SoundAsset::decode(&[], Some("ogg")),
        Err(AudioError::Unsupported(_))
    ));
    let mut e = AudioEngine::headless(EngineConfig::default()).unwrap();
    assert!(e.load_streaming(vec![0u8; 64], Some("mp3")).is_err());
    assert!(e.load_asset(b"RIFF....WAVEfmt ", Some("wav")).is_err());
}
