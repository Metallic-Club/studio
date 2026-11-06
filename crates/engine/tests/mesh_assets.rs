//! A project's glTF mesh and PNG texture drawn on a real adapter (#189): a `.mtproj` with a textured cube model and
//! a texture, mesh parts naming them in its start scene, loaded off the main thread by [`MeshAssets`] and drawn
//! offscreen against a committed reference image (`tests/reference/mesh_asset.png`), on both instance paths, with
//! their shadows; a missing asset is a warning and draws nothing.
//!
//! The model and the texture are the ones `metallic_project`'s tests make and check (`crates/project/tests/assets.rs`
//! says how; made for this repository). Images are compared with a tolerance, as the renderer's own image tests do; run
//! with `METALLIC_UPDATE_REFERENCES=1` to write a new reference after a deliberate change, and look at it. Skips
//! without an adapter unless `METALLIC_REQUIRE_GPU=1` (CI sets it).

use std::path::{Path, PathBuf};
use std::time::Duration;

use metallic_engine::MeshAssets;
use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, Color3, DataModel, Id, Value};
use metallic_engine::metallic_project::{Project, SceneIds, Template, decode_image, save_scene};
use metallic_engine::metallic_render::scene::{InstanceBinding, RenderComponents};
use metallic_engine::metallic_render::{
    CameraReader, ContextOptions, DrawStats, GpuScene, LightingReader, OffscreenTarget, RenderContext, RenderError,
    Renderer, RendererSettings, wgpu,
};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const MODEL: &str = "Assets/Models/textured_cube.glb";
const TEXTURE: &str = "Assets/Textures/quadrants.png";
/// An id no asset has: a part naming it is reported and draws nothing.
const MISSING: &str = "0123456789abcdef0123456789abcdef";

