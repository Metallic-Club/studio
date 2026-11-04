//! Runs a stress scene (ADR 0008 §10) in a window for a number of frames, prints the frame statistics as JSON and
//! exits.
//!
//! ```text
//! cargo run --release -p metallic_engine --example stress -- --scene parts_100k
//! cargo run --release -p metallic_engine --example stress -- --scene all --frames 2000 > results.json
//! cargo run --release -p metallic_engine --example stress -- --list
//! ```
//!
//! Options: `--scene <name|all>`, `--frames <n>` (measured, 1000), `--warm-up <n>` (120), `--seed <n>` (1),
//! `--size <width>x<height>` (logical pixels; the scene's own by default), `--vsync` (off by default, so frame
//! times show the work), `--overlay` (show the performance overlay while running), `--no-cull` (draw every part, the
//! numbers before culling), `--view-distance <m>` and `--min-screen-size <px>` (the culling thresholds; infinite and
//! 1 by default), `--fov <degrees>` (narrow or widen the scene's camera), `--moving-camera` (walk the camera through
//! the scene instead of keeping it still), `--no-shadows`, `--shadow-cascades <n>`, `--shadow-resolution <texels>`,
//! `--shadow-distance <m>` and `--dynamic-cascades <n>` (the sun's shadows; 3, 2048, 256 and 2 by default),
//! `--lights <low|medium|high|off>` (the point and spot lights' settings as the presets will set them; medium by
//! default), `--light-budget <n>`, `--lights-per-cluster <n>`, `--shadowed-lights <n>` and `--no-light-shadows`,
//! `--lod <low|medium|high|off>` (the levels of detail's preset, high by default; off draws every part at full
//! detail), `--lod-bias <x>` and `--lod-tint` (colour the parts by level of detail).
//! `WGPU_BACKEND=dx12` (or `vulkan`, `gl`) picks the backend.
//!
//! Graphics settings (#84): `--preset <low|medium|high|ultra|auto>` sets the culling, shadows, lights, levels of
//! detail and resolution as the Player's preset does (so it cannot be combined with the flags above that set them
//! one by one; auto calibrates in the warm-up), `--render-scale <x>` draws the scene at that scale, and
//! `--dynamic-resolution <ms>` holds that GPU frame time between `--min-scale <x>` and `--max-scale <x>` (0.5 and 1
//! by default). `--switch-presets <n>` switches between Low and Ultra every `n` frames, to measure the switches.
//!
//! The post chain (#85): `--aa <off|mlaa|msaa2|msaa4|msaa8>` and `--effects <low|medium|high|ultra>` choose the
//! anti-aliasing and the effect quality (with `--preset`, as the player's overrides); `--merge-scene` records the
//! scene passes as one; `--no-post` draws the frame as before the post chain (no HDR target, bloom, tone curve or
//! MLAA; the scene passes apart), to measure what it costs.
//!
//! No hitches (#86): `--toggle-effects <n>` toggles the next of `metallic_engine::stress::TOGGLES` every `n` frames
//! from the first (every preset, anti-aliasing and effect quality, lights, shadows and the overlay off and on), while
//! the renderer still compiles its pipelines in the background; the JSON's `spikes` counts the frames over twice the
//! frame target after the first 2 s (`--frame-target <ms>`, a 60 fps frame by default), and `graphics.pipelines` says
//! what was compiled when; `METALLIC_TRACE_SPIKES=1` prints each spike on stderr with the toggle before it, and
//! `METALLIC_TRACE_SWITCHES=1` the ten frames after each toggle: their times, the shadows and atlas tiles drawn, the
//! textures still being made off the frame (#152), and each frame's GPU time per pass once it arrives (its own writes
//! to stderr can cost a frame now and then: count spikes without it).
//! `--pipeline-cache <dir>` keeps Vulkan's pipeline cache in `dir`.
//!
//! `--scene all` runs every scene, each in its own process (the event loop can only be made once per process), and
//! prints a JSON array. The JSON's fields are `metallic_engine::stress::BenchReport`'s; times are in milliseconds.

