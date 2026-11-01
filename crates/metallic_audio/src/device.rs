//! Real output through the system's default audio device, using cpal (Apache-2.0).
//!
//! The cpal stream lives on its own thread (`metallic-audio-device`) so the engine stays `Send`
//! on every platform (cpal streams are not `Send` everywhere). That thread only owns the stream;
//! samples are produced in cpal's callback by the [`MixRenderer`].

use crate::backend::AudioBackend;
use crate::error::AudioError;
use crate::mixer::MixRenderer;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Frames rendered per inner step of the device callback (bounds the scratch buffer).
const CALLBACK_CHUNK: usize = 4096;

enum Control {
    Start(Box<MixRenderer>),
    Stop,
}

/// Output to the default device of the default host (WASAPI on Windows, CoreAudio on macOS,
/// ALSA/PulseAudio on Linux). Mono, stereo and multichannel devices are supported: stereo goes
/// to the first two channels, mono gets the average, extra channels stay silent.
pub struct DeviceBackend {
    name: String,
    sample_rate: u32,
    control: Sender<Control>,
    started: Receiver<Result<(), AudioError>>,
    error: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for DeviceBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceBackend")
            .field("name", &self.name)
            .field("sample_rate", &self.sample_rate)
            .finish()
    }
}

impl DeviceBackend {
    /// Open the default output device. Fails with [`AudioError::NoDevice`] on machines without
    /// one (servers, CI runners).
    pub fn open() -> Result<Self, AudioError> {
        let (info_tx, info_rx) = channel();
        let (control_tx, control_rx) = channel();
        let (started_tx, started_rx) = channel();
        let error = Arc::new(Mutex::new(None));
        let thread_error = Arc::clone(&error);
        let thread = std::thread::Builder::new()
            .name("metallic-audio-device".into())
            .spawn(move || device_thread(info_tx, control_rx, started_tx, thread_error))?;
        let info = info_rx
            .recv()
            .map_err(|_| AudioError::Device("device thread exited".into()));
        let (name, sample_rate) = match info.and_then(|r| r) {
            Ok(info) => info,
            Err(e) => {
                // The thread has already returned; reap it before reporting.
                if thread.join().is_err() {
                    return Err(AudioError::Device("device thread panicked".into()));
                }
                return Err(e);
            }
        };
        Ok(Self {
            name,
            sample_rate,
            control: control_tx,
            started: started_rx,
            error,
            thread: Some(thread),
        })
    }
}

impl AudioBackend for DeviceBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn start(&mut self, renderer: MixRenderer) -> Result<(), AudioError> {
        self.control
            .send(Control::Start(Box::new(renderer)))
            .map_err(|_| AudioError::Device("device thread exited".into()))?;
        self.started
            .recv()
            .map_err(|_| AudioError::Device("device thread exited".into()))?
    }

    fn last_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }
}

impl Drop for DeviceBackend {
    fn drop(&mut self) {
        // The thread may already be gone; either way it stops.
        let _ = self.control.send(Control::Stop);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            // A panic on the device thread has nothing left to clean up.
        }
    }
}

fn device_thread(
    info: Sender<Result<(String, u32), AudioError>>,
    control: Receiver<Control>,
    started: Sender<Result<(), AudioError>>,
    error: Arc<Mutex<Option<String>>>,
) {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        let _ = info.send(Err(AudioError::NoDevice));
        return;
    };
    let supported = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            let _ = info.send(Err(AudioError::Device(e.to_string())));
            return;
        }
    };
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "default output".into());
    let format = supported.sample_format();
    let config = supported.config();
    if info.send(Ok((name, config.sample_rate))).is_err() {
        return;
    }
    let renderer = match control.recv() {
        Ok(Control::Start(r)) => r,
        Ok(Control::Stop) | Err(_) => return,
    };
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, renderer, error),
        SampleFormat::I16 => build::<i16>(&device, config, renderer, error),
        SampleFormat::U16 => build::<u16>(&device, config, renderer, error),
        SampleFormat::I32 => build::<i32>(&device, config, renderer, error),
        SampleFormat::F64 => build::<f64>(&device, config, renderer, error),
        SampleFormat::I8 => build::<i8>(&device, config, renderer, error),
        SampleFormat::U8 => build::<u8>(&device, config, renderer, error),
        other => Err(AudioError::Device(format!(
            "unsupported device sample format {other}"
        ))),
    };
    let stream = match stream.and_then(|s| {
        s.play()
            .map(|_| s)
            .map_err(|e| AudioError::Device(e.to_string()))
    }) {
        Ok(s) => s,
        Err(e) => {
            let _ = started.send(Err(e));
            return;
        }
    };
    if started.send(Ok(())).is_err() {
        return;
    }
    // Park until the engine is dropped; the stream plays from cpal's own thread meanwhile.
    while let Ok(msg) = control.recv() {
        if matches!(msg, Control::Stop) {
            break;
        }
    }
    drop(stream);
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut renderer: Box<MixRenderer>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = (config.channels as usize).max(1);
    let mut scratch = vec![0.0f32; CALLBACK_CHUNK * 2];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _info: &cpal::OutputCallbackInfo| {
                for chunk in data.chunks_mut(CALLBACK_CHUNK * channels) {
                    let frames = chunk.len() / channels;
                    let stereo = &mut scratch[..frames * 2];
                    renderer.render(stereo);
                    for (frame, out) in chunk.chunks_mut(channels).enumerate() {
                        let (l, r) = (stereo[2 * frame], stereo[2 * frame + 1]);
                        if channels == 1 {
                            out[0] = T::from_sample(0.5 * (l + r));
                        } else {
                            out[0] = T::from_sample(l);
                            out[1] = T::from_sample(r);
                            for s in out.iter_mut().skip(2) {
                                *s = T::from_sample(0.0f32);
                            }
                        }
                    }
                }
            },
            move |e: cpal::Error| {
                if let Ok(mut slot) = error.lock() {
                    *slot = Some(e.to_string());
                }
            },
            None,
        )
        .map_err(|e| AudioError::Device(e.to_string()))
}
