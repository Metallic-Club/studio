//! Import of the default avatar and of small hand-built files, including hostile ones.
use glam::{Mat4, Vec3};
use metallic_asset::{AssetError, Pose, SkinnedModel};
use serde_json::{Value, json};

const AVATAR: &[u8] = include_bytes!("../../../art/avatar/out/default_avatar.glb");

fn avatar() -> SkinnedModel {
    SkinnedModel::from_glb(AVATAR).expect("default avatar imports")
}

#[test]
fn default_avatar_has_the_vrm_skeleton() {
    let m = avatar();
    let s = &m.skeleton;
    // 53 joints: `root` plus 52 VRM-humanoid bones. The armature node above `root` is not a
    // joint; it becomes `Skeleton::base`.
    assert_eq!(s.len(), 53);
    assert_eq!(s.names[0], "root");
    assert_eq!(s.parents[0], None);
    for name in [
        "hips",
        "spine",
        "chest",
        "upperChest",
        "neck",
        "head",
        "leftUpperArm",
        "rightLowerArm",
        "leftThumbDistal",
        "rightLittleDistal",
        "leftUpperLeg",
        "rightToes",
    ] {
        assert!(s.joint(name).is_some(), "missing joint {name}");
    }
    for (j, p) in s.parents.iter().enumerate() {
        if let Some(p) = p {
            assert!(*p < j, "parent {p} of joint {j} comes later");
        }
    }
    let hips = s.joint("hips").unwrap();
    assert_eq!(s.parents[s.joint("spine").unwrap()], Some(hips));
    assert_eq!(s.parents[hips], Some(0));
}

#[test]
fn default_avatar_has_three_lods_and_six_slots() {
    let m = avatar();
    assert_eq!(m.lods.len(), 3);
    let names: Vec<&str> = m.lods.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "DefaultAvatar_LOD0",
            "DefaultAvatar_LOD1",
            "DefaultAvatar_LOD2"
        ]
    );
    let tris: Vec<usize> = m.lods.iter().map(|l| l.indices.len() / 3).collect();
    assert_eq!(tris, [5908, 2812, 888]);
    let verts: Vec<usize> = m.lods.iter().map(|l| l.vertices.len()).collect();
    assert_eq!(verts, [4960, 2897, 1102]);
    let slots: Vec<&str> = m.materials.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        slots,
        ["M_Head", "M_Torso", "M_Arms", "M_Legs", "M_Hands", "M_Feet"]
    );
    // 1.54 m tall (5.5 studs of 0.28 m), feet at 0.
    assert!((m.height() - 1.54).abs() < 1e-3, "{}", m.height());
    assert!(m.lods[0].min.y.abs() < 1e-4);
    for lod in &m.lods {
        let mut used = [false; 6];
        for v in &lod.vertices {
            let sum: f32 = v.weights.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5);
            assert!(v.joints.iter().all(|&j| (j as usize) < m.skeleton.len()));
            used[v.material as usize] = true;
        }
        assert_eq!(used, [true; 6], "{}", lod.name);
    }
}

/// Clip names and frame ranges from the Phase B report (30 fps, frames 0..=last).
#[test]
fn default_avatar_has_every_clip_at_30_fps() {
    let m = avatar();
    let want = [
        ("idle", 90),
        ("idle_b", 120),
        ("walk_slow", 20),
        ("walk", 16),
        ("run", 16),
        ("jump", 18),
        ("fall", 24),
        ("land", 16),
        ("climb", 24),
        ("swim", 36),
        ("swim_idle", 48),
        ("sit", 60),
        ("wave", 60),
        ("point", 45),
        ("cheer", 48),
        ("dance", 60),
    ];
    for (name, last) in want {
        let c = m
            .clip(name)
            .unwrap_or_else(|| panic!("missing clip {name}"));
        let frames = c.duration * 30.0;
        assert!(
            (frames - last as f32).abs() < 1e-3,
            "{name}: {} s = {frames} frames at 30 fps, want {last}",
            c.duration
        );
        // Every joint's translation, rotation and scale is keyed.
        assert_eq!(c.tracks.len(), 53 * 3, "{name}");
    }
    for pose in ["pose_hand_on_hip", "pose_run", "pose_sit_back"] {
        assert!(m.clip(pose).is_some(), "missing {pose}");
    }
    assert_eq!(m.clips.len(), 19);
}

