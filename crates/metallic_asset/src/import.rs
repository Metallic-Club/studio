//! `.glb` import: skeleton, skinned meshes per level of detail, materials and clips.
use crate::accessor::{read_floats, read_uints};
use crate::anim::{Channel, Clip, Interpolation, Track, Transform};
use crate::skeleton::Skeleton;
use crate::{
    AssetError, MAX_CLIPS, MAX_GLB_BYTES, MAX_INDICES, MAX_INFLUENCES, MAX_JOINTS, MAX_KEYFRAMES,
    MAX_MATERIALS, MAX_VERTICES, MaterialSlot, SkinnedMesh, SkinnedModel, SkinnedVertex,
};
use glam::{Mat4, Quat, Vec3};
use gltf::Document;
use gltf::animation::{Interpolation as GltfInterpolation, Property};
use gltf::mesh::{Mode, Semantic};

fn invalid(msg: impl Into<String>) -> AssetError {
    AssetError::Invalid(msg.into())
}

fn too_large(what: &str, count: usize, limit: usize) -> AssetError {
    AssetError::TooLarge {
        what: what.to_owned(),
        count,
        limit,
    }
}

pub(crate) fn from_glb(bytes: &[u8]) -> Result<SkinnedModel, AssetError> {
    if bytes.len() > MAX_GLB_BYTES {
        return Err(too_large("file size in bytes", bytes.len(), MAX_GLB_BYTES));
    }
    // Only binary glTF: a .gltf JSON file would need its buffers from other files.
    if !bytes.starts_with(b"glTF") {
        return Err(AssetError::Parse("not a binary glTF (.glb) file".into()));
    }
    let gltf = parse(bytes)?;
    let bin: &[u8] = gltf.blob.as_deref().unwrap_or(&[]);
    let doc = &gltf.document;
    for buffer in doc.buffers() {
        if !matches!(buffer.source(), gltf::buffer::Source::Bin) {
            return Err(AssetError::Unsupported(format!(
                "buffer {} is not the file's BIN chunk",
                buffer.index()
            )));
        }
        if buffer.length() > bin.len() {
            return Err(invalid(format!(
                "buffer {} declares {} bytes, the BIN chunk has {}",
                buffer.index(),
                buffer.length(),
                bin.len()
            )));
        }
    }

    let parents = node_parents(doc)?;
    let skin = the_skin(doc)?;
    let (skeleton, joint_of_node) = read_skeleton(doc, bin, &skin, &parents)?;
    let (materials, default_slot) = read_materials(doc)?;
    let lods = read_lods(doc, bin, &skin, &joint_of_node, default_slot)?;
    let clips = read_clips(doc, bin, &joint_of_node)?;
    Ok(SkinnedModel {
        skeleton,
        lods,
        materials,
        clips,
    })
}

/// Parses and validates the glTF JSON.
///
/// gltf-json 1.4 indexes a few things before (or without) checking them: the POSITION accessor
/// in its own validation, and animation channel targets (node index and path) in the typed API.
/// Those are checked here first. Any other panic inside the third-party parser is turned into an
/// error as a last line of defence, so a hostile file can never take the engine down.
fn parse(bytes: &[u8]) -> Result<gltf::Gltf, AssetError> {
    use gltf::json::validation::{Checked, Validate};
    let parsed = std::panic::catch_unwind(|| {
        let gltf = gltf::Gltf::from_slice_without_validation(bytes)
            .map_err(|e| AssetError::Parse(e.to_string()))?;
        let root = gltf.document.as_json();
        for mesh in &root.meshes {
            for prim in &mesh.primitives {
                if prim
                    .attributes
                    .values()
                    .any(|a| a.value() >= root.accessors.len())
                {
                    return Err(invalid("a mesh attribute names a missing accessor"));
                }
            }
        }
        for anim in &root.animations {
            for ch in &anim.channels {
                if ch.target.node.value() >= root.nodes.len()
                    || ch.sampler.value() >= anim.samplers.len()
                    || !matches!(ch.target.path, Checked::Valid(_))
                {
                    return Err(invalid("an animation channel has a bad target or sampler"));
                }
            }
        }
        let mut first: Option<String> = None;
        root.validate(root, gltf::json::Path::new, &mut |path, error| {
            if first.is_none() {
                first = Some(format!("{}: {error}", path()));
            }
        });
        match first {
            Some(e) => Err(AssetError::Parse(e)),
            None => Ok(gltf),
        }
    });
    parsed.unwrap_or_else(|_| Err(AssetError::Parse("the glTF parser failed".into())))
}

