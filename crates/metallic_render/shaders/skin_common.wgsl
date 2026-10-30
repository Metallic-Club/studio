// Shared by the skinned scene and shadow shaders: one instance's joint matrices and slots.
struct Skin {
    params: vec4<f32>,                 // x: studs per model unit (procedural detail scale)
    slot_color: array<vec4<f32>, 8>,   // linear RGB, alpha
    slot_material: array<vec4<f32>, 8>,// roughness, metalness, material id, unused
    joints: array<mat4x4<f32>>,
}

@group(1) @binding(0) var<storage, read> skin: Skin;

struct SkinIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) joints: vec4<u32>,
    @location(3) weights: vec4<f32>,
    @location(4) slot: u32,
}

// Linear blend skinning: the weighted sum of the vertex's joint matrices (weights sum to 1).
fn skin_matrix(j: vec4<u32>, w: vec4<f32>) -> mat4x4<f32> {
    return skin.joints[j.x] * w.x + skin.joints[j.y] * w.y + skin.joints[j.z] * w.z + skin.joints[j.w] * w.w;
}
