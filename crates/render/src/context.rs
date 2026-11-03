//! The instance, adapter, device and queue, and what the adapter can do.
//!
//! **Every instance, surface, adapter and device is created under one process-wide lock** ([`setup_lock`]).
//! wgpu's OpenGL backend on Windows (WGL) is not safe to set up on several threads at once: each GL instance makes
//! a hidden window, sets its pixel format and creates a context in the vendor's driver, and threads doing that
//! together killed the archived renderer's test runs with `STATUS_HEAP_CORRUPTION` (archive #276; upstream
//! gfx-rs/wgpu#9760, #6211). Serialising setup removed the crash. Rendering and teardown stay parallel.
//!
//! Backends are tried in tiers: Vulkan, Metal and DX12 first, and only when none of them has an adapter, GL. An
//! instance only ever enables one tier, so most machines never set up GL at all.

use std::sync::{Arc, Mutex, MutexGuard};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::RenderError;
use crate::surface::{RenderSurface, SurfaceWindow, WindowDisplay};

/// Held while an instance, surface, adapter or device is created. See the module documentation.
pub(crate) fn setup_lock() -> MutexGuard<'static, ()> {
    static SETUP: Mutex<()> = Mutex::new(());
    // A panic while holding the lock leaves nothing half-written behind it (the guarded data is `()`).
    SETUP.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Device features the renderer turns on whenever the adapter has them.
const WANTED_FEATURES: wgpu::Features = wgpu::Features::TIMESTAMP_QUERY
    .union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    .union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    .union(wgpu::Features::INDIRECT_FIRST_INSTANCE)
    .union(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
    // MSAA counts (and other format features) beyond WebGPU's guaranteed minimum, such as 2x and 8x.
    .union(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);

/// How [`RenderContext`] looks for an adapter.
#[derive(Clone, Debug)]
pub struct ContextOptions {
    /// Backends that may be used. Primary ones (Vulkan, Metal, DX12) are always tried before GL.
    ///
    /// The default is every native backend, narrowed by the `WGPU_BACKEND` environment variable when it is set
    /// (for example `WGPU_BACKEND=gl` to check the GL path).
    pub backends: wgpu::Backends,
    /// Which adapter to prefer when there are several (a laptop's integrated and discrete GPUs).
    pub power_preference: wgpu::PowerPreference,
    /// Use the vertex-buffer instance path (GLES's) even where vertex shaders can read storage buffers, by
    /// reporting [`Capabilities::vertex_storage`] as false. For tests of that path on any backend.
    pub force_vertex_instances: bool,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY | wgpu::Backends::GL),
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_vertex_instances: false,
        }
    }
}

/// Multisample counts a format can be rendered with, as a set of powers of two (1, 2, 4, 8, 16).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MsaaSampleCounts(u8);

impl MsaaSampleCounts {
    /// Every count wgpu knows, in ascending order.
    const ALL: [u32; 5] = [1, 2, 4, 8, 16];

    /// The counts `flags` allows.
    fn from_flags(flags: wgpu::TextureFormatFeatureFlags) -> Self {
        let mut bits = 0;
        for (i, &count) in Self::ALL.iter().enumerate() {
            if flags.sample_count_supported(count) {
                bits |= 1 << i;
            }
        }
        Self(bits)
    }

    /// Whether `samples` per pixel is supported.
    pub fn contains(self, samples: u32) -> bool {
        Self::ALL
            .iter()
            .position(|&count| count == samples)
            .is_some_and(|i| self.0 & (1 << i) != 0)
    }

    /// The largest supported count (1 when nothing else is).
    pub fn max(self) -> u32 {
        self.iter().last().unwrap_or(1)
    }

    /// The supported counts, ascending.
    pub fn iter(self) -> impl Iterator<Item = u32> {
        Self::ALL
            .into_iter()
            .enumerate()
            .filter(move |&(i, _)| self.0 & (1 << i) != 0)
            .map(|(_, count)| count)
    }

