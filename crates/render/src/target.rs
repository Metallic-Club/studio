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
/// [`ViewportTarget::USAGE`], which `egui_wgpu::Renderer::register_native_texture` samples as it samples its own
/// textures: 8-bit `Rgba8Unorm` holding gamma-encoded colour, read without decoding. After a resize,
/// [`ViewportTarget::generation`] changes and the caller registers the new [`ViewportTarget::view`] again
/// (`update_egui_texture_from_wgpu_texture`).
///
/// One renderer draws one target; nothing in the renderer knows of "the" viewport, so several are several renderers
/// (and targets) on the same device.
#[derive(Debug)]
pub struct ViewportTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    generation: u64,
}

impl ViewportTarget {
    /// The texture's format: 8-bit, not an sRGB format, so the finish pass writes gamma-encoded colour itself. That is
    /// what egui-wgpu samples: it keeps its own textures in gamma space, and an sRGB texture, decoded to linear when
    /// read, shows far too dark there. Make the renderer that draws it with this format
    /// ([`crate::RendererSettings::color_format`]).
    pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

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
    }

    fn with_texture(texture: wgpu::Texture, generation: u64) -> Self {
        let view = in_heap_turn(|| texture.create_view(&wgpu::TextureViewDescriptor::default()));
        Self {
            texture,
            view,
            generation,
        }
    }

    /// Takes `texture` (of a new size) in place of the one it had: the frame that switches size draws into it.
    pub(crate) fn replace(&mut self, texture: wgpu::Texture) {
        *self = Self::with_texture(texture, self.generation + 1);
    }

    /// The texture (for copies; the UI samples [`ViewportTarget::view`]).
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// The view to show: register it with the UI renderer, and again whenever [`ViewportTarget::generation`]
    /// changes.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
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

    /// The size a target is kept at to draw a `wanted` panel, from its `current` size (#330): the same while the panel
    /// fits in it (the frame then fills only its top-left corner, [`crate::Renderer::set_drawn_size`]), else room for
    /// the panel with a fifth to spare on the side that grew, in whole 64-pixel steps, so a panel dragged larger needs
    /// a new texture only now and then. It never shrinks, and a side is never past `limit` (the device's largest
    /// texture). The UI that shows the target computes the same, so it knows this frame's corner before it is drawn.
    pub fn room_for(wanted: (u32, u32), current: (u32, u32), limit: u32) -> (u32, u32) {
        let side = |want: u32, have: u32| {
            if want <= have {
                have
            } else {
                (want + want / 5).div_ceil(ROOM_STEP) * ROOM_STEP
            }
            .min(limit)
            .max(1)
        };
        (side(wanted.0, current.0), side(wanted.1, current.1))
    }

    /// Makes the texture `width` x `height` now (#330), in place of the one it had: a viewport that grows past its
    /// target draws at its new size in the same frame. Callers resize the renderer that draws it to the same size.
    pub fn resize_now(&mut self, context: &RenderContext, width: u32, height: u32) -> Result<(), RenderError> {
        check_size(context, width, height)?;
        let texture = Self::spec(self.format(), width, height).make_now(context.device());
        self.replace(texture);
        Ok(())
    }
}

/// The steps a viewport target grows in ([`ViewportTarget::room_for`]).
const ROOM_STEP: u32 = 64;

/// Rejects sizes wgpu would refuse (zero, or above the device's texture limit) before they reach it.
pub(crate) fn check_size(context: &RenderContext, width: u32, height: u32) -> Result<(), RenderError> {
    let max = context.capabilities().max_texture_size;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(RenderError::InvalidSize { width, height });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ViewportTarget;

    #[test]
    fn a_target_keeps_room_and_grows_by_a_fifth_in_steps() {
        // The panel fits: the target stays as it is, larger in both or either.
        assert_eq!(ViewportTarget::room_for((800, 600), (800, 600), 8192), (800, 600));
        assert_eq!(ViewportTarget::room_for((640, 480), (1024, 768), 8192), (1024, 768));
        // Wider than it: a fifth more, in 64-pixel steps, on that side only.
        assert_eq!(ViewportTarget::room_for((1000, 600), (800, 600), 8192), (1216, 600));
        assert_eq!(ViewportTarget::room_for((1000, 900), (800, 600), 8192), (1216, 1088));
        // Never past the device's limit, and never zero.
        assert_eq!(ViewportTarget::room_for((8000, 10), (100, 10), 8192), (8192, 10));
        assert_eq!(ViewportTarget::room_for((1, 1), (0, 0), 8192), (64, 64));
    }
}
