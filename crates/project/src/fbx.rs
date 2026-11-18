//! FBX models (#832): binary FBX files (the format's 7.x versions, which every tool exports) decoded into a
//! [`MeshAsset`], as glTF models are ([`decode_mesh`](crate::decode_mesh) tells them apart by their first bytes).
//!
//! # What is read
//!
//! The file is a tree of named nodes, each with a list of typed properties; arrays may be zlib-compressed. Of it:
//!
//! - every `Objects/Geometry` of type `Mesh`: its control points (`Vertices`), its polygons (`PolygonVertexIndex`, the
//!   last corner of each stored as `-index - 1`), and its first normal and UV layers (`LayerElementNormal`,
//!   `LayerElementUV`) in any of the mapping and reference modes FBX has;
//! - every `Objects/Model`'s place: `Lcl Translation`, `PreRotation`, `Lcl Rotation` (degrees, X then Y then Z) and
//!   `Lcl Scaling`, and the `Connections` that hang geometries on models and models on one another, so a model's
//!   parts sit where they do in the file;
//! - `GlobalSettings`' `UpAxis` (a Z-up file is turned to Y-up) and `UnitScaleFactor` (centimetres per unit), for
//!   the mesh's size in metres.
//!
//! Polygons are cut into fans of triangles; each corner is a vertex of its own (so hard edges stay hard). A mesh with
//! no normals gets each triangle's. Materials, textures, skins and animation are not read.
//!
//! ASCII FBX files are refused, with a note to export binary.

use std::collections::HashMap;
use std::io::Read;

use metallic_core::glam::{EulerRot, Mat3, Mat4, Quat, Vec3};

use crate::assets::{DecodeError, MAX_MESH_VERTICES, MeshAsset};

/// The first bytes of a binary FBX file.
pub const FBX_MAGIC: &[u8] = b"Kaydara FBX Binary  \0";

/// How deep the node tree may go: deeper is a damaged file.
const MAX_DEPTH: usize = 64;

/// How large an array may be (elements): more is a damaged file, or one too large to load.
const MAX_ARRAY: usize = 1 << 26;

/// A property of a node.
#[derive(Clone, Debug, PartialEq)]
enum Prop {
    Int(i64),
    Float(f64),
    Text(String),
    Ints(Vec<i64>),
    Floats(Vec<f64>),
    /// Raw bytes, and arrays of booleans: never read.
    Other,
}

impl Prop {
    fn int(&self) -> Option<i64> {
        match self {
            Prop::Int(i) => Some(*i),
            Prop::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    fn float(&self) -> Option<f64> {
        match self {
            Prop::Float(f) => Some(*f),
            Prop::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Prop::Text(text) => Some(text),
            _ => None,
        }
    }

    fn floats(&self) -> Option<&[f64]> {
        match self {
            Prop::Floats(values) => Some(values),
            _ => None,
        }
    }

    fn ints(&self) -> Option<&[i64]> {
        match self {
            Prop::Ints(values) => Some(values),
            _ => None,
        }
    }
}

/// A node: its name, properties and children.
#[derive(Clone, Debug, Default)]
struct Node {
    name: String,
    props: Vec<Prop>,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// The text of the first property of the child `name`.
    fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name)?.props.first()?.text()
    }

    /// A `Properties70` entry's values (after its name and three type strings), by its name.
    fn property70(&self, name: &str) -> Option<&[Prop]> {
        self.child("Properties70")?
            .children_named("P")
            .find(|p| p.props.first().and_then(Prop::text) == Some(name))
            .map(|p| p.props.get(4..).unwrap_or_default())
    }

    fn vec3_of(&self, name: &str) -> Option<Vec3> {
        let values = self.property70(name)?;
        let get = |i: usize| values.get(i).and_then(Prop::float).map(|v| v as f32);
        Some(Vec3::new(get(0)?, get(1)?, get(2)?))
    }
}

/// Reads binary FBX bytes.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    /// Version 7500 on: offsets and counts are 64-bit.
    wide: bool,
}

