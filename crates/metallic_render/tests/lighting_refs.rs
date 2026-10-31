//! The lighting shaders against the numpy reference renderer (#155).
//!
//! `tests/data/lighting_refs.json` is written by `python -I tools/lighting/make_refs.py` from
//! `docs/research/lighting-shaders/reference/` (the #37 model). The checks cover only what the
//! engine models:
//! - sky: the engine draws the sky from a table of the reference sky (`m1_sky_table.md`, three
//!   significant figures) instead of simulating it. At the table's elevations its zenith,
//!   horizon and sun colours must match the reference to the table's rounding; between them the
//!   engine interpolates, and claims only the reference's channel order (hue) and the direction
//!   of each brightness change as the sun sets. The colours are checked on the CPU (`evaluate`)
//!   and through `sky_gradient` in `common.wgsl` on the GPU.
//! - fog: `fog_depth` in `lighting.wgsl` (optical depth of exponential height fog) on the GPU.
//! - tone curves: `ToneCurve::map` on the CPU and `apply_curve` in `tonecurve.wgsl` on the GPU.
//!
//! What the engine deliberately leaves out (azimuth in the sky, its vertical profile, point
//! lights, the physical overcast sky) is listed in `docs/agents/155-lighting-refs.md`.

use metallic_render::lighting::evaluate;
use metallic_render::{LightingInputs, RenderContext, ToneCurve};
use serde_json::Value;
use wgpu::util::DeviceExt;

const REFS: &str = include_str!("data/lighting_refs.json");
const COLOR_WGSL: &str = include_str!("../shaders/color.wgsl");
const TONECURVE_WGSL: &str = include_str!("../shaders/tonecurve.wgsl");
const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");
const LIGHTING_WGSL: &str = include_str!("../shaders/lighting.wgsl");

/// Table rows hold three significant figures: at most 0.5% rounding, plus f32 noise.
const TABLE_REL: f32 = 0.006;
/// Two channels closer than this (relative) have no reliable order.
const HUE_MARGIN: f32 = 0.05;
/// Brightness steps between neighbouring sample elevations smaller than this (relative) are
/// within the table's interpolation error, so their direction is not claimed.
const STEP_MARGIN: f32 = 0.10;
/// `fog_depth` falls back to the first-order integral below |k dy| = 1e-3 (error up to 5e-4).
const FOG_REL: f32 = 1e-3;
/// Tone curves, display-linear 0..1: f32 against f64.
const CURVE_ABS: f32 = 1e-3;

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

fn refs() -> Value {
    serde_json::from_str(REFS).expect("lighting_refs.json")
}

fn f(v: &Value) -> f32 {
    v.as_f64().expect("number") as f32
}

fn v3(v: &Value) -> [f32; 3] {
    let a = v.as_array().expect("array");
    [f(&a[0]), f(&a[1]), f(&a[2])]
}

fn luma(c: [f32; 3]) -> f32 {
    c.iter().zip(LUMA).map(|(a, b)| a * b).sum()
}

fn rel(got: f32, want: f32) -> f32 {
    (got - want).abs() / want.abs().max(1e-30)
}

// ---- Sky ------------------------------------------------------------------------------------

/// The engine's sky (zenith, horizon, sun) for the sun at `elevation` degrees, without the
/// exposure the CPU folds into every light. Default lighting with Brightness, MoonBrightness 1,
/// no colour shift, no clouds and the reference air colour, as the table was made.
fn engine_sky(elevation: f32) -> [[f32; 3]; 3] {
    let mut inp = LightingInputs {
        // At latitude 0 the sun's elevation is 90 degrees minus its hour angle.
        clock_time: 12.0 + (90.0 - elevation) / 15.0,
        latitude: 0.0,
        brightness: 1.0,
        moon_brightness: 1.0,
        color_shift_top: [0.0; 3],
        cloud_cover: 0.0,
        auto_exposure: false,
        ..LightingInputs::default()
    };
    if let Some(a) = inp.atmosphere.as_mut() {
        a.air_color = [116.0 / 255.0, 172.0 / 255.0, 1.0];
    }
    let frame = evaluate(&inp);
    let e = frame.exposure;
    let un = |c: [f32; 3]| c.map(|v| v / e);
    [
        un(frame.sky.zenith),
        un(frame.sky.horizon),
        un(frame.sky.sun_color),
    ]
}

