// Debug views (#23, src/debug_views.rs). Joined after scene.wgsl into the scene module.

// Shadow cascade covering a surface `view_depth` away, or -1 past the last.
fn debug_cascade(view_depth: f32) -> i32 {
    let count = i32(g.params.x);
    for (var i = 0; i < count; i = i + 1) {
        if (view_depth < g.cascade_splits[i]) {
            return i;
        }
    }
    return -1;
}

// The scene light for a surface in the current debug view: `col` itself unless the view
// replaces it. `lit` is the sun's share on the surface (n.l times shadow), 0..1.
fn debug_scene(col: vec3<f32>, n: vec3<f32>, lit: f32, view_depth: f32) -> vec3<f32> {
    let mode = debug_mode();
    if (mode == 2) {
        return unfinish(n * 0.5 + 0.5);
    }
    if (mode == 4) {
        let ci = debug_cascade(view_depth);
        var tint = vec3<f32>(0.42, 0.43, 0.46);
        if (ci == 0) {
            tint = vec3<f32>(0.56, 0.44, 0.86);
        } else if (ci == 1) {
            tint = vec3<f32>(0.36, 0.6, 0.82);
        } else if (ci == 2) {
            tint = vec3<f32>(0.86, 0.68, 0.38);
        }
        return unfinish(tint * (0.35 + 0.65 * lit));
    }
    return col;
}

// Edges of parts (Wireframe) and debug shapes: their colour, lightened a little, as displayed.
// Alpha 0 keeps them out of the exposure meter.
@fragment
fn fs_wire(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(unfinish(mix(in.color.rgb, vec3<f32>(1.0), 0.3)), 0.0);
}

// One layer of overdraw, added up by the blend state: brighter and warmer where more surfaces
// cover a pixel.
@fragment
fn fs_overdraw(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(unfinish(vec3<f32>(0.11, 0.035, 0.012)), 0.0);
}