fn damaged(what: &str) -> DecodeError {
    DecodeError::new(format!("the FBX file is damaged ({what})"))
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.at.checked_add(n).filter(|&end| end <= self.bytes.len());
        let end = end.ok_or_else(|| damaged("it ends early"))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// An offset or a count: 32 or 64 bits as the version says.
    fn word(&mut self) -> Result<u64, DecodeError> {
        if self.wide {
            self.u64()
        } else {
            self.u32().map(u64::from)
        }
    }

    /// The node at the reader, or `None` for the empty record that ends a list of nodes.
    fn node(&mut self, depth: usize) -> Result<Option<Node>, DecodeError> {
        if depth > MAX_DEPTH {
            return Err(damaged("its nodes nest too deep"));
        }
        let end = usize::try_from(self.word()?).map_err(|_| damaged("an offset"))?;
        let count = self.word()?;
        let _list_length = self.word()?;
        let name_length = usize::from(self.u8()?);
        if end == 0 {
            return Ok(None);
        }
        if end > self.bytes.len() || end < self.at {
            return Err(damaged("a node's end"));
        }
        let name = String::from_utf8_lossy(self.take(name_length)?).into_owned();
        let mut props = Vec::new();
        for _ in 0..count {
            props.push(self.prop()?);
        }
        let mut children = Vec::new();
        while self.at < end {
            match self.node(depth + 1)? {
                Some(child) => children.push(child),
                None => break,
            }
        }
        self.at = end;
        Ok(Some(Node { name, props, children }))
    }

    fn prop(&mut self) -> Result<Prop, DecodeError> {
        let kind = self.u8()?;
        Ok(match kind {
            b'Y' => {
                let b = self.take(2)?;
                Prop::Int(i64::from(i16::from_le_bytes([b[0], b[1]])))
            }
            b'C' => Prop::Int(i64::from(self.u8()?)),
            b'I' => Prop::Int(i64::from(self.u32()? as i32)),
            b'L' => Prop::Int(self.u64()? as i64),
            b'F' => {
                let b = self.take(4)?;
                Prop::Float(f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])))
            }
            b'D' => Prop::Float(f64::from_bits(self.u64()?)),
            b'S' => {
                let length = self.u32()? as usize;
                Prop::Text(String::from_utf8_lossy(self.take(length)?).into_owned())
            }
            b'R' => {
                let length = self.u32()? as usize;
                self.take(length)?;
                Prop::Other
            }
            b'f' | b'd' | b'l' | b'i' | b'b' => self.array(kind)?,
            other => return Err(damaged(&format!("a property of type `{}`", char::from(other)))),
        })
    }

    /// An array property of element type `kind`, inflated when it is compressed.
    fn array(&mut self, kind: u8) -> Result<Prop, DecodeError> {
        let length = self.u32()? as usize;
        let encoding = self.u32()?;
        let stored = self.u32()? as usize;
        if length > MAX_ARRAY {
            return Err(DecodeError::new(format!(
                "the FBX file has an array of more than {MAX_ARRAY} values"
            )));
        }
        let size = match kind {
            b'f' | b'i' => 4,
            b'd' | b'l' => 8,
            _ => 1,
        };
        let raw = self.take(stored)?;
        let data: Vec<u8> = match encoding {
            0 => raw.to_vec(),
            1 => {
                let mut out = Vec::with_capacity(length * size);
                flate2::read::ZlibDecoder::new(raw)
                    .take((length * size) as u64)
                    .read_to_end(&mut out)
                    .map_err(|_| damaged("a compressed array"))?;
                out
            }
            _ => return Err(damaged("an array's encoding")),
        };
        if data.len() < length * size {
            return Err(damaged("an array shorter than it says"));
        }
        let chunks = data.chunks_exact(size).take(length);
        Ok(match kind {
            b'f' => Prop::Floats(
                chunks
                    .map(|c| f64::from(f32::from_le_bytes([c[0], c[1], c[2], c[3]])))
                    .collect(),
            ),
            b'd' => Prop::Floats(
                chunks
                    .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
                    .collect(),
            ),
            b'i' => Prop::Ints(
                chunks
                    .map(|c| i64::from(i32::from_le_bytes([c[0], c[1], c[2], c[3]])))
                    .collect(),
            ),
            b'l' => Prop::Ints(
                chunks
                    .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
                    .collect(),
            ),
            _ => Prop::Other,
        })
    }
}

/// The top-level nodes of a binary FBX file.
fn parse(bytes: &[u8]) -> Result<Vec<Node>, DecodeError> {
    if !bytes.starts_with(FBX_MAGIC) {
        if bytes.starts_with(b"; FBX") {
            return Err(DecodeError::new(
                "the model is an ASCII FBX file, which Metallic does not read yet: export it as binary FBX",
            ));
        }
        return Err(DecodeError::new("not an FBX model"));
    }
    let mut reader = Reader {
        bytes,
        at: FBX_MAGIC.len() + 2,
        wide: false,
    };
    let version = reader.u32()?;
    reader.wide = version >= 7500;
    let mut nodes = Vec::new();
    while reader.at < bytes.len() {
        match reader.node(0)? {
            Some(node) => nodes.push(node),
            None => break,
        }
    }
    Ok(nodes)
}