    /// Counts supported by both sets (for example a colour format's and the depth format's).
    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

/// What the adapter and device can do, recorded once at setup so later systems choose their path (GPU-driven or
/// CPU culling, storage or vertex instance buffers, timing on or off) without asking wgpu again.
#[derive(Clone, Debug)]
pub struct Capabilities {
    /// The adapter's name, for logs and bug reports.
    pub adapter_name: String,
    /// The backend in use.
    pub backend: wgpu::Backend,
    /// Discrete, integrated, software (`Cpu`), ...
    pub device_type: wgpu::DeviceType,
    /// The driver name and version, when the backend reports them.
    pub driver: String,
    /// Compute shaders exist (needed for GPU culling).
    pub compute: bool,
    /// Indirect draws exist (`draw_indirect`, `multi_draw_indirect`; the latter may be emulated by a loop).
    pub indirect_draw: bool,
    /// `multi_draw_indirect` runs natively and `multi_draw_indirect_count` exists (Vulkan 1.2+, DX12).
    pub multi_draw_indirect_native: bool,
    /// Indirect draws may use a non-zero first instance (needed to draw instance ranges from one buffer).
    pub indirect_first_instance: bool,
    /// Timestamp queries can be written at the start and end of passes, so per-pass GPU timing works.
    pub timestamp_queries: bool,
    /// Timestamps can also be written inside a pass (finer timing than per pass).
    pub timestamp_queries_inside_passes: bool,
    /// Vertex shaders can read storage buffers (instance data in a storage buffer; otherwise the vertex-buffer
    /// fallback, as on GLES).
    pub vertex_storage: bool,
    /// The largest 2D texture side in pixels.
    pub max_texture_size: u32,
    /// MSAA counts usable for both an 8-bit sRGB colour target and the `Depth32Float` depth buffer.
    pub msaa: MsaaSampleCounts,
    /// Transient (memoryless) attachments save memory here (tile-based GPUs: mobile, Apple).
    pub transient_attachments: bool,
}

impl Capabilities {
    fn new(adapter: &wgpu::Adapter, device: &wgpu::Device) -> Self {
        let info = adapter.get_info();
        let features = device.features();
        let limits = device.limits();
        let downlevel = adapter.get_downlevel_capabilities();
        let driver = match (info.driver.is_empty(), info.driver_info.is_empty()) {
            (false, false) => format!("{} {}", info.driver, info.driver_info),
            (false, true) => info.driver.clone(),
            (true, _) => info.driver_info.clone(),
        };
        let msaa = |format| msaa_sample_counts(adapter, device, format);
        Self {
            adapter_name: info.name.clone(),
            backend: info.backend,
            device_type: info.device_type,
            driver,
            compute: downlevel.flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS),
            indirect_draw: downlevel.flags.contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION),
            multi_draw_indirect_native: features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
            indirect_first_instance: features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            timestamp_queries: features.contains(wgpu::Features::TIMESTAMP_QUERY),
            timestamp_queries_inside_passes: features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
            vertex_storage: downlevel.flags.contains(wgpu::DownlevelFlags::VERTEX_STORAGE)
                && limits.max_storage_buffers_per_shader_stage > 0,
            max_texture_size: limits.max_texture_dimension_2d,
            msaa: msaa(wgpu::TextureFormat::Rgba8UnormSrgb).intersect(msaa(crate::depth::DEPTH_FORMAT)),
            transient_attachments: info.transient_saves_memory.unwrap_or(false),
        }
    }
}

/// The GPU the renderer draws with: instance, adapter, device and queue, plus the adapter's [`Capabilities`].
///
/// Cheap to clone (every field is reference-counted by wgpu).
#[derive(Clone, Debug)]
pub struct RenderContext {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    capabilities: Arc<Capabilities>,
}

impl RenderContext {
    /// Creates a device with no window, for offscreen rendering (tests, CI, thumbnails, headless capture).
    pub fn new_offscreen(options: &ContextOptions) -> Result<Self, RenderError> {
        for tier in backend_tiers(options.backends) {
            let _setup = setup_lock();
            let instance = new_instance(tier, None);
            if let Some(adapter) = request_adapter(&instance, options, None) {
                return Self::from_adapter(instance, adapter, options);
            }
        }
        Err(RenderError::NoAdapter {
            tried: options.backends,
        })
    }