struct SkyRow {
    elevation: f32,
    table: bool,
    /// zenith, horizon, sun
    reference: [[f32; 3]; 3],
}

const PARTS: [&str; 3] = ["zenith", "horizon", "sun"];

fn sky_rows() -> Vec<SkyRow> {
    let r = refs();
    r["sky"]
        .as_array()
        .expect("sky")
        .iter()
        .map(|row| SkyRow {
            elevation: f(&row["elevation"]),
            table: row["table"].as_bool().expect("table"),
            reference: [v3(&row["zenith"]), v3(&row["horizon"]), v3(&row["sun"])],
        })
        .collect()
}

/// Between the horizon and 1 degree below it the sun fades out instead of setting at once
/// (the engine keeps lighting with it until -1 degree); the reference sun has set.
fn sun_fading(elevation: f32) -> bool {
    elevation < 0.0 && elevation > -2.0
}

#[test]
fn sky_matches_the_reference_at_the_table_elevations() {
    let mut worst = 0f32;
    for row in sky_rows().iter().filter(|r| r.table) {
        let got = engine_sky(row.elevation);
        for (k, part) in PARTS.iter().enumerate() {
            for c in 0..3 {
                let e = rel(got[k][c], row.reference[k][c]);
                worst = worst.max(e);
                assert!(
                    e <= TABLE_REL,
                    "{part} at {:+}: got {:?}, reference {:?}",
                    row.elevation,
                    got[k],
                    row.reference[k]
                );
            }
        }
    }
    eprintln!("table rows: worst {:.4}% off", worst * 100.0);
}