/// Parent node of every node; a node listed as the child of two nodes (or of itself) is refused.
fn node_parents(doc: &Document) -> Result<Vec<Option<usize>>, AssetError> {
    let mut parents = vec![None; doc.nodes().len()];
    for node in doc.nodes() {
        for child in node.children() {
            let slot = parents
                .get_mut(child.index())
                .ok_or_else(|| invalid("child node index out of range"))?;
            if slot.is_some() || child.index() == node.index() {
                return Err(invalid(format!(
                    "node {} has more than one parent",
                    child.index()
                )));
            }
            *slot = Some(node.index());
        }
    }
    Ok(parents)
}

/// The one skin used by the file's skinned mesh nodes.
fn the_skin(doc: &Document) -> Result<gltf::Skin<'_>, AssetError> {
    let mut found: Option<gltf::Skin<'_>> = None;
    for node in doc.nodes() {
        if let (Some(_), Some(skin)) = (node.mesh(), node.skin()) {
            match &found {
                Some(s) if s.index() != skin.index() => {
                    return Err(AssetError::Unsupported(
                        "skinned meshes use more than one skin".into(),
                    ));
                }
                _ => found = Some(skin),
            }
        }
    }
    found.ok_or_else(|| invalid("the file has no skinned mesh"))
}

fn node_transform(node: &gltf::Node<'_>) -> Result<Transform, AssetError> {
    let (t, r, s) = node.transform().decomposed();
    let translation = Vec3::from_array(t);
    let scale = Vec3::from_array(s);
    let rotation = Quat::from_array(r);
    if !translation.is_finite() || !scale.is_finite() || !rotation.is_finite() {
        return Err(invalid(format!(
            "node {} has a non-finite transform",
            node.index()
        )));
    }
    if rotation.length_squared() < 1e-12 {
        return Err(invalid(format!(
            "node {} has a zero rotation",
            node.index()
        )));
    }
    Ok(Transform {
        translation,
        rotation: rotation.normalize(),
        scale,
    })
}

/// Reads the skin's joints, sorts them parents-first and returns the skeleton plus the map from
/// node index to joint index.
fn read_skeleton(
    doc: &Document,
    bin: &[u8],
    skin: &gltf::Skin<'_>,
    parents: &[Option<usize>],
) -> Result<(Skeleton, Vec<Option<usize>>), AssetError> {
    let nodes: Vec<gltf::Node<'_>> = doc.nodes().collect();
    let joint_nodes: Vec<usize> = skin.joints().map(|n| n.index()).collect();
    let count = joint_nodes.len();
    if count == 0 {
        return Err(invalid("the skin has no joints"));
    }
    if count > MAX_JOINTS {
        return Err(too_large("joints", count, MAX_JOINTS));
    }
    // File joint index of each node.
    let mut file_joint = vec![None; nodes.len()];
    for (j, &n) in joint_nodes.iter().enumerate() {
        let slot = file_joint
            .get_mut(n)
            .ok_or_else(|| invalid("joint node out of range"))?;
        if slot.is_some() {
            return Err(invalid(format!("node {n} is listed twice as a joint")));
        }
        *slot = Some(j);
    }

    // Parent joint (file order) and depth of each joint; a cycle shows up as a chain longer
    // than the node count.
    let mut file_parent = vec![None; count];
    let mut depth = vec![0usize; count];
    let mut base_node: Option<Option<usize>> = None;
    for (j, &n) in joint_nodes.iter().enumerate() {
        let parent_node = parents[n];
        match parent_node.and_then(|p| file_joint[p]) {
            Some(pj) => file_parent[j] = Some(pj),
            None => {
                // A root joint: everything above it must be non-joint nodes, the same for all
                // roots.
                let mut cur = parent_node;
                let mut steps = 0;
                while let Some(c) = cur {
                    if file_joint[c].is_some() {
                        return Err(AssetError::Unsupported(
                            "a non-joint node sits between two joints".into(),
                        ));
                    }
                    steps += 1;
                    if steps > nodes.len() {
                        return Err(invalid("the node hierarchy has a cycle"));
                    }
                    cur = parents[c];
                }
                match base_node {
                    Some(b) if b != parent_node => {
                        return Err(AssetError::Unsupported(
                            "root joints have different parents".into(),
                        ));
                    }
                    _ => base_node = Some(parent_node),
                }
            }
        }
        let mut d = 0;
        let mut cur = file_parent[j];
        while let Some(p) = cur {
            d += 1;
            if d > count {
                return Err(invalid("the joint hierarchy has a cycle"));
            }
            cur = file_parent[p];
        }
        depth[j] = d;
    }

    // Model-space transform of the nodes above the roots.
    let mut base = Mat4::IDENTITY;
    let mut cur = base_node.flatten();
    while let Some(c) = cur {
        base = node_transform(&nodes[c])?.to_mat4() * base;
        cur = parents[c];
    }

    // Parents-first order: a stable sort by depth keeps the file order among equals.
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|&j| depth[j]);
    let mut new_of_file = vec![0usize; count];
    for (new, &old) in order.iter().enumerate() {
        new_of_file[old] = new;
    }

    let inverse_bind_file: Vec<Mat4> = match skin.inverse_bind_matrices() {
        Some(acc) => {
            if acc.count() != count {
                return Err(invalid(format!(
                    "{} inverse bind matrices for {count} joints",
                    acc.count()
                )));
            }
            read_floats(&acc, bin, 16, MAX_JOINTS, "inverse bind matrices")?
                .as_chunks::<16>()
                .0
                .iter()
                .map(Mat4::from_cols_array)
                .collect()
        }
        None => vec![Mat4::IDENTITY; count],
    };

    let mut skeleton = Skeleton {
        names: Vec::with_capacity(count),
        parents: Vec::with_capacity(count),
        rest: Vec::with_capacity(count),
        inverse_bind: Vec::with_capacity(count),
        base,
    };
    for &old in &order {
        let node = &nodes[joint_nodes[old]];
        skeleton.names.push(
            node.name()
                .map_or_else(|| format!("joint{old}"), str::to_owned),
        );
        skeleton
            .parents
            .push(file_parent[old].map(|p| new_of_file[p]));
        skeleton.rest.push(node_transform(node)?);
        skeleton.inverse_bind.push(inverse_bind_file[old]);
    }
    let joint_of_node = file_joint
        .iter()
        .map(|j| j.map(|old| new_of_file[old]))
        .collect();
    Ok((skeleton, joint_of_node))
}

