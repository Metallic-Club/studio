// Histogram auto-exposure (research prototype, issue #37). Two compute passes per frame:
//   1. build_histogram: 16x16 tiles over the HDR target (Low tier: over a quarter-res copy), 256 log2 bins,
//      centre-weighted, workgroup atomics then one global add per bin (Tardif's layout).
//   2. resolve_exposure: one 256-thread workgroup. Percentile band (ignore the darkest 30% and the
//      brightest 5% of weighted pixels), log-average, EV100, clamp, night bias curve, exponential
//      adaptation with different speeds up and down, then clears the histogram.
// The HDR target is pre-exposed with last frame's exposure, so bins cover a fixed range around display
// values and the pass divides by the previous exposure to recover absolute luminance.

struct ExposureParams {
    min_log2: f32,            // lowest bin edge in pre-exposed units, e.g. -12
    inv_log2_range: f32,      // 1 / (max_log2 - min_log2), e.g. 1 / 18
    log2_nits_per_unit: f32,  // log2(cd/m^2 per renderer unit); renderer unit 1 = sun TOA 120000 lux
    delta_time: f32,          // seconds
    ev100_min: f32,           // Lighting.ExposureMin (default -4)
    ev100_max: f32,           // Lighting.ExposureMax (default 16)
    compensation: f32,        // Lighting.ExposureCompensation, EV (-5..5)
    night_adaptation: f32,    // Lighting.NightAdaptation 0..1
    speed_up: f32,            // adaptation rate toward brighter scenes, 1/s (default 3)
    speed_down: f32,          // adaptation rate toward darker scenes, 1/s (default 1)
    low_percent: f32,         // default 0.30
    high_percent: f32,        // default 0.95
    auto_enabled: u32,        // Lighting.AutoExposure; 0 = hold manual_ev100
    manual_ev100: f32,
    key: f32,                 // display mid-grey target, 0.18
    reset: u32,               // 1 on camera cuts / first frame: jump straight to the target
}

struct Exposure {
    exposure: f32,       // multiplier the renderer applies to every light (pre-exposure) next frame
    ev100: f32,          // adapted EV100
    avg_log2_lum: f32,   // metered log2 luminance, cd/m^2
    target_ev100: f32,   // before adaptation
}

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: ExposureParams;
@group(0) @binding(2) var<storage, read_write> histogram: array<atomic<u32>, 256>;
@group(0) @binding(3) var<storage, read_write> exposure_state: Exposure;

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
const BINS: u32 = 256u;

var<workgroup> local_bins: array<atomic<u32>, 256>;
var<workgroup> scan: array<f32, 256>;
var<workgroup> wsum: array<f32, 256>;

fn bin_of(lum: f32) -> u32 {
    if (lum < 1e-10) {
        return 0u;
    }
    let t = clamp((log2(lum) - params.min_log2) * params.inv_log2_range, 0.0, 1.0);
    return u32(t * 254.0 + 1.0);
}

@compute @workgroup_size(16, 16, 1)
fn build_histogram(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    atomicStore(&local_bins[li], 0u);
    workgroupBarrier();

    let size = textureDimensions(hdr_tex);
    if (gid.x < size.x && gid.y < size.y) {
        let c = textureLoad(hdr_tex, vec2<i32>(gid.xy), 0).rgb;
        let lum = dot(c, LUMA);
        // Centre weight 1..4 (integer so atomics stay exact): the player looks at the middle.
        let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(size) - 0.5;
        let w = 1u + u32(3.0 * exp(-dot(uv, uv) / (2.0 * 0.25 * 0.25)) + 0.5);
        atomicAdd(&local_bins[bin_of(lum)], w);
    }
    workgroupBarrier();
    let count = atomicLoad(&local_bins[li]);
    if (count > 0u) {
        atomicAdd(&histogram[li], count);
    }
}

@compute @workgroup_size(256, 1, 1)
fn resolve_exposure(@builtin(local_invocation_index) li: u32) {
    let count = f32(atomicLoad(&histogram[li]));
    // Bin 0 holds black pixels (sky-less voids, letterbox): never meter them.
    var c = count;
    if (li == 0u) {
        c = 0.0;
    }
    scan[li] = c;
    workgroupBarrier();

    // Inclusive prefix sum (Hillis-Steele). Barriers stay outside the guarded writes.
    for (var off = 1u; off < BINS; off = off * 2u) {
        var add = 0.0;
        if (li >= off) {
            add = scan[li - off];
        }
        workgroupBarrier();
        scan[li] = scan[li] + add;
        workgroupBarrier();
    }
    let total = scan[BINS - 1u];
    let lo = params.low_percent * total;
    let hi = params.high_percent * total;
    // Portion of this bin inside the [lo, hi] band of the cumulative distribution.
    let start = scan[li] - c;
    let end = scan[li];
    let inside = max(min(end, hi) - max(start, lo), 0.0);
    let bin_log2 = params.min_log2 + ((f32(li) - 1.0 + 0.5) / 254.0) / params.inv_log2_range;
    wsum[li] = inside * bin_log2;
    scan[li] = inside; // reuse as weight storage after the prefix sum is consumed
    workgroupBarrier();
    for (var stride = BINS / 2u; stride > 0u; stride = stride / 2u) {
        if (li < stride) {
            wsum[li] = wsum[li] + wsum[li + stride];
            scan[li] = scan[li] + scan[li + stride];
        }
        workgroupBarrier();
    }

    if (li == 0u) {
        let prev = exposure_state;
        var avg_pre = params.min_log2 + 0.5 / params.inv_log2_range; // empty frame: hold the middle
        if (scan[0] > 0.0) {
            avg_pre = wsum[0] / scan[0];
        }
        // Undo last frame's pre-exposure and convert to cd/m^2.
        let avg_log2_nits = avg_pre - log2(max(prev.exposure, 1e-20)) + params.log2_nits_per_unit;
        // EV100 = log2(L * S / K) with S = 100, K = 12.5 (Filament's convention).
        var ev = avg_log2_nits + 3.0;
        ev = clamp(ev, params.ev100_min, params.ev100_max);
        if (params.auto_enabled == 0u) {
            ev = params.manual_ev100;
        }
        var adapted = ev;
        if (params.reset == 0u && params.auto_enabled == 1u) {
            let rate = select(params.speed_down, params.speed_up, ev > prev.ev100);
            adapted = prev.ev100 + (ev - prev.ev100) * (1.0 - exp(-params.delta_time * rate));
        }
        // Night bias: dark scenes stay darker than mid-grey (up to -2 EV at EV100 <= 0) so night reads
        // as night instead of a grey day. Strength follows NightAdaptation.
        let s = 1.0 - smoothstep(0.0, 10.0, adapted);
        let bias = -2.0 * s * (0.5 + 0.5 * params.night_adaptation);
        // Scene luminance that maps to mid-grey, in renderer units.
        let l_avg_units = exp2(adapted - 3.0 - params.log2_nits_per_unit);
        var out: Exposure;
        out.exposure = params.key / l_avg_units * exp2(bias + params.compensation);
        out.ev100 = adapted;
        out.avg_log2_lum = avg_log2_nits;
        out.target_ev100 = ev;
        exposure_state = out;
    }
    // Clear for next frame (every thread clears its own bin; the read above happened before barriers).
    atomicStore(&histogram[li], 0u);
}