use std::error::Error;
use std::process::{Command, ExitCode};

use metallic_engine::stress::{BenchOptions, StressKind, run_bench};
use metallic_render::resolution::ResolutionSettings;
use metallic_render::settings::{GraphicsSettings, Preset, Quality, Tier};
use metallic_render::{AntiAliasing, CullSettings, LightSettings, LodSettings};

fn usage() -> String {
    format!(
        "usage: stress --scene <name|all> [--frames N] [--warm-up N] [--seed N] [--size WxH] [--vsync] [--overlay]\n\
         \x20             [--no-cull] [--view-distance M] [--min-screen-size PX] [--fov DEGREES] [--moving-camera]\n\
         \x20             [--no-shadows] [--shadow-cascades N] [--shadow-resolution N] [--shadow-distance M]\n\
         \x20             [--dynamic-cascades N] [--lights low|medium|high|off] [--light-budget N]\n\
         \x20             [--lights-per-cluster N] [--shadowed-lights N] [--no-light-shadows]\n\
         \x20             [--lod low|medium|high|off] [--lod-bias X] [--lod-tint]\n\
         \x20             [--preset low|medium|high|ultra|auto] [--render-scale X] [--dynamic-resolution MS]\n\
         \x20             [--min-scale X] [--max-scale X] [--switch-presets N]\n\
         \x20             [--aa off|mlaa|msaa2|msaa4|msaa8] [--effects low|medium|high|ultra] [--merge-scene]\n\
         \x20             [--no-post] [--toggle-effects N] [--frame-target MS] [--pipeline-cache DIR]\n\
         scenes: {}",
        StressKind::names().join(", ")
    )
}