/// Material slots, plus the slot given to primitives without a material (appended when needed).
fn read_materials(doc: &Document) -> Result<(Vec<MaterialSlot>, u8), AssetError> {
    let mut slots: Vec<MaterialSlot> = doc
        .materials()
        .map(|m| {
            let pbr = m.pbr_metallic_roughness();
            MaterialSlot {
                name: m.name().map_or_else(
                    || format!("material{}", m.index().unwrap_or(0)),
                    str::to_owned,
                ),
                base_color: pbr.base_color_factor().map(|c| c.clamp(0.0, 1.0)),
                roughness: pbr.roughness_factor().clamp(0.0, 1.0),
                metallic: pbr.metallic_factor().clamp(0.0, 1.0),
            }
        })
        .collect();
    let needs_default = doc
        .meshes()
        .flat_map(|m| m.primitives())
        .any(|p| p.material().index().is_none());
    if needs_default {
        // The glTF default material.
        slots.push(MaterialSlot {
            name: "default".into(),
            base_color: [1.0; 4],
            roughness: 1.0,
            metallic: 1.0,
        });
    }
    if slots.len() > MAX_MATERIALS {
        return Err(too_large("materials", slots.len(), MAX_MATERIALS));
    }
    let default_slot = slots.len().saturating_sub(1) as u8;
    Ok((slots, default_slot))
}

/// The level of detail named by a `..._LOD<n>` suffix; 0 without one.
fn lod_of(name: Option<&str>) -> usize {
    name.and_then(|n| n.rsplit_once("LOD"))
        .and_then(|(_, digits)| digits.parse().ok())
        .unwrap_or(0)
}