/// A pixel differing by more than this in a channel counts as different.
const THRESHOLD: u8 = 12;
/// At most this share of pixels may differ (triangle edges rasterised a little differently).
const MAX_OVER: f64 = 0.01;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../project/tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A device on any adapter, or `None` (after saying why) when the machine has none and `METALLIC_REQUIRE_GPU` is not
/// set.
fn gpu_with(options: &ContextOptions) -> Option<RenderContext> {
    match RenderContext::new_offscreen(options) {
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

fn gpu() -> Option<RenderContext> {
    gpu_with(&ContextOptions::default())
}

fn service(dm: &DataModel, class: &str) -> Id {
    dm.service(dm.registry().class_id(class).expect("mesh asset test setup"))
        .expect("mesh asset test setup")
}

fn new_in(dm: &mut DataModel, class: &str, parent: Id, name: &str) -> Id {
    let id = dm
        .create(dm.registry().class_id(class).expect("mesh asset test setup"))
        .expect("mesh asset test setup");
    dm.set_name(id, name.into()).expect("mesh asset test setup");
    dm.set_parent(id, Some(parent)).expect("mesh asset test setup");
    id
}

fn mesh_part(dm: &mut DataModel, name: &str, (mesh, texture): (&str, &str), size: f32, cframe: CFrame) -> Id {
    let workspace = service(dm, "Workspace");
    let part = new_in(dm, "MeshPart", workspace, name);
    dm.set_by_name(part, "MeshId", Value::String(mesh.into()))
        .expect("mesh asset test setup");
    dm.set_by_name(part, "TextureId", Value::String(texture.into()))
        .expect("mesh asset test setup");
    dm.set_by_name(part, "Size", Value::Vector3(Vec3::splat(size)))
        .expect("mesh asset test setup");
    dm.set_by_name(part, "CFrame", Value::CFrame(cframe))
        .expect("mesh asset test setup");
    dm.set_by_name(part, "Color", Value::Color3(Color3::WHITE))
        .expect("mesh asset test setup");
    part
}

/// A project in a fresh folder for `test`: the model and the texture as assets, and a start scene with a baseplate,
/// the textured cube, the same cube untextured and red, and a part whose mesh is missing, seen by the camera from
/// above and to the side. With `textured` false the textured cube names no texture. Saved, then opened again, so the
/// game is what the file holds.
fn project(test: &str, textured: bool) -> Project {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mesh_assets").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mesh asset test setup");
    let template = Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d"))
        .expect("mesh asset test setup");
    let path = dir.join("Game.mtproj");
    let mut project = Project::create(&path, &template, "Meshes").expect("mesh asset test setup");
    project
        .write(MODEL, fixture("textured_cube.glb"))
        .expect("mesh asset test setup");
    project
        .write(TEXTURE, fixture("quadrants.png"))
        .expect("mesh asset test setup");
    project.refresh_assets().expect("mesh asset test setup");
    let model = project.assets().id(MODEL).expect("mesh asset test setup").to_string();
    let texture = project.assets().id(TEXTURE).expect("mesh asset test setup").to_string();

    let mut dm = metallic_classes::new_game().expect("mesh asset test setup");
    let workspace = service(&dm, "Workspace");
    let base = new_in(&mut dm, "Part", workspace, "Baseplate");
    dm.set_by_name(base, "Size", Value::Vector3(Vec3::new(40.0, 1.0, 40.0)))
        .expect("mesh asset test setup");
    dm.set_by_name(base, "CFrame", Value::CFrame(CFrame::new(Vec3::new(0.0, -0.5, 0.0))))
        .expect("mesh asset test setup");
    dm.set_by_name(base, "Color", Value::Color3(Color3::from_rgb_u8(160, 165, 170)))
        .expect("mesh asset test setup");
    let turned = |x: f32, z: f32, size: f32, yaw: f32| {
        CFrame::from_parts(
            Vec3::new(x, size / 2.0, z),
            CFrame::from_orientation_deg(0.0, yaw, 0.0).rotation(),
        )
    };
    let skin = if textured { texture.as_str() } else { "" };
    mesh_part(&mut dm, "Crate", (&model, skin), 4.0, turned(0.0, 0.0, 4.0, 30.0));
    let plain = mesh_part(&mut dm, "Plain", (&model, ""), 2.0, turned(5.0, 3.0, 2.0, -15.0));
    dm.set_by_name(plain, "Color", Value::Color3(Color3::from_rgb_u8(200, 60, 50)))
        .expect("mesh asset test setup");
    mesh_part(&mut dm, "Lost", (MISSING, ""), 2.0, turned(-5.0, 3.0, 2.0, 0.0));
    let camera = match dm.get_by_name(workspace, "CurrentCamera") {
        Ok(Value::Ref(Some(camera))) => camera,
        other => panic!("CurrentCamera: {other:?}"),
    };
    let eye = Vec3::new(9.0, 8.0, 11.0);
    dm.set_by_name(
        camera,
        "CFrame",
        Value::CFrame(CFrame::look_at(eye, Vec3::new(1.0, 1.5, 0.0), Vec3::Y)),
    )
    .expect("mesh asset test setup");
    let scene = save_scene(&dm, &mut SceneIds::new()).expect("mesh asset test setup");
    assert_eq!(scene.warnings, []);
    let start = project.manifest().start_scene.clone();
    project.write(&start, scene.text).expect("mesh asset test setup");
    project.save().expect("mesh asset test setup");
    drop(project);
    Project::open(&path).expect("mesh asset test setup")
}

/// What a frame drew, and what the assets reported.
struct Drawn {
    pixels: Vec<[u8; 4]>,
    /// The last frame's.
    stats: DrawStats,
    /// The most shadow draws a frame made: the cascades are cached, so later frames draw none.
    shadow_draws: u32,
    warnings: Vec<String>,
}

/// `project`'s start scene drawn on `context` at `width` × `height`: a frame (which asks for the assets), the assets
/// waited for, then two more frames so everything is uploaded.
fn draw(context: &RenderContext, project: &mut Project, (width, height): (u32, u32)) -> Drawn {
    let mut dm = metallic_classes::new_game().expect("mesh asset test setup");
    let loaded = project.load_start_scene(&mut dm).expect("mesh asset test setup");
    assert_eq!(loaded.warnings, [], "the mesh parts load from the file");
    let workspace = service(&dm, "Workspace");
    let lighting = service(&dm, "Lighting");
    let target = OffscreenTarget::new(context, width, height, FORMAT).expect("mesh asset test setup");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            gpu_timing: false,
            ..RendererSettings::new(FORMAT, width, height)
        },
    )
    .expect("mesh asset test setup");
    renderer.wait_for_pipelines();
    renderer.set_lighting(LightingReader::new().read(&dm, lighting));
    let mut scene =
        GpuScene::new(context, &mut dm, Some(workspace), &RenderComponents::builtin()).expect("mesh asset test setup");
    let mut assets = MeshAssets::new(project.asset_reader());
    let mut reader = CameraReader::new();
    let mut shadow_draws = 0;
    for frame in 0..3 {
        let camera = reader.read(&dm, workspace);
        if let Some(camera) = camera {
            scene.set_camera_position(&dm, camera.position);
        }
        scene.sync(&mut dm);
        if frame == 0 {
            assert!(
                assets.wait(scene.scene(), &mut renderer, Duration::from_secs(60)),
                "the assets load"
            );
        } else {
            assets.update(scene.scene(), &mut renderer);
        }
        renderer.render_scene(target.view(), &mut scene, camera.as_ref());
        shadow_draws = shadow_draws.max(renderer.draw_stats().shadow_draws);
    }
    Drawn {
        pixels: read_back(context, target.texture()),
        stats: renderer.draw_stats(),
        shadow_draws,
        warnings: assets.warnings().to_vec(),
    }
}

