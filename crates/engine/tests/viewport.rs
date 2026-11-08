//! The engine's [`SceneRenderer`] drawing a game into a viewport target on a device the caller made (#213, ADR 0009):
//! what Studio's viewport panel does with the device it shares with egui-wgpu. The player's graphics settings apply to
//! it as they do to a window, the game is drawn into the target, and a resize draws the new size in the same frame:
//! into the target's top-left corner while it fits, else into a target made larger at once (#330). Skips without an adapter unless `METALLIC_REQUIRE_GPU=1` (CI sets it).

use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, Color3, DataModel, Id, Value};
use metallic_engine::metallic_render::settings::{GraphicsSettings, Preset, Quality};
use metallic_engine::metallic_render::{
    Camera, ContextOptions, RenderContext, RenderError, Renderer, RendererSettings, ViewportTarget, wgpu,
};
use metallic_engine::{FrameStats, SceneRenderer};

/// A device on any adapter, or `None` (after saying why) when the machine has none and `METALLIC_REQUIRE_GPU` is not
/// set.
fn gpu() -> Option<RenderContext> {
    match RenderContext::new_offscreen(&ContextOptions::default()) {
        Ok(context) => Some(context),
        Err(error @ RenderError::NoAdapter { .. }) => {
            if std::env::var("METALLIC_REQUIRE_GPU").is_ok_and(|value| value == "1") {
                panic!("{error}, and METALLIC_REQUIRE_GPU=1");
            }
            eprintln!("skipped: {error}");
            None
        }
        Err(error) => panic!("could not create a device: {error}"),
    }
}

fn service(dm: &DataModel, class: &str) -> Id {
    dm.service(dm.registry().class_id(class).expect("a class"))
        .expect("a service")
}

/// A game whose camera looks straight at a large red block.
fn red_wall() -> DataModel {
    let mut dm = metallic_classes::new_game().expect("a game");
    let workspace = service(&dm, "Workspace");
    let part = dm
        .create(dm.registry().class_id("Part").expect("Part"))
        .expect("a part");
    dm.set_by_name(part, "Size", Value::Vector3(Vec3::new(40.0, 40.0, 1.0)))
        .expect("Size");
    dm.set_by_name(part, "CFrame", Value::CFrame(CFrame::new(Vec3::new(0.0, 0.0, -10.0))))
        .expect("CFrame");
    dm.set_by_name(part, "Color", Value::Color3(Color3::from_rgb_u8(200, 30, 30)))
        .expect("Color");
    dm.set_parent(part, Some(workspace)).expect("parented");
    let Ok(Value::Ref(Some(camera))) = dm.get_by_name(workspace, "CurrentCamera") else {
        panic!("a new game has a CurrentCamera");
    };
    dm.set_by_name(
        camera,
        "CFrame",
        Value::CFrame(CFrame::look_at(
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::new(0.0, 0.0, -10.0),
            Vec3::Y,
        )),
    )
    .expect("camera CFrame");
    dm
}

/// The target's centre pixel, read back as `[r, g, b, a]` (waiting for the GPU).
fn centre(context: &RenderContext, target: &ViewportTarget) -> [u8; 4] {
    let (width, height) = target.size();
    pixel(context, target, (width / 2, height / 2))
}

/// The target's pixel at `(x, y)`, read back as `[r, g, b, a]` (waiting for the GPU).
fn pixel(context: &RenderContext, target: &ViewportTarget, (x, y): (u32, u32)) -> [u8; 4] {
    let texture = target.texture();
    let (width, height) = target.size();
    let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    context.submit(Some(encoder.finish()));
    buffer.map_async(wgpu::MapMode::Read, .., |result| result.expect("map the readback"));
    context
        .device()
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for the readback");
    let data = buffer.get_mapped_range(..).expect("mapped");
    let at = (y * padded + x * 4) as usize;
    [data[at], data[at + 1], data[at + 2], data[at + 3]]
}

fn assert_red(pixel: [u8; 4], when: &str) {
    let [r, g, b, _] = pixel;
    assert!(
        r > g.saturating_add(60) && r > b.saturating_add(60),
        "{when}: the centre is {pixel:?}, not the red wall"
    );
}