fn read_lods(
    doc: &Document,
    bin: &[u8],
    skin: &gltf::Skin<'_>,
    joint_of_node: &[Option<usize>],
    default_slot: u8,
) -> Result<Vec<SkinnedMesh>, AssetError> {
    // Joint index in file order -> skeleton order.
    let remap: Vec<usize> = skin
        .joints()
        .map(|n| joint_of_node.get(n.index()).copied().flatten().unwrap_or(0))
        .collect();
    let mut levels: Vec<(usize, SkinnedMesh)> = Vec::new();
    let mut vertex_total = 0usize;
    let mut index_total = 0usize;
    for node in doc.nodes() {
        let (Some(mesh), Some(node_skin)) = (node.mesh(), node.skin()) else {
            continue;
        };
        if node_skin.index() != skin.index() {
            continue;
        }
        let lod = lod_of(node.name());
        let at = match levels.iter().position(|(l, _)| *l == lod) {
            Some(i) => i,
            None => {
                levels.push((
                    lod,
                    SkinnedMesh {
                        name: node.name().unwrap_or("mesh").to_owned(),
                        vertices: Vec::new(),
                        indices: Vec::new(),
                        min: Vec3::splat(f32::MAX),
                        max: Vec3::splat(f32::MIN),
                    },
                ));
                levels.len() - 1
            }
        };
        let target = &mut levels[at].1;
        for prim in mesh.primitives() {
            if prim.mode() != Mode::Triangles {
                return Err(AssetError::Unsupported(format!(
                    "mesh {}: primitive mode {:?} (only triangles)",
                    mesh.index(),
                    prim.mode()
                )));
            }
            let slot = prim
                .material()
                .index()
                .map_or(default_slot, |i| i.min(255) as u8);
            let added = read_primitive(
                bin,
                &prim,
                &remap,
                slot,
                target,
                MAX_VERTICES - vertex_total,
            )?;
            vertex_total += added.0;
            index_total += added.1;
            if index_total > MAX_INDICES {
                return Err(too_large("triangle indices", index_total, MAX_INDICES));
            }
        }
    }
    levels.retain(|(_, m)| !m.vertices.is_empty());
    if levels.is_empty() {
        return Err(invalid("the skinned meshes have no vertices"));
    }
    levels.sort_by_key(|(l, _)| *l);
    Ok(levels.into_iter().map(|(_, m)| m).collect())
}

/// Appends one primitive to `target`; returns the vertices and indices added. `room` is how many
/// vertices the limit still allows.
fn read_primitive(
    bin: &[u8],
    prim: &gltf::Primitive<'_>,
    remap: &[usize],
    slot: u8,
    target: &mut SkinnedMesh,
    room: usize,
) -> Result<(usize, usize), AssetError> {
    let attr = |s: Semantic| prim.get(&s);
    let need = |s: Semantic, what: &str| {
        attr(s).ok_or_else(|| invalid(format!("a skinned primitive has no {what}")))
    };
    let position_acc = need(Semantic::Positions, "POSITION")?;
    let n = position_acc.count();
    if n > room {
        return Err(too_large("vertices", MAX_VERTICES - room + n, MAX_VERTICES));
    }
    let positions = read_floats(&position_acc, bin, 3, n, "POSITION")?;
    let same_count = |what: &str, len: usize, width: usize| {
        if len == n * width {
            Ok(())
        } else {
            Err(invalid(format!("{what} count differs from POSITION")))
        }
    };
    let normals = read_floats(&need(Semantic::Normals, "NORMAL")?, bin, 3, n, "NORMAL")?;
    same_count("NORMAL", normals.len(), 3)?;
    let uvs = match attr(Semantic::TexCoords(0)) {
        Some(acc) => read_floats(&acc, bin, 2, n, "TEXCOORD_0")?,
        None => vec![0.0; n * 2],
    };
    same_count("TEXCOORD_0", uvs.len(), 2)?;
    // Up to two sets of four influences; the strongest MAX_INFLUENCES are kept.
    let mut sets = Vec::new();
    for set in 0..2 {
        let (Some(j), Some(w)) = (attr(Semantic::Joints(set)), attr(Semantic::Weights(set))) else {
            if set == 0 {
                return Err(invalid("a skinned primitive has no JOINTS_0 / WEIGHTS_0"));
            }
            break;
        };
        let joints = read_uints(&j, bin, 4, n, "JOINTS")?;
        let weights = read_floats(&w, bin, 4, n, "WEIGHTS")?;
        same_count("JOINTS", joints.len(), 4)?;
        same_count("WEIGHTS", weights.len(), 4)?;
        sets.push((joints, weights));
    }

    let first = target.vertices.len();
    for i in 0..n {
        let mut influences: Vec<(u32, f32)> = Vec::with_capacity(8);
        for (joints, weights) in &sets {
            for k in 0..4 {
                let (joint, weight) = (joints[i * 4 + k], weights[i * 4 + k]);
                if joint as usize >= remap.len() {
                    return Err(invalid(format!(
                        "vertex {i} uses joint {joint}, the skin has {}",
                        remap.len()
                    )));
                }
                if weight < 0.0 {
                    return Err(invalid(format!("vertex {i} has a negative weight")));
                }
                if weight > 0.0 {
                    influences.push((joint, weight));
                }
            }
        }
        influences.sort_by(|a, b| b.1.total_cmp(&a.1));
        influences.truncate(MAX_INFLUENCES);
        let sum: f32 = influences.iter().map(|x| x.1).sum();
        if !sum.is_finite() || sum <= 1e-6 {
            return Err(invalid(format!("vertex {i} has no joint weight")));
        }
        let mut v = SkinnedVertex {
            position: [positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]],
            normal: [normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]],
            uv: [uvs[i * 2], uvs[i * 2 + 1]],
            joints: [0; 4],
            weights: [0.0; 4],
            material: slot,
        };
        for (k, (joint, weight)) in influences.iter().enumerate() {
            v.joints[k] = remap[*joint as usize] as u16;
            v.weights[k] = weight / sum;
        }
        let p = Vec3::from_array(v.position);
        target.min = target.min.min(p);
        target.max = target.max.max(p);
        target.vertices.push(v);
    }

    let indices = match prim.indices() {
        Some(acc) => read_uints(&acc, bin, 1, MAX_INDICES, "indices")?,
        None => (0..n as u32).collect(),
    };
    if indices.len() % 3 != 0 {
        return Err(invalid("index count is not a multiple of 3"));
    }
    for &i in &indices {
        if i as usize >= n {
            return Err(invalid(format!("index {i} is out of range ({n} vertices)")));
        }
        target.indices.push(first as u32 + i);
    }
    Ok((n, indices.len()))
}