/// Copies `texture` (8-bit RGBA, `COPY_SRC`) back to the CPU, waiting for the GPU.
fn read_back(context: &RenderContext, texture: &wgpu::Texture) -> Vec<[u8; 4]> {
    let (width, height) = (texture.width(), texture.height());
    let unpadded = width * 4;
    let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
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
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for row in data.chunks_exact(padded as usize) {
        pixels.extend(row[..unpadded as usize].as_chunks::<4>().0);
    }
    pixels
}

/// The largest channel difference of each pixel: (largest, mean, share over [`THRESHOLD`]).
fn compare(a: &[[u8; 4]], b: &[[u8; 4]]) -> (u8, f64, f64) {
    assert_eq!(a.len(), b.len());
    let mut max = 0u8;
    let (mut sum, mut over) = (0u64, 0usize);
    for (pa, pb) in a.iter().zip(b) {
        let d = pa.iter().zip(pb).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        max = max.max(d);
        sum += u64::from(d);
        over += usize::from(d > THRESHOLD);
    }
    let n = a.len().max(1) as f64;
    (max, sum as f64 / n, over as f64 / n)
}

fn write_png(path: &Path, (width, height): (u32, u32), pixels: &[[u8; 4]]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mesh asset test setup");
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path).expect("mesh asset test setup"));
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header().expect("mesh asset test setup");
    writer
        .write_image_data(pixels.as_flattened())
        .expect("mesh asset test setup");
}

fn reference_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/mesh_asset.png")
}

/// The mean difference an image may have from the reference: the references are drawn through the `Rg11b10Ufloat`
/// scene colour, which rounds a little darker than the `Rgba16Float` of devices without it (as in
/// `metallic_render`'s image tests).
fn max_mean(context: &RenderContext) -> f64 {
    if metallic_engine::metallic_render::post::hdr_format(context) == wgpu::TextureFormat::Rg11b10Ufloat {
        1.0
    } else {
        1.5
    }
}

