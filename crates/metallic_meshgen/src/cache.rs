//! Deterministic mesh keys and a cache that shares one mesh between identical Parts.
use crate::error::MeshGenError;
use crate::mesh::MeshData;
use crate::shape::{
    Bevel, BevelStyle, Lod, MAX_SIZE, MIN_SIZE, MeshDesc, Shading, Shape, ShapeParams,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Lengths in a key are stored as multiples of 1/4096 stud (about 0.00024): exact in binary floating point,
/// far below anything visible, and coarse enough that `4.0` and `4.0000001` share a mesh.
const QUANTUM: f32 = 4096.0;
/// Crease angles are stored in 1/100 degree.
const ANGLE_QUANTUM: f32 = 100.0;

/// A quantised, validated [`MeshDesc`]. Equal keys mean bit-identical meshes, so a renderer can key its GPU
/// buffers on it. Parameters that do not affect the shape are zeroed (a `Block` ignores `steps`, a `Ball`
/// ignores the bevel, a convex shape ignores `ao`, a flat-sided shape with no round bevel ignores the LOD).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshKey {
    shape: Shape,
    size: [u32; 3],
    bevel_round: bool,
    bevel: u32,
    lod: Lod,
    shading: u8,
    crease: u32,
    ao: bool,
    steps: u32,
    wall: u32,
    thickness: u32,
    corner_radius: u32,
}

fn quantize(v: f32) -> u32 {
    // Callers check range first; the cast saturates anyway.
    (v * QUANTUM).round() as u32
}

fn dequantize(q: u32) -> f32 {
    q as f32 / QUANTUM
}

fn check_param(name: &'static str, value: f32) -> Result<f32, MeshGenError> {
    if value.is_finite() && value >= 0.0 {
        Ok(value.min(MAX_SIZE))
    } else {
        Err(MeshGenError::InvalidParameter { name, value })
    }
}

impl MeshKey {
    pub(crate) fn from_desc(desc: &MeshDesc) -> Result<MeshKey, MeshGenError> {
        let s = desc.size;
        if s.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err(MeshGenError::InvalidSize(s));
        }
        if s.iter().any(|v| *v > MAX_SIZE) {
            return Err(MeshGenError::SizeTooLarge {
                size: s,
                max: MAX_SIZE,
            });
        }
        let size = s.map(|v| quantize(v.max(MIN_SIZE)).max(1));
        let shape = desc.shape;
        let bevel_width = check_param("bevel.width", desc.bevel.width)?;
        let wall = check_param("params.wall", desc.params.wall)?;
        let thickness = check_param("params.thickness", desc.params.thickness)?;
        let corner_radius = check_param("params.corner_radius", desc.params.corner_radius)?;

        let mut bevel = if shape.takes_bevel() {
            quantize(bevel_width)
        } else {
            0
        };
        let mut bevel_round = desc.bevel.style == BevelStyle::Round;
        if shape == Shape::RoundedBlock {
            bevel = bevel.max(quantize(corner_radius));
            bevel_round = true;
        }
        if bevel == 0 {
            bevel_round = false;
        }

        let (shading, crease) = match desc.shading {
            Shading::Smooth => (0, 0),
            Shading::Flat => (1, 0),
            Shading::Crease { angle_deg } => {
                if !angle_deg.is_finite() || angle_deg < 0.0 {
                    return Err(MeshGenError::InvalidParameter {
                        name: "shading.angle_deg",
                        value: angle_deg,
                    });
                }
                (2, (angle_deg.min(180.0) * ANGLE_QUANTUM).round() as u32)
            }
        };

        // LOD only matters for curved surfaces and round bevels.
        let lod = if shape.is_curved() || (bevel > 0 && bevel_round) {
            desc.lod
        } else {
            Lod::High
        };

