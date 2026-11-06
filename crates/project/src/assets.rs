//! Reading and decoding a project's models and textures (#189): glTF and GLB meshes, PNG and JPEG images.
//!
//! [`Project::asset_reader`](crate::Project::asset_reader) makes an [`AssetReader`], which reads assets by
//! [`AssetId`] on any thread (the engine decodes on a worker, never on the frame). [`decode_mesh`] and
//! [`decode_image`] turn a member's bytes into plain data the engine hands to the renderer: this crate never depends
//! on the renderer (ADR 0001).
//!
//! # Meshes
//!
//! A static mesh with one material: every triangle primitive of every mesh the model's scene places (its default
//! scene, or else its first; a model without scenes gives its meshes as they are), with the nodes' transforms
//! applied, merged into one indexed mesh of positions, normals and the first set of texture coordinates. Normals a
//! primitive lacks are made from its triangles; texture coordinates it lacks are 0. Skins, morph targets, animations,
//! materials and lines or points are not read.
//!
//! The mesh is **fitted to the unit cube**, centred on the origin with its bounds spanning −0.5 to 0.5 on every axis
//! (an axis it is flat along stays flat), as the renderer's meshes are: a `MeshPart` stretches it to its `Size`, so
//! its bounds fill the part. The size it had is kept ([`MeshAsset::size`]).
//!
//! A GLB holds its buffers; a `.gltf` file's buffers are base64 `data:` URIs or other members of the project, named
//! relative to the model's own folder.
//!
//! # Images
//!
//! PNG (any colour type and bit depth) and baseline or progressive JPEG, told apart by their first bytes, decoded to
//! 8-bit RGBA in sRGB, rows from the top. [`ImageAsset::fit`] halves an image too large for a device.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read};
use std::path::{Path, PathBuf};

use metallic_core::glam::{Mat3, Mat4, Vec3};
use zip::ZipArchive;

use crate::asset_index::{AssetId, AssetIndex};

/// The most vertices a mesh may have, all of its primitives together: more is refused rather than risking the
/// memory (a mesh this large wants the levels of detail made at import, ADR 0008 §5).
pub const MAX_MESH_VERTICES: usize = 1 << 22;

/// The most triangles a mesh may have, all of its primitives together.
pub const MAX_MESH_TRIANGLES: usize = 1 << 23;

/// The widest or tallest image decoded, in pixels.
pub const MAX_IMAGE_SIZE: u32 = 16384;

/// How deep a model's node tree may go: deeper is refused, so a malformed file cannot exhaust the stack.
const MAX_NODE_DEPTH: usize = 64;

/// A decoded mesh: one indexed triangle list, fitted to the unit cube (see the [module documentation](self)).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshAsset {
    /// Vertex positions, inside the unit cube (−0.5 to 0.5 on every axis).
    pub positions: Vec<[f32; 3]>,
    /// Unit vertex normals, one per position.
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates, one per position: (0, 0) is the image's top left corner.
    pub uvs: Vec<[f32; 2]>,
    /// Three indices per triangle into the vertices, wound counter-clockwise seen from the front.
    pub indices: Vec<u32>,
    /// The size of the mesh's bounds before it was fitted to the unit cube, in the model's units (metres).
    pub size: [f32; 3],
}

