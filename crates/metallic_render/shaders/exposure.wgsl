// Histogram auto exposure (#151), after the research prototype in
// `docs/research/lighting-shaders/exposure_histogram.wgsl`. Two compute passes after the scene:
//   1. build_histogram: 16x16 tiles over the HDR scene image (before the HDR post passes), 256
//      log2-luminance bins, centre-weighted, workgroup atomics then one global add per bin.
//      Pixels with alpha below 0.5 (toon shading, whose light follows the exposure) are skipped.
//   2. resolve_exposure: one 256-thread workgroup. Ignores the darkest 30% and the brightest 5% of
//      the weighted pixels, log-averages the rest, turns it into EV100, clamps, adapts over time
//      (faster toward bright), applies the night bias and compensation, and writes the multiplier
//      the next frame applies on top of the CPU pre-exposure. Clears the histogram.
// Units: the HDR image is pre-exposed by the lighting's fixed exposure and the finish scales it
// by its own exposure; `log2_base` is both. EV100 is defined so the default lighting's fixed
// exposure meters as EV100 15 (`ev_offset`).

struct Params {
    min_log2: f32,        // lowest bin edge (pre-exposed log2 luminance)
    inv_log2_range: f32,  // 1 / (max - min)
    log2_base: f32,       // log2 of the CPU pre-exposure in the image
    ev_offset: f32,       // EV100 = ev_offset - log2(total exposure)
    log2_key: f32,        // log2 of the pre-exposed luminance metering maps to (mid-grey)
    delta_time: f32,      // seconds since the last metered frame
    ev_min: f32,          // Lighting.ExposureMin
    ev_max: f32,          // Lighting.ExposureMax
    compensation: f32,    // Lighting.ExposureCompensation, EV
    night_adaptation: f32,
    speed_up: f32,        // 1/s toward brighter scenes (EV rising)
    speed_down: f32,      // 1/s toward darker scenes
    low_percent: f32,
    high_percent: f32,
    reset: u32,           // 1: jump to the target (first frame, capture, cut)
    log2_scale: f32,      // log2 of the finish's own exposure on the image (in log2_base too)
}

// Mirrors `ExposureState` in exposure.rs; `multiplier` must stay first (it is copied into the
// frame uniforms).
struct State {
    multiplier: f32,
    ev100: f32,
    metered_log2: f32,
    target_ev100: f32,
}

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> histogram: array<atomic<u32>, 256>;
@group(0) @binding(3) var<storage, read_write> state: State;

const EXPOSURE_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
const BINS: u32 = 256u;

var<workgroup> local_bins: array<atomic<u32>, 256>;
var<workgroup> scan: array<f32, 256>;
var<workgroup> wsum: array<f32, 256>;

// Bin 0: black, NaN or negative (never metered). Infinity counts as the brightest bin.
fn bin_of(lum: f32) -> u32 {
    let bits = bitcast<u32>(lum);
    let non_finite = (bits & 0x7f800000u) == 0x7f800000u;
    if (non_finite) {
        if ((bits & 0x007fffffu) != 0u || (bits & 0x80000000u) != 0u) {
            return 0u;
        }
        return BINS - 1u;
    }
    if (!(lum > 1e-10)) {
        return 0u;
    }
    let t = clamp((log2(lum) - params.min_log2) * params.inv_log2_range, 0.0, 1.0);
    return min(u32(t * 254.0 + 1.0), BINS - 1u);
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
        let px = textureLoad(hdr_tex, vec2<i32>(gid.xy), 0);
        var lum = dot(px.rgb, EXPOSURE_LUMA);
        if (!(px.a >= 0.5)) {
            lum = 0.0;
        }
        // Centre weight 1..4 (integers keep the atomics exact): players look at the middle.
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
    var c = f32(atomicLoad(&histogram[li]));
    if (li == 0u) {
        c = 0.0;
    }
    scan[li] = c;
    workgroupBarrier();

    // Inclusive prefix sum (Hillis-Steele); barriers outside the guarded reads.
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
    // The part of this bin inside the [lo, hi] band of the cumulative distribution.
    let start = scan[li] - c;
    let end = scan[li];
    let inside = max(min(end, hi) - max(start, lo), 0.0);
    let bin_log2 = params.min_log2 + ((f32(li) - 0.5) / 254.0) / params.inv_log2_range;
    workgroupBarrier();
    wsum[li] = inside * bin_log2;
    scan[li] = inside;
    workgroupBarrier();
    for (var stride = BINS / 2u; stride > 0u; stride = stride / 2u) {
        if (li < stride) {
            wsum[li] = wsum[li] + wsum[li + stride];
            scan[li] = scan[li] + scan[li + stride];
        }
        workgroupBarrier();
    }

    if (li == 0u) {
        let prev = state;
        let have_prev = prev.multiplier > 0.0 && params.reset == 0u;
        var out: State;
        if (scan[0] <= 0.0) {
            // Nothing to meter (all black or invalid): hold the last exposure, or none.
            if (have_prev) {
                out = prev;
            } else {
                out.multiplier = 1.0;
                out.ev100 = params.ev_offset - params.log2_base;
                out.metered_log2 = params.min_log2;
                out.target_ev100 = out.ev100;
            }
        } else {
            let metered = wsum[0] / scan[0] + params.log2_scale;
            // Total exposure that maps the metered luminance to the key, as EV100.
            let raw_log2 = metered - params.log2_base;
            let ev = clamp(params.ev_offset - (params.log2_key - raw_log2), params.ev_min, params.ev_max);
            var adapted = ev;
            if (have_prev) {
                let rate = select(params.speed_down, params.speed_up, ev > prev.ev100);
                adapted = prev.ev100 + (ev - prev.ev100) * (1.0 - exp(-params.delta_time * rate));
            }
            // Night bias: dark scenes stay darker than mid-grey (up to 2 EV), so night reads as
            // night instead of a grey day.
            let s = 1.0 - smoothstep(0.0, 10.0, adapted);
            let bias = -2.0 * s * (0.5 + 0.5 * params.night_adaptation);
            let log2_total = params.ev_offset - adapted + bias + params.compensation;
            out.multiplier = exp2(clamp(log2_total - params.log2_base, -24.0, 24.0));
            out.ev100 = adapted;
            out.metered_log2 = metered;
            out.target_ev100 = ev;
        }
        state = out;
    }
    atomicStore(&histogram[li], 0u);
}