        Ok(MeshKey {
            shape,
            size,
            bevel_round,
            bevel,
            lod,
            shading,
            crease,
            ao: desc.ao && !shape.is_convex(),
            steps: if shape == Shape::Stairs {
                desc.params.steps.min(64)
            } else {
                0
            },
            wall: if shape == Shape::Pipe {
                quantize(wall)
            } else {
                0
            },
            thickness: if shape == Shape::Arch {
                quantize(thickness)
            } else {
                0
            },
            corner_radius: if shape == Shape::RoundedBlock {
                quantize(corner_radius)
            } else {
                0
            },
        })
    }

    /// The (quantised) description this key stands for. Generating from it reproduces the key exactly.
    pub fn desc(&self) -> MeshDesc {
        MeshDesc {
            shape: self.shape,
            size: self.size.map(dequantize),
            bevel: Bevel {
                width: dequantize(self.bevel),
                style: if self.bevel_round {
                    BevelStyle::Round
                } else {
                    BevelStyle::Chamfer
                },
            },
            lod: self.lod,
            shading: match self.shading {
                0 => Shading::Smooth,
                1 => Shading::Flat,
                _ => Shading::Crease {
                    angle_deg: self.crease as f32 / ANGLE_QUANTUM,
                },
            },
            ao: self.ao,
            params: ShapeParams {
                steps: self.steps,
                wall: dequantize(self.wall),
                thickness: dequantize(self.thickness),
                corner_radius: dequantize(self.corner_radius),
            },
        }
    }

    /// The shape of this key.
    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// A 64-bit hash that is the same on every run and machine (FNV-1a over a fixed little-endian encoding,
    /// salted with [`crate::MESHGEN_VERSION`]). Use it to name on-disk caches; use the key itself in maps.
    pub fn stable_hash(&self) -> u64 {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h = OFFSET;
        let mut eat = |bytes: &[u8]| {
            for b in bytes {
                h ^= u64::from(*b);
                h = h.wrapping_mul(PRIME);
            }
        };
        eat(&crate::MESHGEN_VERSION.to_le_bytes());
        eat(&[
            self.shape.id(),
            u8::from(self.bevel_round),
            self.lod.id(),
            self.shading,
            u8::from(self.ao),
        ]);
        for v in self.size {
            eat(&v.to_le_bytes());
        }
        for v in [
            self.bevel,
            self.crease,
            self.steps,
            self.wall,
            self.thickness,
            self.corner_radius,
        ] {
            eat(&v.to_le_bytes());
        }
        h
    }
}

/// Keeps one generated mesh per [`MeshKey`] so identical Parts share a single buffer.
///
/// Not thread-safe on its own; wrap it in a `Mutex` to share it, or keep one per thread.
#[derive(Debug, Default)]
pub struct MeshCache {
    meshes: HashMap<MeshKey, Arc<MeshData>>,
}

impl MeshCache {
    /// An empty cache.
    pub fn new() -> MeshCache {
        MeshCache::default()
    }

    /// The mesh for `desc`, generated on first use.
    pub fn get_or_generate(&mut self, desc: &MeshDesc) -> Result<Arc<MeshData>, MeshGenError> {
        let key = desc.key()?;
        Ok(self.get_or_generate_key(&key))
    }

    /// The mesh for an already validated key, generated on first use.
    pub fn get_or_generate_key(&mut self, key: &MeshKey) -> Arc<MeshData> {
        self.meshes
            .entry(*key)
            .or_insert_with(|| Arc::new(crate::generate_key(key)))
            .clone()
    }

    /// The cached mesh for `key`, if any.
    pub fn get(&self, key: &MeshKey) -> Option<Arc<MeshData>> {
        self.meshes.get(key).cloned()
    }

    /// Number of distinct meshes held.
    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }

    /// Drops meshes no Part holds any more (their `Arc` count is 1), returning how many were dropped.
    pub fn evict_unused(&mut self) -> usize {
        let before = self.meshes.len();
        self.meshes.retain(|_, m| Arc::strong_count(m) > 1);
        before - self.meshes.len()
    }

    /// Drops everything.
    pub fn clear(&mut self) {
        self.meshes.clear();
    }
}