/// The parsed command line: the scene (or `None` for all) and the options for each.
fn parse(args: &[String]) -> Result<Option<(Option<StressKind>, BenchOptions)>, String> {
    let mut scene = None;
    let mut options = BenchOptions::new(StressKind::Parts10k);
    // A flag that sets one system's settings, which a preset would override.
    let mut per_system = None;
    let mut preset = None;
    let (mut render_scale, mut target_ms, mut min_scale, mut max_scale) = (None, None, None, None);
    let (mut anti_aliasing, mut effects) = (None, None);
    let mut it = args.iter();
    let number = |value: Option<&String>, flag: &str| -> Result<u64, String> {
        value
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("{flag} needs a number"))
    };
    let decimal = |value: Option<&String>, flag: &str| -> Result<f32, String> {
        value
            .and_then(|v| v.parse().ok())
            .filter(|v: &f32| v.is_finite() && *v >= 0.0)
            .ok_or_else(|| format!("{flag} needs a number"))
    };
    while let Some(arg) = it.next() {
        if PER_SYSTEM_FLAGS.contains(&arg.as_str()) {
            per_system = Some(arg.clone());
        }
        match arg.as_str() {
            "--preset" => {
                let name = it.next().ok_or("--preset needs a name")?;
                preset = Some(name.parse::<Preset>().map_err(|e| e.to_string())?);
            }
            "--render-scale" => render_scale = Some(decimal(it.next(), "--render-scale")?),
            "--dynamic-resolution" => target_ms = Some(decimal(it.next(), "--dynamic-resolution")?),
            "--min-scale" => min_scale = Some(decimal(it.next(), "--min-scale")?),
            "--max-scale" => max_scale = Some(decimal(it.next(), "--max-scale")?),
            "--switch-presets" => {
                options.switch_presets =
                    Some(u32::try_from(number(it.next(), "--switch-presets")?).map_err(|e| e.to_string())?);
            }
            "--toggle-effects" => {
                options.toggle_effects =
                    Some(u32::try_from(number(it.next(), "--toggle-effects")?).map_err(|e| e.to_string())?);
            }
            "--frame-target" => options.frame_target_ms = Some(decimal(it.next(), "--frame-target")?),
            "--pipeline-cache" => {
                options.context.pipeline_cache_dir = Some(it.next().ok_or("--pipeline-cache needs a folder")?.into());
            }
            "--list" => {
                for kind in StressKind::ALL {
                    println!("{}", kind.name());
                }
                return Ok(None);
            }
            "--scene" => {
                let name = it.next().ok_or("--scene needs a name")?;
                scene = Some(if name == "all" {
                    None
                } else {
                    Some(StressKind::from_name(name).ok_or_else(|| format!("no scene {name:?}"))?)
                });
            }
            "--frames" => options.frames = u32::try_from(number(it.next(), "--frames")?).map_err(|e| e.to_string())?,
            "--warm-up" => {
                options.warm_up = u32::try_from(number(it.next(), "--warm-up")?).map_err(|e| e.to_string())?;
            }
            "--seed" => options.seed = number(it.next(), "--seed")?,
            "--size" => {
                let size = it.next().ok_or("--size needs WxH")?;
                let (w, h) = size.split_once('x').ok_or("--size needs WxH")?;
                let w = w.parse().map_err(|_| "--size needs WxH")?;
                let h = h.parse().map_err(|_| "--size needs WxH")?;
                options.window = Some((w, h));
            }
            "--vsync" => options.vsync = true,
            "--overlay" => options.overlay = true,
            "--no-cull" => options.cull = CullSettings::OFF,
            "--view-distance" => options.cull.max_distance = decimal(it.next(), "--view-distance")?,
            "--min-screen-size" => options.cull.min_screen_size = decimal(it.next(), "--min-screen-size")?,
            "--fov" => options.field_of_view = Some(decimal(it.next(), "--fov")?),
            "--moving-camera" => options.moving_camera = true,
            "--no-shadows" => options.shadows.enabled = false,
            "--shadow-cascades" => {
                options.shadows.cascades =
                    u32::try_from(number(it.next(), "--shadow-cascades")?).map_err(|e| e.to_string())?;
            }
            "--shadow-resolution" => {
                options.shadows.resolution =
                    u32::try_from(number(it.next(), "--shadow-resolution")?).map_err(|e| e.to_string())?;
            }
            "--shadow-distance" => options.shadows.distance = decimal(it.next(), "--shadow-distance")?,
            "--dynamic-cascades" => {
                options.shadows.dynamic_cascades =
                    u32::try_from(number(it.next(), "--dynamic-cascades")?).map_err(|e| e.to_string())?;
            }
            "--lights" => {
                options.lights = match it.next().map(String::as_str) {
                    Some("low") => LightSettings::LOW,
                    Some("medium") => LightSettings::MEDIUM,
                    Some("high") => LightSettings::HIGH,
                    Some("off") => LightSettings::OFF,
                    _ => return Err("--lights needs low, medium, high or off".into()),
                };
            }
            "--light-budget" => {
                options.lights.budget =
                    u32::try_from(number(it.next(), "--light-budget")?).map_err(|e| e.to_string())?;
            }
            "--shadowed-lights" => {
                options.lights.shadowed_lights =
                    u32::try_from(number(it.next(), "--shadowed-lights")?).map_err(|e| e.to_string())?;
            }
            "--no-light-shadows" => options.lights.shadows = false,
            "--lights-per-cluster" => {
                options.lights.per_cluster =
                    u32::try_from(number(it.next(), "--lights-per-cluster")?).map_err(|e| e.to_string())?;
            }
            "--lod" => {
                options.lod = match it.next().map(String::as_str) {
                    Some("low") => LodSettings::LOW,
                    Some("medium") => LodSettings::MEDIUM,
                    Some("high") => LodSettings::HIGH,
                    Some("off") => LodSettings::OFF,
                    _ => return Err("--lod needs low, medium, high or off".into()),
                };
            }
            "--lod-bias" => options.lod.bias = decimal(it.next(), "--lod-bias")?,
            "--lod-tint" => options.lod_tint = true,
            "--aa" => {
                let name = it.next().ok_or("--aa needs off, mlaa, msaa2, msaa4 or msaa8")?;
                anti_aliasing = Some(name.parse::<AntiAliasing>().map_err(|e| e.to_string())?);
            }
            "--effects" => {
                let name = it.next().ok_or("--effects needs low, medium, high or ultra")?;
                effects = Some(name.parse::<Tier>().map_err(|e| e.to_string())?);
            }
            "--merge-scene" => options.merge_scene_passes = true,
            "--no-post" => options.post_chain = false,
            // `cargo bench`-style runners pass this; nothing to do.
            "--bench" => {}
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let scene = scene.ok_or("--scene is required")?;
    if let Some(kind) = scene {
        options.scene = kind;
    }
    if options.switch_presets.is_some() && preset.is_none() {
        preset = Some(Preset::Low);
    }
    match preset {
        Some(preset) => {
            if let Some(flag) = per_system {
                return Err(format!("{flag} sets what --preset sets; use one or the other"));
            }
            options.graphics = Some(GraphicsSettings {
                render_scale,
                dynamic_resolution: target_ms.map(|_| true),
                target_frame_ms: target_ms,
                min_render_scale: min_scale,
                max_render_scale: max_scale,
                anti_aliasing,
                effect_quality: effects,
                ..GraphicsSettings::new(preset)
            });
        }
        None => {
            options.anti_aliasing = anti_aliasing;
            options.effects = effects.map(|tier| Quality::for_tier(tier).post);
            options.resolution = match target_ms {
                Some(target) => {
                    let max = max_scale.unwrap_or(1.0);
                    ResolutionSettings {
                        scale: render_scale.unwrap_or(max),
                        ..ResolutionSettings::dynamic(target, min_scale.unwrap_or(0.5), max)
                    }
                }
                None => ResolutionSettings::fixed(render_scale.unwrap_or(1.0)),
            };
        }
    }
    Ok(Some((scene, options)))
}

/// The flags that set one system's settings, which `--preset` sets as a whole.
const PER_SYSTEM_FLAGS: &[&str] = &[
    "--no-cull",
    "--view-distance",
    "--min-screen-size",
    "--no-shadows",
    "--shadow-cascades",
    "--shadow-resolution",
    "--shadow-distance",
    "--dynamic-cascades",
    "--lights",
    "--light-budget",
    "--shadowed-lights",
    "--no-light-shadows",
    "--lights-per-cluster",
    "--lod",
    "--lod-bias",
];

/// Runs every scene, each in a child process with the same options, and prints a JSON array.
fn run_all(args: &[String]) -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    let mut reports = Vec::new();
    for kind in StressKind::ALL {
        eprintln!("stress: running {}", kind.name());
        let mut child_args: Vec<String> = Vec::with_capacity(args.len());
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if arg == "--scene" {
                it.next();
                continue;
            }
            child_args.push(arg.clone());
        }
        let output = Command::new(&exe)
            .args(&child_args)
            .args(["--scene", kind.name()])
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "{} failed: {}",
                kind.name(),
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        let stdout = String::from_utf8(output.stdout)?;
        let json = stdout
            .lines()
            .rev()
            .find(|line| line.starts_with('{'))
            .ok_or_else(|| format!("{} printed no report", kind.name()))?;
        reports.push(json.to_owned());
    }
    println!("[\n{}\n]", reports.join(",\n"));
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match parse(&args) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}\n{}", usage());
            return ExitCode::FAILURE;
        }
    };
    let result = match parsed {
        (None, _) => run_all(&args),
        (Some(_), options) => run_bench(&options)
            .map(|report| println!("{}", report.to_json()))
            .map_err(Into::into),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("stress: {error}");
            ExitCode::FAILURE
        }
    }
}