/// `pixels` against the committed reference, or written as it when asked to.
fn check_reference(context: &RenderContext, name: &str, pixels: &[[u8; 4]]) {
    let path = reference_path();
    if std::env::var("METALLIC_UPDATE_REFERENCES").is_ok_and(|v| v == "1") {
        write_png(&path, (SIZE, SIZE), pixels);
        eprintln!("wrote {}", path.display());
        return;
    }
    let reference = decode_image(&std::fs::read(&path).expect("the reference image")).expect("mesh asset test setup");
    assert_eq!((reference.width, reference.height), (SIZE, SIZE));
    let reference: Vec<[u8; 4]> = reference.rgba.as_chunks::<4>().0.to_vec();
    let (max, mean, over) = compare(pixels, &reference);
    eprintln!(
        "{name}: max difference {max}, mean {mean:.3}, {:.3}% of pixels over {THRESHOLD}",
        over * 100.0
    );
    if over > MAX_OVER || mean > max_mean(context) {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        write_png(&out, (SIZE, SIZE), pixels);
        panic!(
            "{name} differs from the reference beyond the tolerance; the image is in {}",
            out.display()
        );
    }
}

#[test]
fn a_projects_mesh_and_texture_draw_like_the_reference() {
    let Some(context) = gpu() else { return };
    let caps = context.capabilities();
    eprintln!(
        "adapter: {} ({:?}), instances: {:?}",
        caps.adapter_name,
        caps.backend,
        InstanceBinding::for_context(&context)
    );
    let mut project = project("reference", true);
    let drawn = draw(&context, &mut project, (SIZE, SIZE));
    eprintln!("{:?}", drawn.stats);
    // The baseplate, then one draw per mesh kind whose mesh is here: the textured cube and the red one. The part
    // whose mesh is missing draws nothing, and is reported.
    assert_eq!(drawn.stats.opaque_draws, 3);
    assert_eq!(drawn.stats.opaque_instances, 3);
    assert_eq!(drawn.warnings.len(), 1, "{:?}", drawn.warnings);
    assert!(
        drawn.warnings[0].contains(MISSING) && drawn.warnings[0].contains("no asset"),
        "{:?}",
        drawn.warnings
    );
    // The mesh parts cast sun shadows: once the meshes are here, every cascade is drawn again with the cubes' two kinds
    // besides the baseplate's.
    let cascades = drawn.stats.shadow_cascades;
    assert!(cascades > 0);
    assert_eq!(drawn.shadow_draws, 3 * cascades, "{:?}", drawn.stats);
    check_reference(&context, "mesh_asset", &drawn.pixels);
}

#[test]
fn the_texture_colours_the_mesh() {
    let Some(context) = gpu() else { return };
    let textured = draw(&context, &mut project("textured", true), (SIZE, SIZE));
    let plain = draw(&context, &mut project("untextured", false), (SIZE, SIZE));
    // Untextured, both cubes are the same mesh kind: one draw for the two.
    assert_eq!((textured.stats.opaque_draws, plain.stats.opaque_draws), (3, 2));
    // Only the big cube's faces change: a good part of the image, by a lot.
    let changed = textured
        .pixels
        .iter()
        .zip(&plain.pixels)
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 40))
        .count();
    let share = changed as f64 / textured.pixels.len() as f64;
    assert!(share > 0.03, "the texture changed {:.2}% of the image", share * 100.0);
}

#[test]
fn the_vertex_buffer_fallback_draws_the_same_image() {
    let Some(primary) = gpu() else { return };
    let Some(fallback) = gpu_with(&ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    }) else {
        return;
    };
    assert_eq!(InstanceBinding::for_context(&fallback), InstanceBinding::Vertex);
    let storage = draw(&primary, &mut project("storage", true), (SIZE, SIZE));
    let vertex = draw(&fallback, &mut project("fallback", true), (SIZE, SIZE));
    assert_eq!(vertex.stats.opaque_draws, 3);
    let (max, mean, _) = compare(&vertex.pixels, &storage.pixels);
    assert!(max <= 2 && mean <= 1.0, "the fallback differs: max {max}, mean {mean}");
    check_reference(&fallback, "mesh_asset-fallback", &vertex.pixels);
}
