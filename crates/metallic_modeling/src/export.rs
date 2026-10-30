//! Export: glTF 2.0 (binary `.glb` or text `.gltf` with an embedded buffer) from [`MeshData`], and Wavefront OBJ
//! straight from the polygon mesh (n-gons kept, for debugging in other tools).

use std::fmt::Write as _;

use serde_json::json;

use crate::error::{ModelError, ModelResult};
use crate::mesh::Mesh;
use crate::triangulate::MeshData;

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;

/// Packs the vertex streams and indices into one little-endian buffer and builds the glTF document for it.
fn document(
    data: &MeshData,
    name: &str,
    uri: Option<String>,
) -> ModelResult<(serde_json::Value, Vec<u8>)> {
    let n = data.positions.len();
    if data.normals.len() != n || data.uv0.len() != n {
        return Err(ModelError::Export("vertex streams differ in length".into()));
    }
    if data.indices.is_empty() || !data.indices.len().is_multiple_of(3) {
        return Err(ModelError::Export("no triangles to export".into()));
    }
    if data.indices.iter().any(|&i| i as usize >= n) {
        return Err(ModelError::Export("index out of range".into()));
    }
    let mut bin: Vec<u8> = Vec::with_capacity(n * 32 + data.indices.len() * 4);
    let mut views = Vec::new();
    let mut push_view = |bin: &mut Vec<u8>, bytes: Vec<u8>, target: u32| {
        let offset = bin.len();
        let len = bytes.len();
        bin.extend(bytes);
        views.push(
            json!({ "buffer": 0, "byteOffset": offset, "byteLength": len, "target": target }),
        );
    };
    let floats = |v: &mut Vec<u8>, xs: &[f32]| {
        xs.iter()
            .for_each(|x| v.extend_from_slice(&x.to_le_bytes()))
    };
    let mut b = Vec::new();
    data.positions.iter().for_each(|p| floats(&mut b, &p[..]));
    push_view(&mut bin, b, ARRAY_BUFFER);
    let mut b = Vec::new();
    data.normals.iter().for_each(|p| floats(&mut b, &p[..]));
    push_view(&mut bin, b, ARRAY_BUFFER);
    let mut b = Vec::new();
    data.uv0.iter().for_each(|p| floats(&mut b, &p[..]));
    push_view(&mut bin, b, ARRAY_BUFFER);
    let mut b = Vec::new();
    data.indices
        .iter()
        .for_each(|i| b.extend_from_slice(&i.to_le_bytes()));
    push_view(&mut bin, b, ELEMENT_ARRAY_BUFFER);
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &data.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let mut buffer = json!({ "byteLength": bin.len() });
    if let Some(uri) = uri {
        buffer["uri"] = json!(uri);
    }
    let doc = json!({
        "asset": { "version": "2.0", "generator": "Metallic modeller (metallic_modeling)" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": name }],
        "meshes": [{
            "name": name,
            "primitives": [{
                "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
                "indices": 3,
                "mode": 4
            }]
        }],
        "buffers": [buffer],
        "bufferViews": views,
        "accessors": [
            { "bufferView": 0, "componentType": FLOAT, "count": n, "type": "VEC3", "min": lo, "max": hi },
            { "bufferView": 1, "componentType": FLOAT, "count": n, "type": "VEC3" },
            { "bufferView": 2, "componentType": FLOAT, "count": n, "type": "VEC2" },
            { "bufferView": 3, "componentType": UNSIGNED_INT, "count": data.indices.len(), "type": "SCALAR" }
        ]
    });
    Ok((doc, bin))
}

/// A binary glTF (`.glb`) holding one mesh.
///
/// # Errors
/// [`ModelError::Export`] for empty or inconsistent data.
pub fn to_glb(data: &MeshData, name: &str) -> ModelResult<Vec<u8>> {
    let (doc, mut bin) = document(data, name, None)?;
    let mut json_bytes = serde_json::to_vec(&doc).map_err(|e| ModelError::Export(e.to_string()))?;
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let total =
        u32::try_from(total).map_err(|_| ModelError::Export("mesh too large for glb".into()))?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    Ok(out)
}

/// A text glTF (`.gltf`) with the buffer embedded as a base64 data URI.
///
/// # Errors
/// [`ModelError::Export`] for empty or inconsistent data.
pub fn to_gltf(data: &MeshData, name: &str) -> ModelResult<String> {
    let (_, bin) = document(data, name, None)?;
    let uri = format!("data:application/octet-stream;base64,{}", base64(&bin));
    let (doc, _) = document(data, name, Some(uri))?;
    serde_json::to_string_pretty(&doc).map_err(|e| ModelError::Export(e.to_string()))
}

/// Wavefront OBJ of the polygon mesh: `v`, `vt` (one per face corner), `vn` (one per face) and `f` with n-gons.
/// Edge marks are not representable and are dropped.
#[must_use]
pub fn to_obj(mesh: &Mesh, name: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Metallic modeller export\no {name}");
    let mut index = std::collections::BTreeMap::new();
    for (i, (v, vert)) in mesh.verts().enumerate() {
        let p = vert.position;
        let _ = writeln!(s, "v {} {} {}", p.x, p.y, p.z);
        index.insert(v, i + 1);
    }
    let mut vt = 0usize;
    let mut body = String::new();
    for (k, (f, face)) in mesh.faces().enumerate() {
        let n = mesh.face_normal(f);
        let _ = writeln!(s, "vn {} {} {}", n.x, n.y, n.z);
        let mut line = String::from("f");
        for (v, uv) in face.verts.iter().zip(&face.uvs) {
            vt += 1;
            let _ = writeln!(s, "vt {} {}", uv.x, uv.y);
            let _ = write!(
                line,
                " {}/{}/{}",
                index.get(v).copied().unwrap_or(1),
                vt,
                k + 1
            );
        }
        body.push_str(&line);
        body.push('\n');
    }
    s.push_str(&body);
    s
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
