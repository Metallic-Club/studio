//! A window's swapchain: configuration, resizing, and acquiring images without panicking when the window is
//! minimised, resized or lost.

use std::fmt;
use std::sync::Arc;

use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle};

use crate::context::in_heap_turn;
use crate::{RenderContext, RenderError};

/// A window the renderer can present to. Blanket-implemented, so any `HasWindowHandle + HasDisplayHandle` window
/// type (winit's, or a console platform's) works without this crate knowing it.
pub(crate) trait SurfaceWindow: HasWindowHandle + HasDisplayHandle + Send + Sync {}

impl<T: HasWindowHandle + HasDisplayHandle + Send + Sync + ?Sized> SurfaceWindow for T {}

/// The display connection of a window, handed to a GL instance (wgpu wants it `Debug` and boxed).
pub(crate) struct WindowDisplay(pub(crate) Arc<dyn SurfaceWindow>);

impl fmt::Debug for WindowDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WindowDisplay")
    }
}

impl HasDisplayHandle for WindowDisplay {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}

/// A window's surface (swapchain) and its configuration.
///
/// Presents with `Fifo` (vsync) and at most two frames queued by default (ADR 0008 §9). A zero-sized window
/// (minimised) is never configured; [`RenderSurface::acquire`] then returns `None` and the frame is skipped.
pub struct RenderSurface {
    surface: wgpu::Surface<'static>,
    window: Arc<dyn SurfaceWindow>,
    config: wgpu::SurfaceConfiguration,
    /// Whether `config` is applied to the surface. False while the window has no area.
    configured: bool,
    /// The last image was suboptimal; configure again before the next acquire.
    reconfigure: bool,
}

impl fmt::Debug for RenderSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderSurface")
            .field("config", &self.config)
            .field("configured", &self.configured)
            .finish_non_exhaustive()
    }
}

impl RenderSurface {
    pub(crate) fn new(
        context: &RenderContext,
        surface: wgpu::Surface<'static>,
        window: Arc<dyn SurfaceWindow>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let caps = surface.get_capabilities(context.adapter());
        // An sRGB format, so shaders write linear colour and the hardware encodes it.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| caps.formats.first().copied())
            .ok_or(RenderError::SurfaceUnsupported)?;
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            // SDR sRGB; HDR output is a later, opt-in setting.
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: Vec::new(),
        };
        let mut surface = Self {
            surface,
            window,
            config,
            configured: false,
            reconfigure: false,
        };
        surface.configure(context);
        Ok(surface)
    }

    /// The colour format of the images; the renderer's target format must match it.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// The configured size in pixels (`0 × 0` while the window is minimised).
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// The current present mode.
    pub fn present_mode(&self) -> wgpu::PresentMode {
        self.config.present_mode
    }

    /// The configuration in use.
    pub fn config(&self) -> &wgpu::SurfaceConfiguration {
        &self.config
    }

    /// Switches the present mode (for example `AutoNoVsync` for an uncapped frame rate). A mode the surface does not
    /// support falls back to `Fifo`, which every surface has. Returns the mode now in use.
    pub fn set_present_mode(&mut self, context: &RenderContext, mode: wgpu::PresentMode) -> wgpu::PresentMode {
        let supported = matches!(
            mode,
            wgpu::PresentMode::AutoVsync | wgpu::PresentMode::AutoNoVsync | wgpu::PresentMode::Fifo
        ) || self
            .surface
            .get_capabilities(context.adapter())
            .present_modes
            .contains(&mode);
        let mode = if supported { mode } else { wgpu::PresentMode::Fifo };
        if mode != self.config.present_mode {
            self.config.present_mode = mode;
            self.configure(context);
        }
        mode
    }

    /// The alpha mode the window composites the images with: opaque by default.
    pub fn alpha_mode(&self) -> wgpu::CompositeAlphaMode {
        self.config.alpha_mode
    }

    /// Asks for an alpha mode, for a window whose clear colour should show through where it is transparent (Studio's
    /// splash, with rounded corners): `PreMultiplied` or `PostMultiplied` where the platform composites them. A mode
    /// the surface does not support leaves the one in use; Direct3D 12 window swapchains, for example, are always
    /// opaque. Returns the mode now in use.
    pub fn set_alpha_mode(
        &mut self,
        context: &RenderContext,
        mode: wgpu::CompositeAlphaMode,
    ) -> wgpu::CompositeAlphaMode {
        let supported = mode == wgpu::CompositeAlphaMode::Auto
            || self
                .surface
                .get_capabilities(context.adapter())
                .alpha_modes
                .contains(&mode);
        if supported && mode != self.config.alpha_mode {
            self.config.alpha_mode = mode;
            self.configure(context);
        }
        self.config.alpha_mode
    }

    /// Applies a new window size in pixels. A zero width or height leaves the surface unconfigured until the window
    /// has an area again.
    pub fn resize(&mut self, context: &RenderContext, width: u32, height: u32) {
        if (width, height) == (self.config.width, self.config.height) && self.configured {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.configure(context);
    }

    /// Gets the next image to draw into.
    ///
    /// Returns `Ok(None)` when this frame should be skipped: the window is minimised or occluded, or acquiring
    /// timed out. An outdated surface is configured again and a lost one created again from the window, then
    /// acquired once more, so callers never handle those cases themselves.
    pub fn acquire(&mut self, context: &RenderContext) -> Result<Option<SurfaceFrame>, RenderError> {
        if self.reconfigure {
            self.reconfigure = false;
            self.configure(context);
        }
        if !self.configured {
            return Ok(None);
        }
        for attempt in 0..2 {
            // wgpu makes a view of the image for clearing it.
            match in_heap_turn(|| self.surface.get_current_texture()) {
                wgpu::CurrentSurfaceTexture::Success(texture) => return Ok(Some(SurfaceFrame::new(texture))),
                wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                    // Still presentable; the surface is fixed before the next frame (configuring now would panic
                    // while `texture` is alive).
                    self.reconfigure = true;
                    return Ok(Some(SurfaceFrame::new(texture)));
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Ok(None),
                wgpu::CurrentSurfaceTexture::Outdated => self.configure(context),
                wgpu::CurrentSurfaceTexture::Lost => {
                    if attempt == 0 {
                        self.surface = context.create_raw_surface(&self.window)?;
                    }
                    self.configure(context);
                }
                wgpu::CurrentSurfaceTexture::Validation => return Err(RenderError::SurfaceValidation),
            }
        }
        Ok(None)
    }

    fn configure(&mut self, context: &RenderContext) {
        let max = context.capabilities().max_texture_size;
        // Larger than the device allows happens transiently while a window is dragged across monitors.
        self.config.width = self.config.width.min(max);
        self.config.height = self.config.height.min(max);
        self.configured = self.config.width > 0 && self.config.height > 0;
        if self.configured {
            self.surface.configure(context.device(), &self.config);
        }
    }
}

/// One acquired swapchain image. Render into [`SurfaceFrame::view`], then call [`SurfaceFrame::present`].
#[derive(Debug)]
pub struct SurfaceFrame {
    texture: wgpu::SurfaceTexture,
    view: wgpu::TextureView,
}

impl SurfaceFrame {
    fn new(texture: wgpu::SurfaceTexture) -> Self {
        let view = in_heap_turn(|| texture.texture.create_view(&wgpu::TextureViewDescriptor::default()));
        Self { texture, view }
    }

    /// The view to render into.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The image's texture.
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture.texture
    }

    /// Queues the image for presentation on `context`'s queue. Submit the frame's commands first.
    pub fn present(self, context: &RenderContext) {
        drop(self.view);
        context.queue().present(self.texture);
    }
}
