//! Errors the renderer returns instead of panicking.

/// Why the renderer could not set up or draw a frame.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    /// No graphics adapter was found on any of the backends tried.
    #[error("no graphics adapter found (backends tried: {tried:?})")]
    NoAdapter {
        /// Every backend that was asked for an adapter.
        tried: wgpu::Backends,
    },
    /// The adapter refused to create a device.
    #[error("could not create the graphics device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    /// The window surface could not be created.
    #[error("could not create the window surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    /// The surface reports no format, so the adapter cannot present to it.
    #[error("the window surface cannot be presented by this adapter")]
    SurfaceUnsupported,
    /// Acquiring the next surface image raised a validation error (a bug in how the surface is used).
    #[error("acquiring the next surface image failed validation")]
    SurfaceValidation,
    /// A width or height was zero or above the adapter's maximum texture size.
    #[error("invalid render size {width}x{height}")]
    InvalidSize {
        /// The requested width in pixels.
        width: u32,
        /// The requested height in pixels.
        height: u32,
    },
    /// The scene could not attach to the data model (no free change consumer).
    #[error("the scene could not attach to the data model: {0}")]
    DataModel(#[from] metallic_core::DmError),
    /// The adapter cannot render to this format with the requested sample count.
    #[error("cannot render to {format:?} with {samples} sample(s)")]
    UnsupportedTarget {
        /// The colour format asked for.
        format: wgpu::TextureFormat,
        /// The sample count asked for.
        samples: u32,
    },
}
