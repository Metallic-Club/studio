//! Device creation: a standalone window/headless device, or one borrowed from a host.
//!
//! Every instance, adapter and device made here is made under one process-wide lock
//! ([`setup_lock`]). wgpu's OpenGL backend on Windows (WGL) is not safe to set up on several
//! threads at once: each instance makes a hidden window, sets its pixel format and creates a GL
//! context in the vendor's OpenGL driver, and when threads do that together the process dies
//! with an access violation or `STATUS_HEAP_CORRUPTION` (#276; upstream gfx-rs/wgpu#9760).
//! Serialising the setup removes the crash; tearing devices down and rendering stay parallel.
//! Headless devices also try Vulkan, DX12 or Metal first, so OpenGL is set up at all only where
//! none of those has an adapter (or when it is asked for).
use std::fmt;
use std::sync::{Mutex, MutexGuard};

/// Held while an instance, adapter or device is created. See the module documentation.
fn setup_lock() -> MutexGuard<'static, ()> {
    static SETUP: Mutex<()> = Mutex::new(());
    SETUP.lock().unwrap_or_else(|e| e.into_inner())
}

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
        let _setup = setup_lock();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|e| RenderError::Surface(e.to_string()))?;
        let ctx = pollster::block_on(Self::from_instance(&instance, Some(&surface)))?;
        Ok((ctx, surface))
    }

    /// Creates a device with no window, for tests and screenshots: Vulkan, DX12 or Metal, and
    /// OpenGL only when none of those has an adapter.
    pub fn new_headless() -> Result<RenderContext, RenderError> {
        Self::new_headless_with_backends(wgpu::Backends::PRIMARY)
            .or_else(|_| Self::new_headless_with_backends(wgpu::Backends::GL))
    }

    /// Like [`RenderContext::new_headless`], limited to `backends` (for example
    /// `wgpu::Backends::GL` to check the GL path).
    pub fn new_headless_with_backends(
        backends: wgpu::Backends,
    ) -> Result<RenderContext, RenderError> {
        let _setup = setup_lock();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        pollster::block_on(Self::from_instance(&instance, None))
    }

    /// A headless device on the platform's software adapter (WARP on Windows, a CPU Vulkan
    /// driver elsewhere when installed), for measuring how a change scales on a slow GPU.
    pub fn new_headless_software() -> Result<RenderContext, RenderError> {
        let instance = wgpu::Instance::default();
        pollster::block_on(Self::from_instance_with(&instance, None, true))
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
        Self::from_instance_with(instance, surface, false).await
    }

    async fn from_instance_with(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        software: bool,
    ) -> Result<RenderContext, RenderError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: software,
                compatible_surface: surface,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("metallic device"),
                // GPU frame timing (`Renderer::enable_gpu_timing`) where the adapter has it.
                required_features: adapter.features() & crate::renderer::GPU_TIMING_FEATURES,
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
