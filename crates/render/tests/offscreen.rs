//! Offscreen rendering on a real adapter: clear colours read back, every pass layout, MSAA, resizing, and device
//! setup from many threads at once. Skips when the machine has no adapter (see `support::gpu`).
//!
//! The clear colour is linear light. A frame with the finish pass ([`PassId::Post`], the default) clears the HDR
//! scene colour with it and finishes it like any other light, so an 8-bit UNORM target gets it sRGB-encoded (the
//! finish pass encodes for targets that do not). A frame without it clears the target itself, which stores the
//! values as they are, as before the post chain.

mod support;

use metallic_render::{
    ContextOptions, FramePlan, OffscreenTarget, PassId, PassSet, RenderContext, RenderError, Renderer,
    RendererSettings, wgpu,
};
use support::{assert_all_pixels, gpu, gpu_with, read_back_rgba8};

const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.25,
    g: 0.5,
    b: 0.75,
    a: 1.0,
};
/// `CLEAR` stored as it is in an 8-bit UNORM target.
const CLEAR_UNORM: [u8; 4] = [64, 128, 191, 255];
/// `CLEAR` encoded with the sRGB curve: in an sRGB target, or finished into a UNORM one.
const CLEAR_ENCODED: [u8; 4] = [137, 188, 225, 255];

/// What `CLEAR` reads back as from a UNORM target drawn with `passes`.
fn clear_in_unorm(passes: PassSet) -> [u8; 4] {
    if FramePlan::new(passes, 1).post() {
        CLEAR_ENCODED
    } else {
        CLEAR_UNORM
    }
}

fn render_once(context: &RenderContext, settings: RendererSettings) -> Vec<[u8; 4]> {
    let target = OffscreenTarget::new(context, settings.width, settings.height, settings.color_format)
        .expect("an offscreen target");
    let mut renderer = Renderer::new(context, settings).expect("a renderer");
    renderer.render(target.view());
    read_back_rgba8(context, target.texture())
}

fn settings(format: wgpu::TextureFormat) -> RendererSettings {
    RendererSettings {
        clear_color: CLEAR,
        ..RendererSettings::new(format, 64, 48)
    }
}

#[test]
fn clears_to_a_colour_and_reads_it_back() {
    let Some(context) = gpu() else { return };
    let caps = context.capabilities();
    eprintln!(
        "adapter: {} ({:?}, {:?}, {})",
        caps.adapter_name, caps.backend, caps.device_type, caps.driver
    );
    let pixels = render_once(&context, settings(wgpu::TextureFormat::Rgba8Unorm));
    assert_eq!(pixels.len(), 64 * 48);
    assert_all_pixels(&pixels, CLEAR_ENCODED, 1);
    // Without the finish pass the target is cleared directly.
    let direct = PassSet::default().without(PassId::Post);
    let pixels = render_once(
        &context,
        RendererSettings {
            passes: direct,
            ..settings(wgpu::TextureFormat::Rgba8Unorm)
        },
    );
    assert_all_pixels(&pixels, CLEAR_UNORM, 1);
}

#[test]
fn srgb_targets_encode_the_linear_clear_colour() {
    let Some(context) = gpu() else { return };
    let pixels = render_once(&context, settings(wgpu::TextureFormat::Rgba8UnormSrgb));
    // Linear 0.25 / 0.5 / 0.75 encoded with the sRGB curve.
    assert_all_pixels(&pixels, CLEAR_ENCODED, 1);
    let pixels = render_once(&context, settings(wgpu::TextureFormat::Bgra8UnormSrgb));
    assert_all_pixels(&pixels, CLEAR_ENCODED, 1);
}

#[test]
fn every_pass_layout_produces_the_clear_colour() {
    let Some(context) = gpu() else { return };
    let layouts = [
        PassSet::default(),
        PassSet::ALL,
        PassSet::default().with(PassId::DepthPrepass),
        PassSet::default().with(PassId::Mlaa),
        PassSet::default().with(PassId::Scene),
        PassSet::default().without(PassId::Bloom),
        PassSet::default().without(PassId::Post),
        PassSet::EMPTY.with(PassId::Opaque),
        PassSet::EMPTY.with(PassId::Sky).with(PassId::Transparent),
        PassSet::EMPTY.with(PassId::Ui),
    ];
    for passes in layouts {
        let pixels = render_once(
            &context,
            RendererSettings {
                passes,
                ..settings(wgpu::TextureFormat::Rgba8Unorm)
            },
        );
        assert_all_pixels(&pixels, clear_in_unorm(passes), 1);
    }
}