#[test]
fn default_avatar_rest_pose_reproduces_the_bind_pose() {
    let m = avatar();
    let mut mats = Vec::new();
    Pose::rest(&m.skeleton).skin_matrices(&m.skeleton, Mat4::IDENTITY, &mut mats);
    for (j, mat) in mats.iter().enumerate() {
        assert!(
            mat.abs_diff_eq(Mat4::IDENTITY, 1e-3),
            "joint {} moves the bind pose: {mat:?}",
            m.skeleton.names[j]
        );
    }
}

#[test]
fn default_avatar_idle_keeps_the_feet_on_the_ground() {
    let m = avatar();
    let idle = m.clip("idle").unwrap();
    let mut pose = Pose::rest(&m.skeleton);
    let mut mats = Vec::new();
    let lod = &m.lods[0];
    for k in 0..6 {
        idle.sample(k as f32 * 0.5, &mut pose);
        pose.skin_matrices(&m.skeleton, Mat4::IDENTITY, &mut mats);
        let lowest = lod
            .vertices
            .iter()
            .map(|v| {
                let p = Vec3::from_array(v.position);
                (0..4)
                    .map(|i| mats[v.joints[i] as usize].transform_point3(p) * v.weights[i])
                    .sum::<Vec3>()
                    .y
            })
            .fold(f32::MAX, f32::min);
        assert!(
            lowest.abs() < 0.01,
            "idle at {} s: lowest point {lowest} m",
            k as f32 * 0.5
        );
    }
}

// ---- small hand-built files ----------------------------------------------------------------

/// A two-joint skin with one triangle and a one-track clip; `edit` changes the JSON and the BIN
/// chunk before they are packed.
fn tiny(edit: impl FnOnce(&mut Value, &mut Vec<u8>)) -> Vec<u8> {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut push = |bin: &mut Vec<u8>, bytes: &[u8]| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(json!({"buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len()}));
        bin.extend_from_slice(bytes);
    };
    let f = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    push(&mut bin, &f(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0])); // 0 positions
    push(&mut bin, &f(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0])); // 1 normals
    push(&mut bin, &[0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0]); // 2 joints (u8)
    push(
        &mut bin,
        &f(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0]),
    ); // 3 weights
    push(&mut bin, &[0, 0, 1, 0, 2, 0, 0, 0]); // 4 indices (u16, padded)
    push(&mut bin, &f(&[0.0, 1.0])); // 5 key times
    push(&mut bin, &f(&[0.0, 1.0, 0.0, 0.0, 3.0, 0.0])); // 6 translations
    let mut doc = json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": 0}],
        "bufferViews": views,
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5121, "count": 3, "type": "VEC4"},
            {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4"},
            {"bufferView": 4, "componentType": 5123, "count": 3, "type": "SCALAR"},
            {"bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR",
             "min": [0.0], "max": [1.0]},
            {"bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC3"}
        ],
        "nodes": [
            {"name": "root", "children": [1]},
            {"name": "bone", "translation": [0.0, 1.0, 0.0]},
            {"name": "Body", "mesh": 0, "skin": 0},
            {"name": "Armature", "children": [0, 2], "translation": [0.0, 0.0, 2.0]}
        ],
        "skins": [{"joints": [0, 1]}],
        "meshes": [{"primitives": [{"attributes":
            {"POSITION": 0, "NORMAL": 1, "JOINTS_0": 2, "WEIGHTS_0": 3}, "indices": 4}]}],
        "animations": [{"name": "nod",
            "samplers": [{"input": 5, "output": 6, "interpolation": "LINEAR"}],
            "channels": [{"sampler": 0, "target": {"node": 1, "path": "translation"}}]}],
        "scenes": [{"nodes": [3]}],
        "scene": 0
    });
    edit(&mut doc, &mut bin);
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    if doc["buffers"][0]["byteLength"] == json!(0) {
        doc["buffers"][0]["byteLength"] = json!(bin.len());
    }
    pack(&doc, &bin)
}