/// A model's place relative to its parent: translation, pre-rotation, rotation (X, then Y, then Z) and scale.
fn local_matrix(model: &Node) -> Mat4 {
    let degrees = |v: Vec3| Vec3::new(v.x.to_radians(), v.y.to_radians(), v.z.to_radians());
    let euler = |v: Vec3| {
        let v = degrees(v);
        // X applied first: in matrix order Z * Y * X.
        Quat::from_euler(EulerRot::ZYX, v.z, v.y, v.x)
    };
    let translation = model.vec3_of("Lcl Translation").unwrap_or(Vec3::ZERO);
    let pre = model.vec3_of("PreRotation").map_or(Quat::IDENTITY, euler);
    let rotation = model.vec3_of("Lcl Rotation").map_or(Quat::IDENTITY, euler);
    let scale = model.vec3_of("Lcl Scaling").unwrap_or(Vec3::ONE);
    Mat4::from_scale_rotation_translation(scale, pre * rotation, translation)
}

/// The value of layer `element`'s data (`values`, `width` floats each) at polygon corner `corner` (its control point
/// `point`, in polygon `polygon`), as its mapping and reference modes say.
fn layer_value(element: &Node, values: &str, index: &str, width: usize, at: (usize, usize, usize)) -> Option<Vec<f32>> {
    let (corner, point, polygon) = at;
    let data = element.child(values)?.props.first()?.floats()?;
    let mapping = element.text_of("MappingInformationType").unwrap_or("ByPolygonVertex");
    let slot = match mapping {
        "ByPolygonVertex" => corner,
        "ByVertice" | "ByVertex" | "ByControlPoint" => point,
        "ByPolygon" => polygon,
        "AllSame" => 0,
        _ => return None,
    };
    let slot = match element.text_of("ReferenceInformationType").unwrap_or("Direct") {
        "IndexToDirect" | "Index" => usize::try_from(*element.child(index)?.props.first()?.ints()?.get(slot)?).ok()?,
        _ => slot,
    };
    let value = data.get(slot * width..slot * width + width)?;
    Some(value.iter().map(|&v| v as f32).collect())
}

/// Appends the triangles of the `geometry` node, placed by `world`, to `mesh`.
fn add_geometry(mesh: &mut MeshAsset, geometry: &Node, world: Mat4) -> Result<(), DecodeError> {
    let Some(points) = geometry
        .child("Vertices")
        .and_then(|n| n.props.first())
        .and_then(Prop::floats)
    else {
        return Ok(());
    };
    let Some(polygons) = geometry
        .child("PolygonVertexIndex")
        .and_then(|n| n.props.first())
        .and_then(Prop::ints)
    else {
        return Ok(());
    };
    let normals = geometry.child("LayerElementNormal");
    let uvs = geometry.child("LayerElementUV");
    let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
    let mirrored = Mat3::from_mat4(world).determinant() < 0.0;
    let point = |index: usize| -> Option<Vec3> {
        let p = points.get(index * 3..index * 3 + 3)?;
        Some(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32))
    };
    let mut corners: Vec<(usize, usize)> = Vec::new();
    let mut polygon = 0;
    for (corner, &raw) in polygons.iter().enumerate() {
        // The last corner of a polygon is stored as -index - 1.
        let last = raw < 0;
        let index = usize::try_from(if last { -raw - 1 } else { raw }).map_err(|_| damaged("a polygon"))?;
        corners.push((corner, index));
        if !last {
            continue;
        }
        if corners.len() >= 3 {
            let base = u32::try_from(mesh.positions.len()).map_err(|_| DecodeError::new("too many vertices"))?;
            let mut placed = Vec::with_capacity(corners.len());
            for &(corner, index) in &corners {
                let at = point(index).ok_or_else(|| damaged("a polygon names a missing vertex"))?;
                placed.push(world.transform_point3(at));
                let normal = normals
                    .and_then(|n| layer_value(n, "Normals", "NormalsIndex", 3, (corner, index, polygon)))
                    .map(|n| unit(normal_matrix * Vec3::new(n[0], n[1], n[2])));
                let uv = uvs
                    .and_then(|n| layer_value(n, "UV", "UVIndex", 2, (corner, index, polygon)))
                    .map_or([0.0, 0.0], |uv| [uv[0], 1.0 - uv[1]]);
                mesh.positions.push(world.transform_point3(at).to_array());
                mesh.normals.push(normal.unwrap_or(Vec3::ZERO).to_array());
                mesh.uvs.push(uv);
            }
            if mesh.positions.len() > MAX_MESH_VERTICES {
                return Err(DecodeError::new(format!(
                    "the model has more than {MAX_MESH_VERTICES} vertices"
                )));
            }
            // A corner without a normal takes the polygon's face normal (outward: a mirroring place turns it in).
            let face = unit((placed[1] - placed[0]).cross(placed[2] - placed[0]));
            let face = if mirrored { -face } else { face };
            for n in &mut mesh.normals[base as usize..] {
                if Vec3::from(*n) == Vec3::ZERO {
                    *n = face.to_array();
                }
            }
            for i in 1..corners.len() as u32 - 1 {
                let (b, c) = if mirrored { (i + 1, i) } else { (i, i + 1) };
                mesh.indices.extend([base, base + b, base + c]);
            }
        }
        corners.clear();
        polygon += 1;
    }
    Ok(())
}

