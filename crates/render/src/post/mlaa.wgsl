// MLAA's edges and blend weights (`post/mlaa.rs`), after Reshetov's morphological antialiasing (2009) as Jimenez et
// al. made it practical on GPUs (GPU Pro 2, 2011), in one pass: the edges are found from the scene colour's luma as
// the search walks them, instead of in a pass and a texture of their own.
//
// For a pixel with an edge on its top side (against the pixel above) the search walks left and right along the
// edge, up to `search` pixels each way, to its ends, and looks at each end for a crossing edge in either row: the
// step of a staircase. MLAA's shapes follow: a crossing at one end (L), at both on the same side (U) or on opposite
// sides (Z). The silhouette is reconstructed as a line from the middle of each crossing edge to the middle of the
// edge (or across, for Z), and the area between that line and the edge over this pixel's column is how much the
// pixel on the line's side takes of the other's colour. The left side is the same turned a quarter. The weights go
// in an RGBA8 texture: r, how much this pixel blends with the one above; g, the one above with this; b, this with
// the one to the left; a, the one to the left with this. The finish pass blends (`fs_finish_mlaa`).

struct Mlaa {
    // xy: the drawn corner's size in pixels; z: the edge threshold on luma (0..1); w: the farthest search, in pixels.
    params: vec4<f32>,
    // x: exposure, which the luma is measured after.
    exposure: vec4<f32>,
}

@group(0) @binding(0) var<uniform> mlaa: Mlaa;
@group(0) @binding(1) var scene_color: texture_2d<f32>;

@vertex
fn vs_mlaa(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
}

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// A perceptual luma of the pixel at `p` (clamped to the drawn corner): the square root of its luminance after
// exposure, up to 1, so the threshold means about the same step in dark and bright parts.
fn luma(p: vec2<i32>) -> f32 {
    let q = clamp(p, vec2<i32>(0), vec2<i32>(mlaa.params.xy) - vec2<i32>(1));
    let raw = textureLoad(scene_color, q, 0).rgb;
    // Not-a-number and infinite channels count as black (`finite` in finish.wgsl).
    let ok = (bitcast<vec3<u32>>(raw) & vec3<u32>(0x7f800000u)) != vec3<u32>(0x7f800000u);
    let c = select(vec3<f32>(0.0), raw, ok);
    return sqrt(clamp(dot(c, LUMA) * mlaa.exposure.x, 0.0, 1.0));
}

// Whether there is an edge between `p` and the pixel above it.
fn edge_up(p: vec2<i32>) -> bool {
    return abs(luma(p) - luma(p - vec2<i32>(0, 1))) > mlaa.params.z;
}

// Whether there is an edge between `p` and the pixel to its left.
fn edge_left(p: vec2<i32>) -> bool {
    return abs(luma(p) - luma(p - vec2<i32>(1, 0))) > mlaa.params.z;
}

// The signed area between the line from (x0, y0) to (x1, y1) and the edge (y = 0) over the column from x to x + 1,
// counting only where the line is: positive above the edge.
fn line_area(x0: f32, y0: f32, x1: f32, y1: f32, x: f32) -> f32 {
    let a = max(x, x0);
    let b = min(x + 1.0, x1);
    if b <= a || x1 <= x0 {
        return 0.0;
    }
    let slope = (y1 - y0) / (x1 - x0);
    let ya = y0 + slope * (a - x0);
    let yb = y0 + slope * (b - x0);
    if ya * yb >= 0.0 {
        // A trapezoid on one side of the edge.
        return (ya + yb) * 0.5 * (b - a);
    }
    // The line crosses the edge inside the column: two triangles, and MLAA keeps the larger.
    let cross = a + (b - a) * ya / (ya - yb);
    let left = ya * (cross - a) * 0.5;
    let right = yb * (b - cross) * 0.5;
    return select(right, left, abs(left) > abs(right));
}

// The area over column `x` of a segment `d` pixels long whose ends step to `h0` and `h1` (+0.5: the crossing edge is
// on the far side, -0.5: on this pixel's side, 0: none).
fn shape_area(h0: f32, h1: f32, d: f32, x: f32) -> f32 {
    if h0 == 0.0 && h1 == 0.0 {
        return 0.0;
    }
    if h0 != 0.0 && h1 != 0.0 && h0 != h1 {
        // Z: one line from end to end.
        return line_area(0.0, h0, d, h1, x);
    }
    // L or U: from each end with a crossing to the middle.
    let half = d * 0.5;
    return line_area(0.0, h0, half, 0.0, x) + line_area(half, 0.0, d, h1, x);
}

// The step at a segment's end from its crossing edges: on this pixel's side (`near`), on the far side (`far`), both
// or neither (no step).
fn step_at(near: bool, far: bool) -> f32 {
    if near == far {
        return 0.0;
    }
    return select(0.5, -0.5, near);
}

@fragment
fn fs_mlaa(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(position.xy);
    let search = i32(mlaa.params.w);
    var weights = vec4<f32>(0.0);
    if edge_up(p) {
        // Walk the edge along the row, left then right.
        var left = 0;
        loop {
            if left >= search || !edge_up(p - vec2<i32>(left + 1, 0)) {
                break;
            }
            left += 1;
        }
        var right = 0;
        loop {
            if right >= search || !edge_up(p + vec2<i32>(right + 1, 0)) {
                break;
            }
            right += 1;
        }
        let start = p - vec2<i32>(left, 0);
        let end = p + vec2<i32>(right, 0);
        // Crossing edges at each end: in this pixel's row, or in the row above.
        let h0 = step_at(edge_left(start), edge_left(start - vec2<i32>(0, 1)));
        let h1 = step_at(edge_left(end + vec2<i32>(1, 0)), edge_left(end + vec2<i32>(1, -1)));
        let area = shape_area(h0, h1, f32(left + right + 1), f32(left));
        // Below the edge (this pixel's side) this pixel takes the colour above; above it, the pixel above takes
        // this one's.
        weights.r = max(-area, 0.0);
        weights.g = max(area, 0.0);
    }
    if edge_left(p) {
        var up = 0;
        loop {
            if up >= search || !edge_left(p - vec2<i32>(0, up + 1)) {
                break;
            }
            up += 1;
        }
        var down = 0;
        loop {
            if down >= search || !edge_left(p + vec2<i32>(0, down + 1)) {
                break;
            }
            down += 1;
        }
        let start = p - vec2<i32>(0, up);
        let end = p + vec2<i32>(0, down);
        let h0 = step_at(edge_up(start), edge_up(start - vec2<i32>(1, 0)));
        let h1 = step_at(edge_up(end + vec2<i32>(0, 1)), edge_up(end + vec2<i32>(-1, 1)));
        let area = shape_area(h0, h1, f32(up + down + 1), f32(up));
        weights.b = max(-area, 0.0);
        weights.a = max(area, 0.0);
    }
    return weights;
}