fn pack(doc: &Value, bin: &[u8]) -> Vec<u8> {
    let mut js = serde_json::to_vec(doc).unwrap();
    while !js.len().is_multiple_of(4) {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(bin);
    out
}

fn import(bytes: &[u8]) -> Result<SkinnedModel, AssetError> {
    SkinnedModel::from_glb(bytes)
}

#[test]
fn tiny_file_imports() {
    let m = import(&tiny(|_, _| {})).expect("tiny file");
    assert_eq!(m.skeleton.names, ["root", "bone"]);
    assert_eq!(m.skeleton.parents, [None, Some(0)]);
    // The armature's translation becomes the base.
    assert!(
        m.skeleton
            .base
            .abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, 0.0, 2.0)), 1e-6)
    );
    assert_eq!(m.lods.len(), 1);
    assert_eq!(m.lods[0].indices, [0, 1, 2]);
    // Weights renormalised and sorted, joint indices kept.
    let v = m.lods[0].vertices[2];
    assert_eq!(v.weights, [0.5, 0.5, 0.0, 0.0]);
    assert_eq!(
        m.materials.len(),
        1,
        "a default slot for the material-less primitive"
    );
    assert_eq!(m.clips[0].name, "nod");
    assert_eq!(m.clips[0].duration, 1.0);
}

#[test]
fn joints_are_sorted_parents_first_and_vertices_follow() {
    // List the child before its parent in the skin.
    let m = import(&tiny(|doc, _| doc["skins"][0]["joints"] = json!([1, 0]))).unwrap();
    assert_eq!(m.skeleton.names, ["root", "bone"]);
    // File joint 0 is now `bone` (index 1), so vertex 0 (weighted to file joint 0) uses joint 1.
    assert_eq!(m.lods[0].vertices[0].joints[0], 1);
    assert_eq!(m.clips[0].tracks[0].joint, 1);
}

fn expect_err(bytes: &[u8]) -> AssetError {
    match import(bytes) {
        Ok(_) => panic!("a bad file imported"),
        Err(e) => e,
    }
}

#[test]
fn truncated_files_are_refused() {
    let full = tiny(|_, _| {});
    for len in [0, 3, 11, 12, 19, 20, 40, full.len() / 2, full.len() - 1] {
        let e = expect_err(&full[..len]);
        assert!(matches!(e, AssetError::Parse(_)), "{len}: {e:?}");
    }
    // Every truncation of the real avatar, at a few spots through the JSON and BIN chunks.
    for frac in [1, 7, 33, 50, 51, 77, 99] {
        let len = AVATAR.len() * frac / 100;
        expect_err(&AVATAR[..len]);
    }
}

#[test]
fn out_of_range_joint_index_is_refused() {
    let e = expect_err(&tiny(|_, bin| {
        let joints = 72; // byte offset of view 2 (36 + 36)
        bin[joints] = 9;
    }));
    assert!(
        matches!(&e, AssetError::Invalid(m) if m.contains("joint 9")),
        "{e:?}"
    );
}