impl MeshAsset {
    /// How many triangles the mesh has.
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A decoded image: 8-bit RGBA, sRGB-encoded, rows from the top.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageAsset {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width × height` pixels of red, green, blue and alpha, one byte each.
    pub rgba: Vec<u8>,
}

impl ImageAsset {
    /// Halves the image (averaging each 2 × 2 block, in linear light) until neither side is larger than `max`, for a
    /// device whose textures cannot be that large. Does nothing to an image that fits.
    pub fn fit(&mut self, max: u32) {
        let max = max.max(1);
        while self.width > max || self.height > max {
            self.halve();
        }
    }

    fn halve(&mut self) {
        let (width, height) = (self.width.div_ceil(2), self.height.div_ceil(2));
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        let pixel = |x: u32, y: u32| {
            let x = x.min(self.width - 1) as usize;
            let y = y.min(self.height - 1) as usize;
            let at = (y * self.width as usize + x) * 4;
            self.rgba.get(at..at + 4).unwrap_or(&[0; 4])
        };
        for y in 0..height {
            for x in 0..width {
                let block = [
                    pixel(2 * x, 2 * y),
                    pixel(2 * x + 1, 2 * y),
                    pixel(2 * x, 2 * y + 1),
                    pixel(2 * x + 1, 2 * y + 1),
                ];
                for channel in 0..3 {
                    let sum: f32 = block.iter().map(|p| srgb_to_linear(p[channel])).sum();
                    rgba.push(linear_to_srgb(sum / 4.0));
                }
                let alpha: u32 = block.iter().map(|p| u32::from(p[3])).sum();
                rgba.push(((alpha + 2) / 4) as u8);
            }
        }
        *self = Self { width, height, rgba };
    }
}

fn srgb_to_linear(value: u8) -> f32 {
    let c = f32::from(value) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> u8 {
    let c = value.clamp(0.0, 1.0);
    let encoded = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

/// Why bytes could not be decoded as a mesh or an image.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DecodeError(String);

impl DecodeError {
    fn new(reason: impl Into<String>) -> Self {
        Self(reason.into())
    }
}

/// Decodes a glTF (`.gltf`, JSON) or GLB model into one mesh fitted to the unit cube (see the
/// [module documentation](self)). `buffer` returns the bytes of a buffer the model names by a relative URI (a
/// `.gltf` file's `.bin`), or why it cannot.
pub fn decode_mesh(
    bytes: &[u8],
    mut buffer: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<MeshAsset, DecodeError> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|error| DecodeError::new(format!("not a glTF model: {error}")))?;
    let mut buffers: Vec<Cow<'_, [u8]>> = Vec::new();
    for item in gltf.buffers() {
        let data: Cow<'_, [u8]> = match item.source() {
            gltf::buffer::Source::Bin => Cow::Borrowed(
                gltf.blob
                    .as_deref()
                    .ok_or_else(|| DecodeError::new("a buffer is the GLB's binary chunk, which it does not have"))?,
            ),
            gltf::buffer::Source::Uri(uri) => match uri.strip_prefix("data:") {
                Some(data) => Cow::Owned(data_uri(data)?),
                None => {
                    Cow::Owned(buffer(uri).map_err(|reason| DecodeError::new(format!("buffer `{uri}`: {reason}")))?)
                }
            },
        };
        if data.len() < item.length() {
            return Err(DecodeError::new(format!(
                "buffer {} holds {} bytes, fewer than the {} it declares",
                item.index(),
                data.len(),
                item.length()
            )));
        }
        buffers.push(data);
    }

    let mut mesh = MeshAsset::default();
    let scene = gltf.default_scene().or_else(|| gltf.scenes().next());
    match scene {
        Some(scene) => {
            for node in scene.nodes() {
                add_node(&mut mesh, &buffers, &node, Mat4::IDENTITY, 0)?;
            }
        }
        None => {
            for item in gltf.meshes() {
                add_mesh(&mut mesh, &buffers, &item, Mat4::IDENTITY)?;
            }
        }
    }
    if mesh.indices.is_empty() {
        return Err(DecodeError::new("the model has no triangles"));
    }
    fit_to_unit_cube(&mut mesh);
    Ok(mesh)
}

/// The bytes of a base64 `data:` URI (after `data:`).
fn data_uri(data: &str) -> Result<Vec<u8>, DecodeError> {
    let (header, payload) = data
        .split_once(',')
        .ok_or_else(|| DecodeError::new("a data URI without a `,`"))?;
    if !header.ends_with(";base64") {
        return Err(DecodeError::new("a data URI that is not base64"));
    }
    base64(payload).ok_or_else(|| DecodeError::new("a data URI that is not valid base64"))
}

/// Decodes standard base64, padded or not.
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut count) = (0u32, 0u32);
    for byte in text.trim_end_matches('=').bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    Some(out)
}

fn add_node(
    mesh: &mut MeshAsset,
    buffers: &[Cow<'_, [u8]>],
    node: &gltf::Node<'_>,
    parent: Mat4,
    depth: usize,
) -> Result<(), DecodeError> {
    if depth > MAX_NODE_DEPTH {
        return Err(DecodeError::new(format!(
            "the node tree is deeper than {MAX_NODE_DEPTH} (or loops)"
        )));
    }
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(item) = node.mesh() {
        add_mesh(mesh, buffers, &item, world)?;
    }
    for child in node.children() {
        add_node(mesh, buffers, &child, world, depth + 1)?;
    }
    Ok(())
}

/// Appends every triangle primitive of `item`, placed by `world`.
fn add_mesh(
    mesh: &mut MeshAsset,
    buffers: &[Cow<'_, [u8]>],
    item: &gltf::Mesh<'_>,
    world: Mat4,
) -> Result<(), DecodeError> {
    let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
    // A mirroring transform turns the triangles inside out; their winding is flipped back.
    let mirrored = Mat3::from_mat4(world).determinant() < 0.0;
    for primitive in item.primitives() {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            continue;
        }
        let reader = primitive.reader(|b| buffers.get(b.index()).map(|data| &data[..]));
        let Some(positions) = reader.read_positions() else {
            continue;
        };
        let base = mesh.positions.len();
        mesh.positions
            .extend(positions.map(|p| world.transform_point3(Vec3::from(p)).to_array()));
        let count = mesh.positions.len() - base;
        if mesh.positions.len() > MAX_MESH_VERTICES {
            return Err(DecodeError::new(format!(
                "the model has more than {MAX_MESH_VERTICES} vertices"
            )));
        }
        let first_index = mesh.indices.len();
        let base_index = u32::try_from(base).map_err(|_| DecodeError::new("too many vertices"))?;
        match reader.read_indices() {
            Some(indices) => {
                for index in indices.into_u32() {
                    if index as usize >= count {
                        return Err(DecodeError::new(format!(
                            "an index ({index}) past the primitive's {count} vertices"
                        )));
                    }
                    mesh.indices.push(base_index + index);
                }
            }
            None => mesh.indices.extend((0..count as u32).map(|i| base_index + i)),
        }
        // A trailing partial triangle is dropped.
        let whole = (mesh.indices.len() - first_index) / 3 * 3;
        mesh.indices.truncate(first_index + whole);
        if mesh.indices.len() / 3 > MAX_MESH_TRIANGLES {
            return Err(DecodeError::new(format!(
                "the model has more than {MAX_MESH_TRIANGLES} triangles"
            )));
        }
        if mirrored {
            for triangle in mesh.indices[first_index..].as_chunks_mut::<3>().0 {
                triangle.swap(1, 2);
            }
        }
        match reader.read_normals() {
            Some(normals) => {
                let start = mesh.normals.len();
                mesh.normals
                    .extend(normals.map(|n| unit(normal_matrix * Vec3::from(n)).to_array()));
                // A short normal accessor leaves the rest to be made from the triangles.
                mesh.normals.resize(start + count, [0.0; 3]);
            }
            None => {
                mesh.normals.resize(base + count, [0.0; 3]);
                smooth_normals(mesh, base, first_index);
            }
        }
        match reader.read_tex_coords(0) {
            Some(uvs) => {
                let start = mesh.uvs.len();
                mesh.uvs.extend(uvs.into_f32());
                mesh.uvs.resize(start + count, [0.0; 2]);
            }
            None => mesh.uvs.resize(base + count, [0.0; 2]),
        }
    }
    Ok(())
}

/// `v` scaled to unit length, or +Y when it has no length.
fn unit(v: Vec3) -> Vec3 {
    v.try_normalize().unwrap_or(Vec3::Y)
}

/// Gives the vertices from `base` on the area-weighted mean normal of the triangles (from index `first_index` on)
/// that use them.
fn smooth_normals(mesh: &mut MeshAsset, base: usize, first_index: usize) {
    let mut sums = vec![Vec3::ZERO; mesh.positions.len() - base];
    for triangle in mesh.indices[first_index..].as_chunks::<3>().0 {
        let p = |i: u32| Vec3::from(mesh.positions[i as usize]);
        let face = (p(triangle[1]) - p(triangle[0])).cross(p(triangle[2]) - p(triangle[0]));
        for &i in triangle {
            if let Some(sum) = sums.get_mut(i as usize - base) {
                *sum += face;
            }
        }
    }
    for (normal, sum) in mesh.normals[base..].iter_mut().zip(sums) {
        *normal = unit(sum).to_array();
    }
}

/// Moves and stretches the mesh so its bounds are the unit cube centred on the origin (an axis it is flat along stays
/// flat), recording the size it had.
fn fit_to_unit_cube(mesh: &mut MeshAsset) {
    let (min, max) = mesh.positions.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(min, max), &p| (min.min(Vec3::from(p)), max.max(Vec3::from(p))),
    );
    let size = max - min;
    let centre = (min + max) * 0.5;
    let scale = Vec3::select(size.cmpgt(Vec3::splat(1e-12)), size.recip(), Vec3::ONE);
    for p in &mut mesh.positions {
        *p = ((Vec3::from(*p) - centre) * scale).to_array();
    }
    // Normals follow the inverse transpose of the stretch: divided by it.
    for n in &mut mesh.normals {
        *n = unit(Vec3::from(*n) / scale).to_array();
    }
    mesh.size = size.to_array();
}

/// The first bytes of every PNG file.
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Decodes a PNG or JPEG image (told apart by its first bytes) to 8-bit sRGB RGBA.
pub fn decode_image(bytes: &[u8]) -> Result<ImageAsset, DecodeError> {
    let image = if bytes.starts_with(&PNG_SIGNATURE) {
        decode_png(bytes)?
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        decode_jpeg(bytes)?
    } else {
        return Err(DecodeError::new("not a PNG or JPEG image"));
    };
    let expected = image.width as usize * image.height as usize * 4;
    if image.width == 0 || image.height == 0 || image.rgba.len() != expected {
        return Err(DecodeError::new("the image has no pixels"));
    }
    Ok(image)
}

fn decode_png(bytes: &[u8]) -> Result<ImageAsset, DecodeError> {
    let error = |error: png::DecodingError| DecodeError::new(format!("not a valid PNG: {error}"));
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    // Palettes and low bit depths expanded, 16 bits cut to 8: every image arrives as 8-bit grey or colour.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(error)?;
    let (width, height) = reader.info().size();
    if width > MAX_IMAGE_SIZE || height > MAX_IMAGE_SIZE {
        return Err(DecodeError::new(format!(
            "the image is {width} × {height}, larger than {MAX_IMAGE_SIZE} on a side"
        )));
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| DecodeError::new("the image is too large"))?;
    let mut data = vec![0; size];
    let info = reader.next_frame(&mut data).map_err(error)?;
    data.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => data,
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return Err(DecodeError::new("a PNG palette that could not be expanded")),
    };
    Ok(ImageAsset {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn decode_jpeg(bytes: &[u8]) -> Result<ImageAsset, DecodeError> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default()
        .set_max_width(MAX_IMAGE_SIZE as usize)
        .set_max_height(MAX_IMAGE_SIZE as usize)
        .jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let pixels = decoder
        .decode()
        .map_err(|error| DecodeError::new(format!("not a valid JPEG: {error:?}")))?;
    let (width, height) = decoder
        .dimensions()
        .ok_or_else(|| DecodeError::new("a JPEG without a size"))?;
    let rgba = match decoder.output_colorspace() {
        Some(ColorSpace::RGBA) => pixels,
        Some(ColorSpace::RGB) => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        Some(ColorSpace::Luma) => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => {
            return Err(DecodeError::new(format!(
                "a JPEG in an unsupported colour space ({other:?})"
            )));
        }
    };
    Ok(ImageAsset {
        width: u32::try_from(width).map_err(|_| DecodeError::new("the image is too large"))?,
        height: u32::try_from(height).map_err(|_| DecodeError::new("the image is too large"))?,
        rgba,
    })
}

/// Why an asset could not be loaded. Every message names the asset.
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// The text is not an asset id.
    #[error("`{id}` is not an asset id (32 hex digits)")]
    BadId {
        /// The text.
        id: String,
    },
    /// The project's asset index has no asset with the id.
    #[error("the project has no asset {id}")]
    UnknownId {
        /// The id.
        id: AssetId,
    },
    /// The asset is in the index, but its member is gone from the project.
    #[error("asset {id} (`{member}`) is missing from the project")]
    Missing {
        /// The id.
        id: AssetId,
        /// The member it had.
        member: String,
    },
    /// The member could not be read (the project file is gone or damaged).
    #[error("could not read `{member}`: {source}")]
    Read {
        /// The member.
        member: String,
        /// Why.
        source: io::Error,
    },
    /// The member is not a mesh or image this engine reads.
    #[error("`{member}` could not be decoded: {source}")]
    Decode {
        /// The member.
        member: String,
        /// Why.
        source: DecodeError,
    },
}

/// Reads a project's assets by id on any thread, from the project file and the edits not saved when it was made
/// ([`Project::asset_reader`](crate::Project::asset_reader)). It opens the file again on first use, so it never shares
/// a file handle with the project.
pub struct AssetReader {
    path: PathBuf,
    index: AssetIndex,
    /// Asset members edited or removed in memory when the reader was made.
    edits: BTreeMap<String, Option<Vec<u8>>>,
    archive: Option<ZipArchive<BufReader<File>>>,
}

impl AssetReader {
    /// A reader of the project at `path` with `index`, and `edits` (member contents, `None` for removed ones) over
    /// what the file holds.
    pub(crate) fn new(path: &Path, index: AssetIndex, edits: BTreeMap<String, Option<Vec<u8>>>) -> Self {
        Self {
            path: path.to_path_buf(),
            index,
            edits,
            archive: None,
        }
    }

    /// The project's file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The member path of the asset `id` names (32 hex digits).
    pub fn member(&self, id: &str) -> Result<(AssetId, &str), AssetError> {
        let parsed: AssetId = id.parse().map_err(|_| AssetError::BadId { id: id.to_owned() })?;
        let member = self.index.path(parsed).ok_or(AssetError::UnknownId { id: parsed })?;
        Ok((parsed, member))
    }

    /// The bytes of the asset `id` names, with its member path.
    pub fn read(&mut self, id: &str) -> Result<(String, Vec<u8>), AssetError> {
        let (parsed, member) = self.member(id)?;
        let member = member.to_owned();
        match self.read_member(&member) {
            Ok(bytes) => Ok((member, bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(AssetError::Missing { id: parsed, member }),
            Err(source) => Err(AssetError::Read { member, source }),
        }
    }

    /// Reads and decodes the mesh asset `id` names ([`decode_mesh`]); a `.gltf` file's other buffers are read from
    /// the members its URIs name, relative to its own folder.
    pub fn load_mesh(&mut self, id: &str) -> Result<MeshAsset, AssetError> {
        let (member, bytes) = self.read(id)?;
        let folder = member.rsplit_once('/').map_or("", |(folder, _)| folder).to_owned();
        decode_mesh(&bytes, |uri| {
            let path = if folder.is_empty() {
                uri.to_owned()
            } else {
                format!("{folder}/{uri}")
            };
            crate::project::check_member_path(&path).map_err(str::to_owned)?;
            self.read_member(&path).map_err(|error| error.to_string())
        })
        .map_err(|source| AssetError::Decode { member, source })
    }

    /// Reads and decodes the image asset `id` names ([`decode_image`]).
    pub fn load_image(&mut self, id: &str) -> Result<ImageAsset, AssetError> {
        let (member, bytes) = self.read(id)?;
        decode_image(&bytes).map_err(|source| AssetError::Decode { member, source })
    }

    /// The bytes of `member`: its edit, or what the file holds. `NotFound` when it has neither.
    fn read_member(&mut self, member: &str) -> io::Result<Vec<u8>> {
        match self.edits.get(member) {
            Some(Some(bytes)) => return Ok(bytes.clone()),
            Some(None) => return Err(io::ErrorKind::NotFound.into()),
            None => {}
        }
        let archive = match self.archive.as_mut() {
            Some(archive) => archive,
            None => {
                let file = File::open(&self.path)?;
                let archive = ZipArchive::new(BufReader::new(file)).map_err(io::Error::other)?;
                self.archive.insert(archive)
            }
        };
        let mut file = match archive.by_name(member) {
            Ok(file) => file,
            Err(zip::result::ZipError::FileNotFound) => return Err(io::ErrorKind::NotFound.into()),
            Err(zip::result::ZipError::Io(error)) => return Err(error),
            Err(other) => return Err(io::Error::other(other)),
        };
        let mut bytes = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0).min(1 << 26));
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

impl std::fmt::Debug for AssetReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetReader")
            .field("path", &self.path)
            .field("assets", &self.index.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes_padded_and_unpadded() {
        assert_eq!(base64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(base64("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(base64("AAEC/w=="), Some(vec![0, 1, 2, 255]));
        assert_eq!(base64(""), Some(Vec::new()));
        assert_eq!(base64("a*b"), None);
    }

    #[test]
    fn a_flat_mesh_stays_flat_and_centred() {
        let mut mesh = MeshAsset {
            positions: vec![[1.0, 2.0, 5.0], [3.0, 2.0, 5.0], [1.0, 2.0, 9.0]],
            normals: vec![[0.0, 1.0, 0.0]; 3],
            uvs: vec![[0.0; 2]; 3],
            indices: vec![0, 2, 1],
            size: [0.0; 3],
        };
        fit_to_unit_cube(&mut mesh);
        assert_eq!(mesh.size, [2.0, 0.0, 4.0]);
        assert_eq!(mesh.positions, [[-0.5, 0.0, -0.5], [0.5, 0.0, -0.5], [-0.5, 0.0, 0.5]]);
        assert_eq!(
            mesh.normals,
            [[0.0, 1.0, 0.0]; 3],
            "the stretch keeps a flat face's normal"
        );
    }
}
