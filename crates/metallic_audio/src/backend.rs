//! Output backends: where mixed samples go.
//!
//! The engine never talks to an audio API directly; it hands a [`MixRenderer`] to an
//! [`AudioBackend`], which pulls samples from it on whatever thread the platform uses. Two
//! exist: the headless backend built into [`AudioEngine`](crate::AudioEngine) (tests, servers,
//! offline rendering) and [`DeviceBackend`](crate::DeviceBackend) (cpal, behind the `device`
//! feature). Another library (an HRTF engine, a console API) only needs this trait.

use crate::error::AudioError;
use crate::mixer::MixRenderer;

/// A sink that drives a [`MixRenderer`] in real time.
pub trait AudioBackend: Send {
    /// Short human-readable name (for logs and the Studio settings page).
    fn name(&self) -> &str;
    /// Output rate in Hz; the mixer is built for this rate.
    fn sample_rate(&self) -> u32;
    /// Take ownership of the renderer and start pulling interleaved stereo `f32` from
    /// [`MixRenderer::render`]. Called once.
    fn start(&mut self, renderer: MixRenderer) -> Result<(), AudioError>;
    /// The last asynchronous error the backend saw (device unplugged, underrun storm), if any.
    fn last_error(&self) -> Option<String> {
        None
    }
}