fn read_clips(
    doc: &Document,
    bin: &[u8],
    joint_of_node: &[Option<usize>],
) -> Result<Vec<Clip>, AssetError> {
    let count = doc.animations().len();
    if count > MAX_CLIPS {
        return Err(too_large("animation clips", count, MAX_CLIPS));
    }
    let mut keys_left = MAX_KEYFRAMES;
    let mut clips = Vec::with_capacity(count);
    for anim in doc.animations() {
        let name = anim
            .name()
            .map_or_else(|| format!("clip{}", anim.index()), str::to_owned);
        let mut tracks = Vec::new();
        let mut duration = 0.0f32;
        for ch in anim.channels() {
            let channel = match ch.target().property() {
                Property::Translation => Channel::Translation,
                Property::Rotation => Channel::Rotation,
                Property::Scale => Channel::Scale,
                Property::MorphTargetWeights => continue,
            };
            // Channels on nodes outside the skeleton (an armature node, say) are not played:
            // root motion stays off and the controller moves the character.
            let Some(joint) = joint_of_node
                .get(ch.target().node().index())
                .copied()
                .flatten()
            else {
                continue;
            };
            let sampler = ch.sampler();
            let what = format!("clip {name}");
            let times = read_floats(&sampler.input(), bin, 1, keys_left, &what)?;
            keys_left -= times.len();
            if times.is_empty() {
                return Err(invalid(format!("{what}: a track has no keys")));
            }
            if times[0] < 0.0 || times.windows(2).any(|w| w[1] < w[0]) {
                return Err(invalid(format!(
                    "{what}: key times are negative or decreasing"
                )));
            }
            let interpolation = match sampler.interpolation() {
                GltfInterpolation::Linear => Interpolation::Linear,
                GltfInterpolation::Step => Interpolation::Step,
                GltfInterpolation::CubicSpline => Interpolation::CubicSpline,
            };
            let per_key = match interpolation {
                Interpolation::CubicSpline => 3,
                _ => 1,
            };
            let width = channel.width();
            let mut values =
                read_floats(&sampler.output(), bin, width, times.len() * per_key, &what)?;
            if values.len() != times.len() * per_key * width {
                return Err(invalid(format!(
                    "{what}: output count does not match its key times"
                )));
            }
            if channel == Channel::Rotation {
                // Unit quaternions; for splines only the value (the middle of each key).
                let value_at = if per_key == 3 { width } else { 0 };
                for key in values.chunks_exact_mut(per_key * width) {
                    let q = &mut key[value_at..value_at + width];
                    let len = q.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if !len.is_finite() || len <= 1e-6 {
                        return Err(invalid(format!("{what}: a zero rotation key")));
                    }
                    q.iter_mut().for_each(|x| *x /= len);
                }
            }
            duration = duration.max(times[times.len() - 1]);
            tracks.push(Track {
                joint,
                channel,
                interpolation,
                times,
                values,
            });
        }
        clips.push(Clip {
            name,
            duration,
            tracks,
        });
    }
    Ok(clips)
}