    /// Creates a device that can present to `window`, and the window's surface sized `width` × `height` pixels.
    ///
    /// `window` is anything with window and display handles, for example an `Arc<winit::window::Window>`; the
    /// surface keeps it alive (and uses it again if the surface is lost).
    pub fn new_with_window<W>(
        window: Arc<W>,
        width: u32,
        height: u32,
        options: &ContextOptions,
    ) -> Result<(Self, RenderSurface), RenderError>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        let window: Arc<dyn SurfaceWindow> = window;
        for tier in backend_tiers(options.backends) {
            let _setup = setup_lock();
            // GL needs the display connection at instance creation (EGL on Wayland/X11); other backends ignore it.
            let display = tier
                .contains(wgpu::Backends::GL)
                .then(|| WindowDisplay(Arc::clone(&window)));
            let instance = new_instance(tier, display);
            let surface = instance.create_surface(wgpu::SurfaceTarget::from(Arc::clone(&window)))?;
            if let Some(adapter) = request_adapter(&instance, options, Some(&surface)) {
                let context = Self::from_adapter(instance, adapter, options)?;
                let surface = RenderSurface::new(&context, surface, window, width, height)?;
                return Ok((context, surface));
            }
        }
        Err(RenderError::NoAdapter {
            tried: options.backends,
        })
    }

    /// Creates a surface for another window on this device (Studio's extra viewports).
    ///
    /// Fails with [`RenderError::SurfaceUnsupported`] when this adapter cannot present to the window.
    pub fn create_surface<W>(&self, window: Arc<W>, width: u32, height: u32) -> Result<RenderSurface, RenderError>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        let window: Arc<dyn SurfaceWindow> = window;
        let surface = self.create_raw_surface(&window)?;
        RenderSurface::new(self, surface, window, width, height)
    }

    /// A wgpu surface for `window` on this context's instance, made under the setup lock.
    pub(crate) fn create_raw_surface(
        &self,
        window: &Arc<dyn SurfaceWindow>,
    ) -> Result<wgpu::Surface<'static>, RenderError> {
        let _setup = setup_lock();
        Ok(self
            .instance
            .create_surface(wgpu::SurfaceTarget::from(Arc::clone(window)))?)
    }

    fn from_adapter(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        options: &ContextOptions,
    ) -> Result<Self, RenderError> {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("metallic device"),
            required_features: adapter.features() & WANTED_FEATURES,
            // Everything the adapter offers: later systems size their buffers from `device.limits()`.
            required_limits: adapter.limits(),
            ..Default::default()
        }))?;
        let mut capabilities = Capabilities::new(&adapter, &device);
        capabilities.vertex_storage &= !options.force_vertex_instances;
        let capabilities = Arc::new(capabilities);
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            capabilities,
        })
    }

    /// The wgpu instance (surfaces for this device must come from it).
    pub fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    /// The adapter the device was created on.
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// The logical device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The command queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// What the adapter and device can do.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// The MSAA counts `format` can be rendered with on this adapter.
    pub fn msaa_sample_counts(&self, format: wgpu::TextureFormat) -> MsaaSampleCounts {
        msaa_sample_counts(&self.adapter, &self.device, format)
    }

    /// What the device allows for `format`: the adapter's own features when the device enabled them, otherwise the
    /// WebGPU minimum (creating a texture beyond that would fail validation).
    pub fn format_features(&self, format: wgpu::TextureFormat) -> wgpu::TextureFormatFeatures {
        format_features(&self.adapter, &self.device, format)
    }
}

fn format_features(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> wgpu::TextureFormatFeatures {
    let features = device.features();
    if features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
        adapter.get_texture_format_features(format)
    } else {
        format.guaranteed_format_features(features)
    }
}

fn msaa_sample_counts(adapter: &wgpu::Adapter, device: &wgpu::Device, format: wgpu::TextureFormat) -> MsaaSampleCounts {
    let features = format_features(adapter, device, format);
    if !features.allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
        return MsaaSampleCounts::default();
    }
    MsaaSampleCounts::from_flags(features.flags)
}

/// The backend sets to try, in order: the primary backends, then GL. Empty tiers are skipped.
fn backend_tiers(allowed: wgpu::Backends) -> impl Iterator<Item = wgpu::Backends> {
    [wgpu::Backends::PRIMARY, wgpu::Backends::SECONDARY]
        .into_iter()
        .map(move |tier| tier & allowed)
        .filter(|tier| !tier.is_empty())
}

fn new_instance(backends: wgpu::Backends, display: Option<WindowDisplay>) -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    // `WGPU_DEBUG`, `WGPU_VALIDATION` and the backend options may still be tuned from the environment.
    descriptor.flags = descriptor.flags.with_env();
    descriptor.backend_options = descriptor.backend_options.with_env();
    if let Some(display) = display {
        descriptor = descriptor.with_display_handle(Box::new(display));
    }
    wgpu::Instance::new(descriptor)
}

fn request_adapter(
    instance: &wgpu::Instance,
    options: &ContextOptions,
    surface: Option<&wgpu::Surface<'_>>,
) -> Option<wgpu::Adapter> {
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: options.power_preference,
        force_fallback_adapter: false,
        compatible_surface: surface,
        apply_limit_buckets: false,
    }))
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_backends_are_tried_before_gl() {
        let tiers: Vec<_> = backend_tiers(wgpu::Backends::all()).collect();
        assert_eq!(tiers.len(), 2);
        assert!(tiers[0].contains(wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12));
        assert!(!tiers[0].contains(wgpu::Backends::GL));
        assert_eq!(tiers[1], wgpu::Backends::GL);
    }

    #[test]
    fn a_single_backend_is_a_single_tier() {
        let gl: Vec<_> = backend_tiers(wgpu::Backends::GL).collect();
        assert_eq!(gl, [wgpu::Backends::GL]);
        let vulkan: Vec<_> = backend_tiers(wgpu::Backends::VULKAN).collect();
        assert_eq!(vulkan, [wgpu::Backends::VULKAN]);
        assert_eq!(backend_tiers(wgpu::Backends::empty()).count(), 0);
    }

    #[test]
    fn msaa_counts_are_a_set() {
        let counts = MsaaSampleCounts(0b0_0101); // 1 and 4
        assert!(counts.contains(1));
        assert!(!counts.contains(2));
        assert!(counts.contains(4));
        assert!(!counts.contains(3));
        assert_eq!(counts.max(), 4);
        assert_eq!(counts.iter().collect::<Vec<_>>(), [1, 4]);
        assert_eq!(counts.intersect(MsaaSampleCounts(0b0_0011)), MsaaSampleCounts(0b0_0001));
        assert_eq!(MsaaSampleCounts::default().max(), 1);
    }
}
