//! Colour targets that are not a window: an offscreen target (tests, CI, thumbnails) and a viewport target (a scene
//! shown inside an editor's UI).

use crate::context::in_heap_turn;
use crate::textures::TextureSpec;
use crate::{RenderContext, RenderError};

/// A texture the renderer can draw into and that can be copied out or sampled afterwards.
#[derive(Debug)]
pub struct OffscreenTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl OffscreenTarget {
    /// Creates a `width` × `height` target in `format`.
    pub fn new(
        context: &RenderContext,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<Self, RenderError> {
        check_size(context, width, height)?;
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING;
        if !context.format_features(format).allowed_usages.contains(usage) {
            return Err(RenderError::UnsupportedTarget { format, samples: 1 });
        }
        let turn = context.descriptor_heap_turn();
        let texture = context.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("metallic offscreen target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        drop(turn);
        Ok(Self { texture, view })
    }

    /// The texture (for copies and sampling).
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// The view to render into.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The colour format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.texture.format()
    }

    /// The size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }
}

/// The colour texture a scene is drawn into for a caller to show in its own UI: Studio's viewport panel, drawn as an
/// image by `egui-wgpu` on the same device (ADR 0009, #213).
///
/// [`Renderer::render_viewport`](crate::Renderer::render_viewport) draws a scene into it through the normal frame (the
/// HDR scene colour, the post chain, the finish pass), and
/// [`Renderer::resize_viewport`](crate::Renderer::resize_viewport) resizes it without a black frame: the old texture
/// is drawn into and shown until the new one exists. Its texture is [`ViewportTarget::FORMAT`] with
/// [`ViewportTarget::USAGE`]. The UI shows it through [`ViewportTarget::ui_view`]: the same bytes read as `Rgba8Unorm`,
/// undecoded, which is how `egui_wgpu::Renderer::register_native_texture` expects a texture (egui-wgpu keeps its own in
/// gamma space and would otherwise show the scene too dark). After a resize, [`ViewportTarget::generation`] changes
/// and the caller registers the new [`ViewportTarget::ui_view`] again (`update_egui_texture_from_wgpu_texture`).
///
/// One renderer draws one target; nothing in the renderer knows of "the" viewport, so several are several renderers
/// (and targets) on the same device.
#[derive(Debug)]
pub struct ViewportTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    ui_view: wgpu::TextureView,
    generation: u64,
}

impl ViewportTarget {
    /// The texture's format: 8-bit sRGB, the renderer writing it encoded. Make the renderer that draws it with this
    /// format ([`crate::RendererSettings::color_format`]).
    pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    /// The texture's usages: drawn into, sampled by the UI, and copied out (screenshots, thumbnails, tests).
    pub const USAGE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
        .union(wgpu::TextureUsages::TEXTURE_BINDING)
        .union(wgpu::TextureUsages::COPY_SRC);

    /// A `width` x `height` target (physical pixels), made now. Its renderer is made at the same size.
    pub fn new(context: &RenderContext, width: u32, height: u32) -> Result<Self, RenderError> {
        check_size(context, width, height)?;
        let texture = Self::spec(Self::FORMAT, width, height).make_now(context.device());
        Ok(Self::with_texture(texture, 0))
    }

    /// What a target's texture of `width` x `height` in `format` is made from.
    pub(crate) fn spec(format: wgpu::TextureFormat, width: u32, height: u32) -> TextureSpec {
        TextureSpec::new("metallic viewport target", (width, height, 1), format, Self::USAGE)
            .viewed_as(format.remove_srgb_suffix())
    }

    fn with_texture(texture: wgpu::Texture, generation: u64) -> Self {
        let (view, ui_view) = in_heap_turn(|| {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let ui_view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("metallic viewport target (ui)"),
                format: Some(texture.format().remove_srgb_suffix()),
                ..Default::default()
            });
            (view, ui_view)
        });
        Self {
            texture,
            view,
            ui_view,
            generation,
        }
    }

    /// Takes `texture` (of a new size) in place of the one it had: the frame that switches size draws into it.
    pub(crate) fn replace(&mut self, texture: wgpu::Texture) {
        *self = Self::with_texture(texture, self.generation + 1);
    }

    /// The texture (for copies; the UI samples [`ViewportTarget::ui_view`]).
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// The view the renderer draws into, in [`ViewportTarget::FORMAT`].
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The view to show: the texture's bytes as `Rgba8Unorm`, undecoded, as egui-wgpu samples its own textures.
    /// Register it with the UI renderer, and again whenever [`ViewportTarget::generation`] changes.
    pub fn ui_view(&self) -> &wgpu::TextureView {
        &self.ui_view
    }

    /// The colour format ([`ViewportTarget::FORMAT`]).
    pub fn format(&self) -> wgpu::TextureFormat {
        self.texture.format()
    }

    /// The size in pixels of the texture shown, which during a resize is still the old one.
    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }

    /// How many times the texture was replaced by a resize: when it changed since the caller last looked, the
    /// caller's handle to the old view (a registered UI texture) is stale.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// Rejects sizes wgpu would refuse (zero, or above the device's texture limit) before they reach it.
pub(crate) fn check_size(context: &RenderContext, width: u32, height: u32) -> Result<(), RenderError> {
    let max = context.capabilities().max_texture_size;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(RenderError::InvalidSize { width, height });
    }
    Ok(())
}
