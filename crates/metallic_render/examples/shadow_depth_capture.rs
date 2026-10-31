//! Captures the same shadow scene on an explicit backend, without a timing benchmark.
//! Run with `-- dx12 output.png` or `-- vulkan output.png`.
use metallic_core::{Color3, Value, Vec3 as WorldVec3, World};
use metallic_render::glam::Vec3;
use metallic_render::{Camera, Quality, RenderContext, RenderSettings, Renderer};
use std::error::Error;
use std::io::BufWriter;
use std::path::Path;

fn part(world: &mut World, name: &str, position: WorldVec3, size: WorldVec3, color: Color3) {
    let id = world
        .create("Part", world.root("Workspace"))
        .expect("fixture Part");
    for (property, value) in [
        ("Name", Value::String(name.to_owned())),
        ("Position", Value::Vector3(position)),
        ("Size", Value::Vector3(size)),
        ("Color", Value::Color3(color)),
        ("Anchored", Value::Bool(true)),
    ] {
        world.set(id, property, value).expect("fixture property");
    }
}

fn scene() -> World {
    let mut world = World::new_with_services();
    let lighting = world.root("Lighting").expect("Lighting");
    world
        .set(lighting, "ClockTime", Value::Number(14.0))
        .expect("fixture time");
    world
        .set(lighting, "GlobalShadows", Value::Bool(true))
        .expect("fixture shadows");
    part(
        &mut world,
        "Ground",
        WorldVec3::new(0.0, -0.5, -80.0),
        WorldVec3::new(180.0, 1.0, 320.0),
        Color3::new(160.0, 160.0, 160.0),
    );
    for row in 0..5 {
        for column in 0..3 {
            let height = 4.0 + f64::from(column) * 4.0;
            part(
                &mut world,
                &format!("Caster_{row}_{column}"),
                WorldVec3::new(
                    (f64::from(column) - 1.0) * 15.0,
                    height * 0.5,
                    15.0 - f64::from(row) * 40.0,
                ),
                WorldVec3::new(6.0, height, 6.0),
                Color3::new(186.0, 186.0, 186.0),
            );
        }
    }
    world
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: shadow_depth_capture <dx12|vulkan> <output.png>".into());
    }
    let backend = match args[0].as_str() {
        "dx12" => wgpu::Backends::DX12,
        "vulkan" => wgpu::Backends::VULKAN,
        _ => return Err("backend must be dx12 or vulkan".into()),
    };
    let output = Path::new(&args[1]);
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let context = RenderContext::new_headless_with_backends(backend)?;
    let info = context
        .adapter
        .as_ref()
        .expect("headless adapter")
        .get_info();
    println!("backend={:?}, adapter={}", info.backend, info.name);
    let mut renderer = Renderer::with_settings(
        &context,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::High,
        },
    );
    renderer.sync(&scene());
    let size = (960, 540);
    let pixels = renderer.capture(
        &Camera {
            position: Vec3::new(40.0, 36.0, 65.0),
            look_at: Vec3::new(0.0, 1.0, -60.0),
            fov_deg: 60.0,
        },
        size,
    )?;
    let mut encoder = png::Encoder::new(
        BufWriter::new(std::fs::File::create(output)?),
        size.0,
        size.1,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&pixels)?;
    writer.finish()?;
    println!("saved {}", output.display());
    Ok(())
}