#[test]
fn msaa_resolves_into_the_target() {
    let Some(context) = gpu() else { return };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let counts = context
        .msaa_sample_counts(format)
        .intersect(context.msaa_sample_counts(metallic_render::post::hdr_format(&context)))
        .intersect(context.msaa_sample_counts(metallic_render::depth::DEPTH_FORMAT));
    for samples in counts.iter().filter(|&samples| samples > 1) {
        for passes in [
            PassSet::default(),
            PassSet::default().with(PassId::Scene),
            PassSet::default().without(PassId::Post),
            PassSet::EMPTY.with(PassId::Opaque),
        ] {
            let settings = RendererSettings {
                msaa_samples: samples,
                passes,
                ..settings(format)
            };
            let renderer = Renderer::new(&context, settings).expect("a renderer");
            assert_eq!(renderer.msaa_sample_counts(), counts);
            let pixels = render_once(&context, settings);
            assert_all_pixels(&pixels, clear_in_unorm(passes), 1);
        }
    }
}

#[test]
fn unsupported_msaa_and_sizes_are_errors() {
    let Some(context) = gpu() else { return };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let three = Renderer::new(
        &context,
        RendererSettings {
            msaa_samples: 3,
            ..settings(format)
        },
    );
    assert!(matches!(three, Err(RenderError::UnsupportedTarget { samples: 3, .. })));
    let empty = Renderer::new(&context, RendererSettings::new(format, 0, 10));
    assert!(matches!(empty, Err(RenderError::InvalidSize { width: 0, .. })));
    assert!(OffscreenTarget::new(&context, 10, 0, format).is_err());
    let mut renderer = Renderer::new(&context, settings(format)).unwrap();
    assert!(renderer.resize(0, 0).is_err());
    assert!(renderer.set_msaa_samples(3).is_err());
}

#[test]
fn resizing_recreates_the_attachments() {
    let Some(context) = gpu() else { return };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = Renderer::new(&context, settings(format)).unwrap();
    let small = OffscreenTarget::new(&context, 64, 48, format).unwrap();
    renderer.render(small.view());
    renderer.resize(200, 120).unwrap();
    let large = OffscreenTarget::new(&context, 200, 120, format).unwrap();
    renderer.set_clear_color(wgpu::Color::RED);
    renderer.render(large.view());
    // Full red through the tone curve, encoded.
    let red = metallic_render::post::tone_curve(1.0);
    let red = (1.055 * red.powf(1.0 / 2.4) - 0.055) * 255.0;
    assert_all_pixels(
        &read_back_rgba8(&context, large.texture()),
        [red.round() as u8, 0, 0, 255],
        1,
    );
    assert_eq!(renderer.frame_index(), 2);
}

#[test]
fn capabilities_are_recorded() {
    let Some(context) = gpu() else { return };
    let caps = context.capabilities();
    assert!(!caps.adapter_name.is_empty());
    assert!(
        caps.max_texture_size >= 2048,
        "max texture size {}",
        caps.max_texture_size
    );
    assert!(caps.msaa.contains(1));
    if caps.backend != wgpu::Backend::Gl {
        assert!(caps.compute && caps.indirect_draw, "{caps:?}");
    }
}

#[test]
fn primary_backends_are_preferred_over_gl() {
    // Machines with a Vulkan, Metal or DX12 adapter never fall back to GL.
    let primary = ContextOptions {
        backends: wgpu::Backends::PRIMARY,
        ..ContextOptions::default()
    };
    if RenderContext::new_offscreen(&primary).is_err() {
        return;
    }
    let Some(context) = gpu_with(&ContextOptions {
        backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
        ..ContextOptions::default()
    }) else {
        return;
    };
    assert_ne!(context.capabilities().backend, wgpu::Backend::Gl);
}

/// The archived renderer crashed (heap corruption) when several threads set up GL at once; setup is serialised.
#[test]
fn devices_set_up_on_many_threads_at_once() {
    for backends in [wgpu::Backends::PRIMARY, wgpu::Backends::GL] {
        let options = ContextOptions {
            backends,
            ..ContextOptions::default()
        };
        if RenderContext::new_offscreen(&options).is_err() {
            eprintln!("skipped {backends:?}: no adapter");
            continue;
        }
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let options = options.clone();
                std::thread::spawn(move || {
                    for _ in 0..3 {
                        let context = RenderContext::new_offscreen(&options).unwrap();
                        let pixels = render_once(&context, settings(wgpu::TextureFormat::Rgba8Unorm));
                        assert_all_pixels(&pixels, CLEAR_ENCODED, 1);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
