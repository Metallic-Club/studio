//! An offscreen colour target: what the renderer draws into when there is no window (tests, CI, thumbnails).

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

/// Rejects sizes wgpu would refuse (zero, or above the device's texture limit) before they reach it.
pub(crate) fn check_size(context: &RenderContext, width: u32, height: u32) -> Result<(), RenderError> {
    let max = context.capabilities().max_texture_size;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(RenderError::InvalidSize { width, height });
    }
    Ok(())
}
