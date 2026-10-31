//! Device creation: a standalone window/headless device, or one borrowed from a host.
use std::fmt;

/// Why a device could not be created.
#[derive(Debug)]
pub enum RenderError {
    /// No suitable graphics adapter was found.
    NoAdapter(String),
    /// The adapter refused to create a device.
    Device(String),
    /// Reading a rendered frame back from the GPU failed.
    Readback(String),
    /// A capture was requested with a zero width or height.
    InvalidSize,
    /// The window surface could not be created.
    Surface(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::NoAdapter(e) => write!(f, "no graphics adapter: {e}"),
            RenderError::Device(e) => write!(f, "could not create graphics device: {e}"),
            RenderError::Readback(e) => write!(f, "could not read back the frame: {e}"),
            RenderError::InvalidSize => write!(f, "cannot capture a zero-sized frame"),
            RenderError::Surface(e) => write!(f, "could not create window surface: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// The wgpu device and queue the renderer draws with.
#[derive(Clone)]
pub struct RenderContext {
    /// The logical device.
    pub device: wgpu::Device,
    /// The command queue.
    pub queue: wgpu::Queue,
    /// The adapter, when this context created it (needed to query surface formats).
    pub adapter: Option<wgpu::Adapter>,
    /// Adapter name for logs.
    pub adapter_name: String,
}

impl RenderContext {
    /// Creates a device that can present to `window` and returns it with the window's surface.
    ///
    /// `window` is anything wgpu accepts as a surface target, for example an
    /// `Arc<winit::window::Window>`.
    pub fn new_for_window(
        window: impl Into<wgpu::SurfaceTarget<'static>>,
    ) -> Result<(RenderContext, wgpu::Surface<'static>), RenderError> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|e| RenderError::Surface(e.to_string()))?;
        let ctx = pollster::block_on(Self::from_instance(&instance, Some(&surface)))?;
        Ok((ctx, surface))
    }

    /// Creates a device with no window, for tests and screenshots.
    pub fn new_headless() -> Result<RenderContext, RenderError> {
        let instance = wgpu::Instance::default();
        pollster::block_on(Self::from_instance(&instance, None))
    }

    /// Like [`RenderContext::new_headless`], limited to `backends` (for example
    /// `wgpu::Backends::GL` to check the GL path).
    pub fn new_headless_with_backends(
        backends: wgpu::Backends,
    ) -> Result<RenderContext, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        pollster::block_on(Self::from_instance(&instance, None))
    }

    /// Wraps the device, queue and adapter of a host that already owns them (the eframe/egui
    /// Studio editor passes the ones from its `wgpu_render_state`). The adapter lets the
    /// renderer check which HDR target format it can use.
    pub fn from_eframe(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: wgpu::Adapter,
    ) -> RenderContext {
        let adapter_name = adapter.get_info().name;
        RenderContext {
            device,
            queue,
            adapter: Some(adapter),
            adapter_name,
        }
    }

    async fn from_instance(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<RenderContext, RenderError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: surface,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("metallic device"),
                ..Default::default()
            })
            .await
            .map_err(|e| RenderError::Device(e.to_string()))?;
        let adapter_name = adapter.get_info().name;
        Ok(RenderContext {
            device,
            queue,
            adapter: Some(adapter),
            adapter_name,
        })
    }
}
