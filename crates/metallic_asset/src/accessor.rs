//! Bounds-checked reading of glTF accessors from the `.glb` BIN chunk.
//!
//! Every offset and length is computed with checked arithmetic and tested against the buffer view
//! and the chunk before any byte is read, and element counts are tested against a limit before
//! anything is allocated, so a hostile file cannot make this panic or allocate without bound.
use crate::AssetError;
use gltf::accessor::{Accessor, DataType};

/// The bytes of one accessor and how to step through them.
struct Raw<'a> {
    data: &'a [u8],
    stride: usize,
    count: usize,
    kind: DataType,
    dims: usize,
    normalized: bool,
}

fn component_size(kind: DataType) -> usize {
    match kind {
        DataType::I8 | DataType::U8 => 1,
        DataType::I16 | DataType::U16 => 2,
        DataType::U32 | DataType::F32 => 4,
    }
}

fn overflow(what: &str) -> AssetError {
    AssetError::Invalid(format!("{what}: accessor range overflows"))
}

fn raw<'a>(
    acc: &Accessor<'_>,
    bin: &'a [u8],
    dims: usize,
    limit: usize,
    what: &str,
) -> Result<Raw<'a>, AssetError> {
    if acc.sparse().is_some() {
        return Err(AssetError::Unsupported(format!("{what}: sparse accessor")));
    }
    if acc.dimensions().multiplicity() != dims {
        return Err(AssetError::Invalid(format!(
            "{what}: expected {dims} components per element, found {}",
            acc.dimensions().multiplicity()
        )));
    }
    let count = acc.count();
    if count > limit {
        return Err(AssetError::TooLarge {
            what: what.to_owned(),
            count,
            limit,
        });
    }
    let view = acc
        .view()
        .ok_or_else(|| AssetError::Unsupported(format!("{what}: accessor without buffer view")))?;
    let kind = acc.data_type();
    let element = component_size(kind) * dims;
    let stride = view.stride().unwrap_or(element);
    if stride < element {
        return Err(AssetError::Invalid(format!(
            "{what}: stride {stride} is shorter than an element ({element} bytes)"
        )));
    }
    let view_end = view
        .offset()
        .checked_add(view.length())
        .ok_or_else(|| overflow(what))?;
    if view_end > bin.len() {
        return Err(AssetError::Invalid(format!(
            "{what}: buffer view ends at byte {view_end}, past the {} byte BIN chunk",
            bin.len()
        )));
    }
    let start = view
        .offset()
        .checked_add(acc.offset())
        .ok_or_else(|| overflow(what))?;
    let end = match count {
        0 => start,
        n => stride
            .checked_mul(n - 1)
            .and_then(|s| s.checked_add(element))
            .and_then(|s| s.checked_add(start))
            .ok_or_else(|| overflow(what))?,
    };
    if end > view_end || start > end {
        return Err(AssetError::Invalid(format!(
            "{what}: {count} elements do not fit in their buffer view"
        )));
    }
    Ok(Raw {
        data: bin.get(start..end).ok_or_else(|| overflow(what))?,
        stride,
        count,
        kind,
        dims,
        normalized: acc.normalized(),
    })
}

impl Raw<'_> {
    /// The bytes of component `c` of element `i` (in range by construction of [`raw`]).
    fn bytes<const N: usize>(&self, i: usize, c: usize) -> [u8; N] {
        let at = i * self.stride + c * N;
        let mut out = [0u8; N];
        if let Some(src) = self.data.get(at..at + N) {
            out.copy_from_slice(src);
        }
        out
    }

    fn float(&self, i: usize, c: usize) -> f32 {
        match self.kind {
            DataType::F32 => f32::from_le_bytes(self.bytes(i, c)),
            DataType::I8 => (f32::from(i8::from_le_bytes(self.bytes(i, c))) / 127.0).max(-1.0),
            DataType::U8 => f32::from(u8::from_le_bytes(self.bytes(i, c))) / 255.0,
            DataType::I16 => (f32::from(i16::from_le_bytes(self.bytes(i, c))) / 32767.0).max(-1.0),
            DataType::U16 => f32::from(u16::from_le_bytes(self.bytes(i, c))) / 65535.0,
            DataType::U32 => 0.0,
        }
    }

    fn uint(&self, i: usize, c: usize) -> u32 {
        match self.kind {
            DataType::U8 => u32::from(u8::from_le_bytes(self.bytes(i, c))),
            DataType::U16 => u32::from(u16::from_le_bytes(self.bytes(i, c))),
            DataType::U32 => u32::from_le_bytes(self.bytes(i, c)),
            _ => 0,
        }
    }
}

/// Reads a float accessor of `dims` components into a flat list (`count * dims` values).
/// `FLOAT` and normalized `BYTE`/`UNSIGNED_BYTE`/`SHORT`/`UNSIGNED_SHORT` are accepted (as the glTF
/// spec allows for weights, texture coordinates and rotations). Every value must be finite.
pub(crate) fn read_floats(
    acc: &Accessor<'_>,
    bin: &[u8],
    dims: usize,
    limit: usize,
    what: &str,
) -> Result<Vec<f32>, AssetError> {
    let r = raw(acc, bin, dims, limit, what)?;
    let ok = match r.kind {
        DataType::F32 => true,
        DataType::U32 => false,
        _ => r.normalized,
    };
    if !ok {
        return Err(AssetError::Unsupported(format!(
            "{what}: component type {:?} (normalized: {})",
            r.kind, r.normalized
        )));
    }
    let mut out = Vec::with_capacity(r.count * r.dims);
    for i in 0..r.count {
        for c in 0..r.dims {
            let v = r.float(i, c);
            if !v.is_finite() {
                return Err(AssetError::Invalid(format!(
                    "{what}: value {i} is not finite"
                )));
            }
            out.push(v);
        }
    }
    Ok(out)
}

/// Reads an unsigned integer accessor (`UNSIGNED_BYTE`, `UNSIGNED_SHORT` or `UNSIGNED_INT`, not
/// normalized) of `dims` components into a flat list.
pub(crate) fn read_uints(
    acc: &Accessor<'_>,
    bin: &[u8],
    dims: usize,
    limit: usize,
    what: &str,
) -> Result<Vec<u32>, AssetError> {
    let r = raw(acc, bin, dims, limit, what)?;
    if !matches!(r.kind, DataType::U8 | DataType::U16 | DataType::U32) || r.normalized {
        return Err(AssetError::Unsupported(format!(
            "{what}: component type {:?} (normalized: {})",
            r.kind, r.normalized
        )));
    }
    let mut out = Vec::with_capacity(r.count * r.dims);
    for i in 0..r.count {
        for c in 0..r.dims {
            out.push(r.uint(i, c));
        }
    }
    Ok(out)
}