#[test]
fn a_scene_renderer_draws_a_game_into_a_viewport_on_the_callers_device() {
    let Some(context) = gpu() else { return };
    let mut dm = red_wall();
    let mut renderer = SceneRenderer::new(ContextOptions::default());
    // Nothing to draw into yet.
    renderer.render_viewport(&mut dm, FrameStats::default());
    assert!(renderer.viewport().is_none() && renderer.renderer().is_none());

    renderer.set_graphics(GraphicsSettings::new(Preset::High));
    renderer.set_frame_cap(Some(60));
    renderer
        .attach_viewport(&context, 160, 120)
        .expect("a viewport on the caller's device");
    // The caller's device, not one of its own.
    let drawn_with = renderer.renderer().expect("a renderer");
    assert_eq!(drawn_with.settings().color_format, ViewportTarget::FORMAT);
    renderer.render_viewport(&mut dm, FrameStats::default());
    // Settings switches still compiling take effect once compiled.
    renderer.renderer_mut().expect("a renderer").wait_for_pipelines();
    renderer.render_viewport(&mut dm, FrameStats::default());
    let viewport = renderer.viewport().expect("a viewport");
    assert_eq!((viewport.size(), viewport.generation()), ((160, 120), 0));
    assert_red(centre(&context, viewport), "the first frames");
    assert!(renderer.first_frame_ms().is_some());

    // The High preset, as a window would draw it.
    let high = Quality::HIGH;
    let drawn_with = renderer.renderer().expect("a renderer");
    assert_eq!(drawn_with.settings().cull, high.cull);
    assert_eq!(drawn_with.settings().shadows, high.shadows);
    assert_eq!(
        drawn_with.anti_aliasing(),
        high.anti_aliasing.supported(drawn_with.msaa_sample_counts())
    );
    assert_eq!(renderer.graphics_status().tier, Some(high.tier));

    // The panel shrinks: the next frame fills the top-left corner of its size, in the same texture (#330).
    renderer.resize_viewport(96, 64);
    renderer.render_viewport(&mut dm, FrameStats::default());
    assert_eq!(renderer.viewport_drawn_size(), Some((96, 64)));
    let viewport = renderer.viewport().expect("a viewport");
    assert_eq!((viewport.size(), viewport.generation()), ((160, 120), 0));
    assert_red(pixel(&context, viewport, (48, 32)), "after the resize");
    // It grows past the texture: a larger one, with room to spare, is drawn into in that same frame.
    renderer.resize_viewport(200, 150);
    renderer.render_viewport(&mut dm, FrameStats::default());
    assert_eq!(renderer.viewport_drawn_size(), Some((200, 150)));
    let viewport = renderer.viewport().expect("a viewport");
    assert_eq!(viewport.generation(), 1);
    assert_eq!(
        viewport.size(),
        ViewportTarget::room_for((200, 150), (160, 120), u32::MAX)
    );
    assert_red(pixel(&context, viewport, (100, 75)), "after growing");
    // A collapsed panel keeps the size it had.
    renderer.resize_viewport(0, 64);
    renderer.render_viewport(&mut dm, FrameStats::default());
    assert_eq!(renderer.viewport_drawn_size(), Some((200, 150)));
    assert!(renderer.perf_mut().summary().frames > 0, "the frames are recorded");
}

/// A renderer made ahead (as Studio's splash makes one) is adopted at its size, with the settings set here.
#[test]
fn a_renderer_made_ahead_draws_the_viewport() {
    let Some(context) = gpu() else { return };
    let mut dm = red_wall();
    let mut renderer = SceneRenderer::new(ContextOptions::default());
    let srgb = Renderer::new(
        &context,
        RendererSettings::new(wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64),
    )
    .expect("a renderer");
    assert!(matches!(
        renderer.attach_viewport_with(&context, srgb),
        Err(RenderError::UnsupportedTarget { .. })
    ));
    assert!(renderer.viewport().is_none());

    let ahead = Renderer::new(&context, RendererSettings::new(ViewportTarget::FORMAT, 128, 96)).expect("a renderer");
    renderer.set_graphics(GraphicsSettings::new(Preset::Low));
    renderer
        .attach_viewport_with(&context, ahead)
        .expect("the renderer is adopted");
    renderer.render_viewport(&mut dm, FrameStats::default());
    renderer.renderer_mut().expect("a renderer").wait_for_pipelines();
    renderer.render_viewport(&mut dm, FrameStats::default());
    let viewport = renderer.viewport().expect("a viewport");
    assert_eq!(viewport.size(), (128, 96));
    assert_red(centre(&context, viewport), "a renderer made ahead");
    let low = Quality::LOW;
    let drawn_with = renderer.renderer().expect("a renderer");
    assert_eq!(drawn_with.settings().cull, low.cull);
    assert_eq!(
        drawn_with.anti_aliasing(),
        low.anti_aliasing.supported(drawn_with.msaa_sample_counts())
    );
}

/// An editor's camera ([`SceneRenderer::set_camera`]) is drawn from instead of the game's, which stays as it was.
#[test]
fn an_editors_camera_replaces_the_games_without_changing_it() {
    let Some(context) = gpu() else { return };
    let mut dm = red_wall();
    let workspace = service(&dm, "Workspace");
    let Ok(Value::Ref(Some(camera))) = dm.get_by_name(workspace, "CurrentCamera") else {
        panic!("a CurrentCamera");
    };
    let before = dm.get_by_name(camera, "CFrame").expect("CFrame");
    let mut renderer = SceneRenderer::new(ContextOptions::default());
    renderer
        .attach_viewport(&context, 64, 64)
        .expect("a viewport on the caller's device");
    // Turned around, away from the wall.
    let away = CFrame::look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 20.0), Vec3::Y);
    renderer.set_camera(Some(Camera::from_cframe(away, 70.0)));
    renderer.render_viewport(&mut dm, FrameStats::default());
    let [r, g, b, _] = centre(&context, renderer.viewport().expect("a viewport"));
    assert!(
        r <= g.saturating_add(60) || r <= b.saturating_add(60),
        "still the wall: {:?}",
        [r, g, b]
    );
    assert_eq!(
        dm.get_by_name(camera, "CFrame").expect("CFrame"),
        before,
        "the game's camera is untouched"
    );

    renderer.set_camera(None);
    renderer.render_viewport(&mut dm, FrameStats::default());
    assert_red(
        centre(&context, renderer.viewport().expect("a viewport")),
        "the game's camera again",
    );
}