fn unit(v: Vec3) -> Vec3 {
    v.try_normalize().unwrap_or(Vec3::Y)
}

/// Decodes a binary FBX model into one mesh (not yet fitted to the unit cube) and its size in metres per unit.
pub(crate) fn decode(bytes: &[u8]) -> Result<(MeshAsset, f32), DecodeError> {
    let nodes = parse(bytes)?;
    let top = |name: &str| nodes.iter().find(|node| node.name == name);
    let settings = top("GlobalSettings");
    // Centimetres per unit (1 is centimetres, 100 metres).
    let unit_scale = settings
        .and_then(|s| s.property70("UnitScaleFactor"))
        .and_then(|values| values.first())
        .and_then(Prop::float)
        .unwrap_or(1.0) as f32;
    let up = settings
        .and_then(|s| s.property70("UpAxis"))
        .and_then(|values| values.first())
        .and_then(Prop::int)
        .unwrap_or(1);
    // Z up (3ds Max): turned so the model stands up in Metallic's Y-up world.
    let axes = if up == 2 {
        Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
    } else {
        Mat4::IDENTITY
    };
    let objects = top("Objects").ok_or_else(|| DecodeError::new("the FBX file has no objects"))?;
    let id_of = |node: &Node| node.props.first().and_then(Prop::int);
    let models: HashMap<i64, &Node> = objects
        .children_named("Model")
        .filter_map(|model| Some((id_of(model)?, model)))
        .collect();
    // Child to parent, object to object.
    let mut parent: HashMap<i64, i64> = HashMap::new();
    if let Some(connections) = top("Connections") {
        for link in connections.children_named("C") {
            if link.props.first().and_then(Prop::text) != Some("OO") {
                continue;
            }
            if let (Some(child), Some(to)) = (
                link.props.get(1).and_then(Prop::int),
                link.props.get(2).and_then(Prop::int),
            ) {
                parent.insert(child, to);
            }
        }
    }
    // A model's place in the file: its own after its parents'.
    let world_of = |mut id: i64| {
        let mut world = Mat4::IDENTITY;
        for _ in 0..MAX_DEPTH {
            let Some(model) = models.get(&id) else {
                break;
            };
            world = local_matrix(model) * world;
            match parent.get(&id) {
                Some(&up) => id = up,
                None => break,
            }
        }
        world
    };
    let mut mesh = MeshAsset::default();
    for geometry in objects.children_named("Geometry") {
        if geometry
            .props
            .get(2)
            .and_then(Prop::text)
            .is_some_and(|kind| kind != "Mesh")
        {
            continue;
        }
        let world = id_of(geometry)
            .and_then(|id| parent.get(&id).copied())
            .map_or(Mat4::IDENTITY, world_of);
        add_geometry(&mut mesh, geometry, axes * world)?;
    }
    if mesh.indices.is_empty() {
        return Err(DecodeError::new("the model has no triangles"));
    }
    Ok((mesh, unit_scale / 100.0))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    /// A property, as a binary FBX file holds it.
    pub(crate) enum Out<'a> {
        Int(i32),
        Long(i64),
        Double(f64),
        Text(&'a str),
        Doubles(&'a [f64], bool),
        Ints(&'a [i32], bool),
    }

    /// A node to write: its name, properties and children.
    pub(crate) struct OutNode<'a>(pub &'a str, pub Vec<Out<'a>>, pub Vec<OutNode<'a>>);

    fn array(out: &mut Vec<u8>, kind: u8, count: usize, raw: Vec<u8>, compressed: bool) {
        out.push(kind);
        out.extend((count as u32).to_le_bytes());
        let data = if compressed {
            let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&raw).unwrap();
            encoder.finish().unwrap()
        } else {
            raw
        };
        out.extend(u32::from(compressed).to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
    }

    fn write_node(out: &mut Vec<u8>, node: &OutNode<'_>) {
        let start = out.len();
        out.extend([0u8; 12]);
        out.push(node.0.len() as u8);
        out.extend(node.0.as_bytes());
        let props_start = out.len();
        for prop in &node.1 {
            match prop {
                Out::Int(i) => {
                    out.push(b'I');
                    out.extend(i.to_le_bytes());
                }
                Out::Long(l) => {
                    out.push(b'L');
                    out.extend(l.to_le_bytes());
                }
                Out::Double(d) => {
                    out.push(b'D');
                    out.extend(d.to_le_bytes());
                }
                Out::Text(s) => {
                    out.push(b'S');
                    out.extend((s.len() as u32).to_le_bytes());
                    out.extend(s.as_bytes());
                }
                Out::Doubles(values, compressed) => {
                    let raw = values.iter().flat_map(|v| v.to_le_bytes()).collect();
                    array(out, b'd', values.len(), raw, *compressed);
                }
                Out::Ints(values, compressed) => {
                    let raw = values.iter().flat_map(|v| v.to_le_bytes()).collect();
                    array(out, b'i', values.len(), raw, *compressed);
                }
            }
        }
        let list = out.len() - props_start;
        if !node.2.is_empty() {
            for child in &node.2 {
                write_node(out, child);
            }
            out.extend([0u8; 13]);
        }
        let end = out.len() as u32;
        out[start..start + 4].copy_from_slice(&end.to_le_bytes());
        out[start + 4..start + 8].copy_from_slice(&(node.1.len() as u32).to_le_bytes());
        out[start + 8..start + 12].copy_from_slice(&(list as u32).to_le_bytes());
    }

    /// A binary FBX file (version 7400) of `nodes`.
    pub(crate) fn fbx_file(nodes: &[OutNode<'_>]) -> Vec<u8> {
        let mut out = FBX_MAGIC.to_vec();
        out.extend([0x1a, 0x00]);
        out.extend(7400u32.to_le_bytes());
        for node in nodes {
            write_node(&mut out, node);
        }
        out.extend([0u8; 13]);
        out
    }

    /// A `Properties70` entry of three numbers.
    fn p<'a>(name: &'a str, v: [f64; 3]) -> OutNode<'a> {
        OutNode(
            "P",
            vec![
                Out::Text(name),
                Out::Text("Lcl"),
                Out::Text(""),
                Out::Text("A"),
                Out::Double(v[0]),
                Out::Double(v[1]),
                Out::Double(v[2]),
            ],
            vec![],
        )
    }

    /// A cube of side 2 (eight corners, six quads), compressed or not, with its model moved by `offset` and scaled by
    /// `scale`, in a file whose units are `unit` centimetres.
    pub(crate) fn cube_fbx(offset: [f64; 3], scale: [f64; 3], compressed: bool, unit: f64) -> Vec<u8> {
        const POINTS: [f64; 24] = [
            -1.0, -1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, -1.0, //
            -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0, 1.0, 1.0, -1.0, 1.0, 1.0,
        ];
        // Six quads, wound counter-clockwise seen from outside; each one's last corner as -index - 1.
        const QUADS: [i32; 24] = [
            0, 3, 2, -2, 4, 5, 6, -8, 0, 1, 5, -5, 3, 7, 6, -3, 0, 4, 7, -4, 1, 2, 6, -6,
        ];
        const UV: [f64; 8] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        const UV_INDEX: [i32; 24] = [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3];
        let geometry = OutNode(
            "Geometry",
            vec![Out::Long(10), Out::Text("Cube\0\u{1}Geometry"), Out::Text("Mesh")],
            vec![
                OutNode("Vertices", vec![Out::Doubles(&POINTS, compressed)], vec![]),
                OutNode("PolygonVertexIndex", vec![Out::Ints(&QUADS, compressed)], vec![]),
                OutNode(
                    "LayerElementUV",
                    vec![Out::Int(0)],
                    vec![
                        OutNode("MappingInformationType", vec![Out::Text("ByPolygonVertex")], vec![]),
                        OutNode("ReferenceInformationType", vec![Out::Text("IndexToDirect")], vec![]),
                        OutNode("UV", vec![Out::Doubles(&UV, compressed)], vec![]),
                        OutNode("UVIndex", vec![Out::Ints(&UV_INDEX, compressed)], vec![]),
                    ],
                ),
            ],
        );
        let model = OutNode(
            "Model",
            vec![Out::Long(20), Out::Text("Cube\0\u{1}Model"), Out::Text("Mesh")],
            vec![OutNode(
                "Properties70",
                vec![],
                vec![p("Lcl Translation", offset), p("Lcl Scaling", scale)],
            )],
        );
        let settings = OutNode(
            "GlobalSettings",
            vec![],
            vec![OutNode(
                "Properties70",
                vec![],
                vec![OutNode(
                    "P",
                    vec![
                        Out::Text("UnitScaleFactor"),
                        Out::Text("double"),
                        Out::Text("Number"),
                        Out::Text(""),
                        Out::Double(unit),
                    ],
                    vec![],
                )],
            )],
        );
        let objects = OutNode("Objects", vec![], vec![geometry, model]);
        let connections = OutNode(
            "Connections",
            vec![],
            vec![
                OutNode("C", vec![Out::Text("OO"), Out::Long(10), Out::Long(20)], vec![]),
                OutNode("C", vec![Out::Text("OO"), Out::Long(20), Out::Long(0)], vec![]),
            ],
        );
        fbx_file(&[settings, objects, connections])
    }

    #[test]
    fn a_cube_decodes_to_twelve_triangles_placed_as_its_model_says() {
        for compressed in [false, true] {
            let (mesh, metres) = decode(&cube_fbx([5.0, 0.0, 0.0], [1.0, 3.0, 1.0], compressed, 100.0)).unwrap();
            assert_eq!(mesh.triangles(), 12, "compressed: {compressed}");
            assert_eq!(mesh.positions.len(), 24, "a vertex for each quad's corner");
            assert_eq!(metres, 1.0, "100 centimetres a unit");
            let xs: Vec<f32> = mesh.positions.iter().map(|p| p[0]).collect();
            let ys: Vec<f32> = mesh.positions.iter().map(|p| p[1]).collect();
            let span =
                |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min);
            assert!((span(&xs) - 2.0).abs() < 1e-5 && (span(&ys) - 6.0).abs() < 1e-5);
            assert!(xs.iter().all(|&x| (4.0..=6.0).contains(&x)), "moved by its model");
            // Face normals, outward: the first quad faces -Z.
            assert_eq!(mesh.normals[0], [0.0, 0.0, -1.0]);
            // UVs from the layer, flipped to top-left.
            assert_eq!(mesh.uvs[0], [0.0, 1.0]);
        }
    }

    #[test]
    fn what_is_not_a_binary_fbx_file_says_so() {
        assert!(
            parse(b"; FBX 7.4.0 project file")
                .unwrap_err()
                .to_string()
                .contains("ASCII")
        );
        assert!(parse(b"not a model").is_err());
        let mut cut = cube_fbx([0.0; 3], [1.0; 3], true, 1.0);
        cut.truncate(cut.len() / 2);
        assert!(decode(&cut).is_err());
    }
}

#[cfg(test)]
mod decode_tests {
    use super::tests::cube_fbx;

    /// #832: through `decode_mesh`, as every caller reads a mesh: fitted to the unit cube, its size in metres.
    #[test]
    fn an_fbx_model_is_a_mesh_asset() {
        // One centimetre a unit: the cube of side 2 is 2 cm across, 6 cm tall.
        let model = cube_fbx([0.0; 3], [1.0, 3.0, 1.0], true, 1.0);
        let mesh = crate::decode_mesh(&model, |_| Err(String::new())).unwrap();
        assert_eq!(mesh.triangles(), 12);
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(close(mesh.size, [0.02, 0.06, 0.02]), "{:?}", mesh.size);
        assert!(
            mesh.positions
                .iter()
                .all(|p| p.iter().all(|c| (-0.5..=0.5).contains(c)))
        );
    }
}
