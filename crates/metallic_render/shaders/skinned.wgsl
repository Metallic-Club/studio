// Skinned meshes in the main pass. Writes the same VOut as vs_main, so fs_main shades skinned
// characters exactly like parts.
@vertex
fn vs_skinned(v: SkinIn) -> VOut {
    let m = skin_matrix(v.joints, v.weights);
    let wp = m * vec4<f32>(v.pos, 1.0);
    let s = min(v.slot, 7u);
    var out: VOut;
    out.clip = g.view_proj * wp;
    out.world_pos = wp.xyz;
    // Joint matrices carry rotation and uniform scale only, so the normal needs no inverse.
    out.world_n = normalize((m * vec4<f32>(v.normal, 0.0)).xyz);
    // Procedural detail sticks to the bind pose, so it does not swim as the joints move.
    out.local_p = v.pos * skin.params.x;
    out.local_n = v.normal;
    out.color = skin.slot_color[s];
    out.material = skin.slot_material[s];
    return out;
}