#[test]
fn sky_keeps_the_reference_hue_at_every_elevation() {
    for row in sky_rows() {
        let got = engine_sky(row.elevation);
        for (k, part) in PARTS.iter().enumerate() {
            if k == 2 && sun_fading(row.elevation) {
                continue;
            }
            let (g, w) = (got[k], row.reference[k]);
            for i in 0..3 {
                for j in 0..3 {
                    if w[i] > w[j] * (1.0 + HUE_MARGIN) {
                        assert!(
                            g[i] > g[j],
                            "{part} at {:+}: channel {i} should exceed {j}: got {g:?}, \
                             reference {w:?}",
                            row.elevation
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn sky_brightens_and_darkens_with_the_reference() {
    let mut rows = sky_rows();
    rows.sort_by(|a, b| b.elevation.total_cmp(&a.elevation));
    let got: Vec<_> = rows.iter().map(|r| engine_sky(r.elevation)).collect();
    for (k, part) in PARTS.iter().enumerate() {
        for i in 1..rows.len() {
            let (a, b) = (&rows[i - 1], &rows[i]);
            if k == 2 && (sun_fading(a.elevation) || sun_fading(b.elevation)) {
                continue;
            }
            let (wa, wb) = (luma(a.reference[k]), luma(b.reference[k]));
            let (ga, gb) = (luma(got[i - 1][k]), luma(got[i][k]));
            if wb < wa * (1.0 - STEP_MARGIN) {
                assert!(
                    gb < ga,
                    "{part} should darken from {:+} to {:+}: got {ga} -> {gb}, reference \
                     {wa} -> {wb}",
                    a.elevation,
                    b.elevation
                );
            } else if wb > wa * (1.0 + STEP_MARGIN) {
                assert!(
                    gb > ga,
                    "{part} should brighten from {:+} to {:+}: got {ga} -> {gb}, reference \
                     {wa} -> {wb}",
                    a.elevation,
                    b.elevation
                );
            }
        }
    }
}

#[test]
fn moonlight_is_not_the_setting_sun() {
    // Between 1 and 2 degrees below the horizon the moon lights the scene; its colour must be
    // the moon's (a few millionths of the day sun), not the tail of the sunset.
    for e in [-1.01f32, -1.5, -1.99] {
        let sun = luma(engine_sky(e)[2]);
        assert!(sun < 1e-5, "light at {e:+}: luminance {sun}");
    }
}

// ---- GPU probes -----------------------------------------------------------------------------

const PROBE_WGSL: &str = r#"
struct ProbeIn { a: vec4<f32>, b: vec4<f32> }
@group(1) @binding(0) var<storage, read> probe_in: array<ProbeIn>;
@group(1) @binding(1) var<storage, read_write> probe_out: array<vec4<f32>>;

// b.w selects: 0 sky_gradient(a.xyz), 1 fog_depth(a.xyz, b.xyz), 2 apply_curve(a.rgb, a.w).
@compute @workgroup_size(64)
fn probe(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= arrayLength(&probe_in)) {
        return;
    }
    let p = probe_in[i];
    var out = vec4<f32>(0.0);
    let mode = u32(p.b.w + 0.5);
    if (mode == 0u) {
        out = vec4<f32>(sky_gradient(p.a.xyz), 0.0);
    } else if (mode == 1u) {
        out = vec4<f32>(fog_depth(p.a.xyz, p.b.xyz), 0.0, 0.0, 0.0);
    } else {
        out = vec4<f32>(apply_curve(p.a.rgb, u32(p.a.w + 0.5)), 0.0);
    }
    probe_out[i] = out;
}
"#;

fn probe_source() -> String {
    [
        COLOR_WGSL,
        TONECURVE_WGSL,
        COMMON_WGSL,
        LIGHTING_WGSL,
        PROBE_WGSL,
    ]
    .concat()
}

/// Byte offsets of `Globals` members and the struct size, read from the shader itself.
fn globals_layout(src: &str) -> (std::collections::HashMap<String, u32>, u32) {
    let module =
        naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    for (_, ty) in module.types.iter() {
        if ty.name.as_deref() == Some("Globals")
            && let naga::TypeInner::Struct { members, span } = &ty.inner
        {
            let offsets = members
                .iter()
                .map(|m| (m.name.clone().unwrap_or_default(), m.offset))
                .collect();
            return (offsets, *span);
        }
    }
    panic!("no Globals struct");
}

struct Probe {
    ctx: RenderContext,
    pipeline: wgpu::ComputePipeline,
    offsets: std::collections::HashMap<String, u32>,
    globals_size: u32,
}

/// Values for `Globals` members by name (zero elsewhere).
type GlobalsFill<'a> = &'a [(&'a str, [f32; 4])];

impl Probe {
    fn new() -> Option<Probe> {
        let ctx = match RenderContext::new_headless() {
            Ok(c) => c,
            Err(_) => {
                eprintln!("no GPU adapter; skipping");
                return None;
            }
        };
        let l = ctx.device.limits();
        if l.max_storage_buffers_per_shader_stage < 2 || l.max_compute_workgroup_size_x < 64 {
            eprintln!("no compute shaders; skipping");
            return None;
        }
        let src = probe_source();
        let (offsets, globals_size) = globals_layout(&src);
        let module = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("lighting probe"),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
        let pipeline = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("lighting probe"),
                layout: None,
                module: &module,
                entry_point: Some("probe"),
                compilation_options: Default::default(),
                cache: None,
            });
        Some(Probe {
            ctx,
            pipeline,
            offsets,
            globals_size,
        })
    }

    /// Runs the probe on `inputs` (a, b pairs) with `Globals` filled from `fill`.
    fn run(&self, fill: GlobalsFill, inputs: &[[[f32; 4]; 2]]) -> Vec<[f32; 4]> {
        let device = &self.ctx.device;
        let mut globals = vec![0f32; self.globals_size as usize / 4];
        for (name, v) in fill {
            let at = self.offsets[*name] as usize / 4;
            globals[at..at + 4].copy_from_slice(v);
        }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("probe globals"),
            contents: bytemuck::cast_slice(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("probe in"),
            contents: bytemuck::cast_slice(inputs),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let size = (inputs.len() * 16) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe out"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe staging"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let g0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("probe globals"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let g1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("probe io"),
            layout: &self.pipeline.get_bind_group_layout(1),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("probe"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("probe"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &g0, &[]);
            pass.set_bind_group(1, &g1, &[]);
            pass.dispatch_workgroups(inputs.len().div_ceil(64) as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, size);
        self.ctx.queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        rx.recv().expect("map").expect("map");
        let out = {
            let mapped = slice.get_mapped_range().expect("mapped");
            bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec()
        };
        staging.unmap();
        out
    }
}

/// GPU tests one at a time (the GL backend deadlocks with devices in parallel).
fn gpu() -> std::sync::MutexGuard<'static, ()> {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

fn w(c: [f32; 3]) -> [f32; 4] {
    [c[0], c[1], c[2], 0.0]
}

#[test]
fn gpu_sky_gradient_draws_the_table_colours() {
    let _gpu = gpu();
    let Some(probe) = Probe::new() else { return };
    let dirs = [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]];
    for row in sky_rows().iter().filter(|r| r.table) {
        let [zenith, horizon, _] = engine_sky(row.elevation);
        let fill = [
            ("sky_zenith", w(zenith)),
            ("sky_horizon", w(horizon)),
            ("sky_ground", w(horizon)),
        ];
        let inputs: Vec<_> = dirs
            .iter()
            .map(|d| [[d[0], d[1], d[2], 0.0], [0.0; 4]])
            .collect();
        let out = probe.run(&fill, &inputs);
        let want = [row.reference[0], row.reference[1], row.reference[1]];
        for (k, o) in out.iter().enumerate() {
            for c in 0..3 {
                assert!(
                    rel(o[c], want[k][c]) <= TABLE_REL,
                    "sky_gradient({:?}) at {:+}: got {o:?}, reference {:?}",
                    dirs[k],
                    row.elevation,
                    want[k]
                );
            }
        }
    }
}

#[test]
fn gpu_fog_depth_matches_the_reference() {
    let _gpu = gpu();
    let Some(probe) = Probe::new() else { return };
    let r = refs();
    let mut worst = 0f32;
    for case in r["fog"].as_array().expect("fog") {
        let fill = [(
            "fog_params",
            [
                f(&case["sigma0"]),
                f(&case["falloff"]),
                f(&case["fog_height"]),
                1.0,
            ],
        )];
        let (p0, p1) = (v3(&case["p0"]), v3(&case["p1"]));
        let out = probe.run(&fill, &[[w(p0), [p1[0], p1[1], p1[2], 1.0]]]);
        let want = f(&case["tau"]);
        let e = rel(out[0][0], want);
        worst = worst.max(e);
        assert!(
            e <= FOG_REL,
            "fog_depth {p0:?} -> {p1:?} ({case}): got {}, reference {want}",
            out[0][0]
        );
    }
    eprintln!("fog: worst {:.2e} relative", worst);
}

fn curve_by_name(name: &str) -> ToneCurve {
    match name {
        "Filmic" => ToneCurve::Filmic,
        "Agx" => ToneCurve::Agx,
        "AgxPunchy" => ToneCurve::AgxPunchy,
        "Neutral" => ToneCurve::Neutral,
        "Aces" => ToneCurve::Aces,
        "Clamp" => ToneCurve::Clamp,
        _ => panic!("unknown curve {name}"),
    }
}

/// Reference outputs of one curve, one per input.
type CurveOutputs = (ToneCurve, Vec<[f32; 3]>);

fn curve_refs() -> (Vec<[f32; 3]>, Vec<CurveOutputs>) {
    let r = refs();
    let inputs = r["curves"]["inputs"]
        .as_array()
        .expect("inputs")
        .iter()
        .map(v3)
        .collect();
    let outputs = r["curves"]["outputs"]
        .as_object()
        .expect("outputs")
        .iter()
        .map(|(name, v)| {
            (
                curve_by_name(name),
                v.as_array().expect("outputs").iter().map(v3).collect(),
            )
        })
        .collect();
    (inputs, outputs)
}

#[test]
fn cpu_tone_curves_match_the_reference() {
    let (inputs, outputs) = curve_refs();
    assert_eq!(outputs.len(), ToneCurve::ALL.len());
    for (curve, want) in outputs {
        // `map` applies the curve's pre-scale first; the reference has none.
        let s = curve.prescale();
        for (x, w) in inputs.iter().zip(want) {
            let got = curve.map(x.map(|v| v / s));
            for c in 0..3 {
                assert!(
                    (got[c] - w[c]).abs() <= CURVE_ABS,
                    "{curve:?} {x:?}: got {got:?}, reference {w:?}"
                );
            }
        }
    }
}

#[test]
fn gpu_tone_curves_match_the_reference() {
    let _gpu = gpu();
    let Some(probe) = Probe::new() else { return };
    let (inputs, outputs) = curve_refs();
    for (curve, want) in outputs {
        let k = curve.index() as f32;
        let probes: Vec<_> = inputs
            .iter()
            .map(|x| [[x[0], x[1], x[2], k], [0.0, 0.0, 0.0, 2.0]])
            .collect();
        let out = probe.run(&[], &probes);
        for ((x, w), got) in inputs.iter().zip(want).zip(out) {
            for c in 0..3 {
                assert!(
                    (got[c] - w[c]).abs() <= CURVE_ABS,
                    "{curve:?} {x:?}: got {got:?}, reference {w:?}"
                );
            }
        }
    }
}