#[test]
fn nan_and_negative_weights_are_refused() {
    let weights = 84; // view 3
    let e = expect_err(&tiny(|_, bin| {
        bin[weights..weights + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    }));
    assert!(
        matches!(&e, AssetError::Invalid(m) if m.contains("not finite")),
        "{e:?}"
    );
    let e = expect_err(&tiny(|_, bin| {
        bin[weights..weights + 4].copy_from_slice(&(-1.0f32).to_le_bytes());
    }));
    assert!(matches!(e, AssetError::Invalid(_)), "{e:?}");
    let e = expect_err(&tiny(|_, bin| {
        bin[weights..weights + 4].copy_from_slice(&0.0f32.to_le_bytes());
    }));
    assert!(
        matches!(&e, AssetError::Invalid(m) if m.contains("no joint weight")),
        "{e:?}"
    );
}

#[test]
fn huge_counts_are_refused_before_allocating() {
    let e = expect_err(&tiny(|doc, _| {
        doc["accessors"][0]["count"] = json!(4_000_000_000u64)
    }));
    assert!(matches!(e, AssetError::TooLarge { .. }), "{e:?}");
    // In the limit, but more elements than the buffer view holds.
    let e = expect_err(&tiny(|doc, _| {
        doc["accessors"][0]["count"] = json!(100_000)
    }));
    assert!(matches!(e, AssetError::Invalid(_)), "{e:?}");
    let e = expect_err(&tiny(|doc, _| {
        doc["accessors"][5]["count"] = json!(usize::MAX)
    }));
    assert!(
        matches!(e, AssetError::TooLarge { .. } | AssetError::Invalid(_)),
        "{e:?}"
    );
    let joints: Vec<usize> = (0..300).map(|_| 0).collect();
    let e = expect_err(&tiny(|doc, _| doc["skins"][0]["joints"] = json!(joints)));
    assert!(
        matches!(e, AssetError::TooLarge { .. } | AssetError::Parse(_)),
        "{e:?}"
    );
}

#[test]
fn broken_references_are_refused() {
    // Index past the triangle's three vertices.
    let e = expect_err(&tiny(|_, bin| bin[132] = 7));
    assert!(
        matches!(&e, AssetError::Invalid(m) if m.contains("index 7")),
        "{e:?}"
    );
    // A buffer view reaching past the BIN chunk.
    let e = expect_err(&tiny(|doc, _| {
        doc["bufferViews"][6]["byteLength"] = json!(1 << 20)
    }));
    assert!(
        matches!(e, AssetError::Invalid(_) | AssetError::Parse(_)),
        "{e:?}"
    );
    // Decreasing key times.
    let e = expect_err(&tiny(|_, bin| {
        bin[140..144].copy_from_slice(&5.0f32.to_le_bytes())
    }));
    assert!(
        matches!(&e, AssetError::Invalid(m) if m.contains("decreasing")),
        "{e:?}"
    );
    // A node that is its own child.
    let e = expect_err(&tiny(|doc, _| doc["nodes"][1]["children"] = json!([1])));
    assert!(
        matches!(e, AssetError::Invalid(_) | AssetError::Parse(_)),
        "{e:?}"
    );
    // External buffers are never opened.
    let e = expect_err(&tiny(|doc, _| {
        doc["buffers"][0]["uri"] = json!("../../secret.bin")
    }));
    assert!(matches!(e, AssetError::Unsupported(_)), "{e:?}");
    // A .gltf (JSON) file is refused outright.
    assert!(matches!(
        expect_err(b"{\"asset\":{}}"),
        AssetError::Parse(_)
    ));
}

/// Flipping bytes all over a valid file never panics (errors or a successful import are both
/// fine).
#[test]
fn corrupted_bytes_never_panic() {
    let clean = tiny(|_, _| {});
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..20_000 {
        let mut bytes = clean.clone();
        for _ in 0..1 + next() % 4 {
            let at = (next() % bytes.len() as u64) as usize;
            bytes[at] = next() as u8;
        }
        let _ = import(&bytes);
    }
    // And a few on the real avatar's BIN chunk, where the mesh and key data live.
    let bin_start = AVATAR.len() - 810_000;
    for _ in 0..8 {
        let mut bytes = AVATAR.to_vec();
        for _ in 0..64 {
            let at = bin_start + (next() % 800_000) as usize;
            bytes[at] = next() as u8;
        }
        let _ = import(&bytes);
    }
}
