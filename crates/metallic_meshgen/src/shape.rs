//! Shapes and the request ([`MeshDesc`]) that describes one mesh.
use crate::cache::MeshKey;
use crate::error::MeshGenError;
use std::str::FromStr;

/// Smallest size per axis, in studs. Smaller positive sizes are raised to this (as Roblox does).
pub const MIN_SIZE: f32 = 0.001;
/// Largest size per axis, in studs.
pub const MAX_SIZE: f32 = 4096.0;

/// Every shape the generator can build. See the crate docs for each shape's orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Shape {
    /// A box.
    Block,
    /// A right triangular prism; the slope descends from the top of the +Z side to the bottom of the -Z side.
    Wedge,
    /// A square-based pyramid whose peak sits above the (+X, +Z) corner; vertical faces on +X and +Z.
    CornerWedge,
    /// An ellipsoid filling the box (`Ball` in `metallic_core`; a sphere when the size is uniform).
    Ball,
    /// A cylinder along X.
    Cylinder,
    /// A cone with its base at -Y and its tip at +Y.
    Cone,
    /// A pyramid with a rectangular base at -Y and its tip at +Y.
    Pyramid,
    /// A roof-shaped prism: triangle cross-section in the YZ plane, ridge at the top along X.
    TriangularPrism,
    /// A ring around the X axis (a wheel when upright).
    Torus,
    /// A hollow cylinder (tube) along X; wall thickness in [`ShapeParams::wall`].
    Pipe,
    /// A quarter pipe: a concave curved ramp rising toward +Z with a short flat deck on top.
    Ramp,
    /// A staircase rising toward +Z; step count in [`ShapeParams::steps`].
    Stairs,
    /// An arch spanning X with a round opening; walk through along Z. Thickness in [`ShapeParams::thickness`].
    Arch,
    /// A box with fully rounded edges and corners; radius in [`ShapeParams::corner_radius`].
    RoundedBlock,
    /// A cylinder along X with rounded ends.
    Capsule,
    /// Half an ellipsoid: flat base at -Y, dome up.
    Hemisphere,
    /// A hexagonal prism along X, with a flat face at the top and bottom.
    HexPrism,
}

impl Shape {
    /// Every shape, in a stable order.
    pub const ALL: [Shape; 17] = [
        Shape::Block,
        Shape::Wedge,
        Shape::CornerWedge,
        Shape::Ball,
        Shape::Cylinder,
        Shape::Cone,
        Shape::Pyramid,
        Shape::TriangularPrism,
        Shape::Torus,
        Shape::Pipe,
        Shape::Ramp,
        Shape::Stairs,
        Shape::Arch,
        Shape::RoundedBlock,
        Shape::Capsule,
        Shape::Hemisphere,
        Shape::HexPrism,
    ];

    /// The name used in scene files and the Luau API (`Ball`, not `Sphere`, as in `metallic_core`).
    pub fn name(self) -> &'static str {
        match self {
            Shape::Block => "Block",
            Shape::Wedge => "Wedge",
            Shape::CornerWedge => "CornerWedge",
            Shape::Ball => "Ball",
            Shape::Cylinder => "Cylinder",
            Shape::Cone => "Cone",
            Shape::Pyramid => "Pyramid",
            Shape::TriangularPrism => "TriangularPrism",
            Shape::Torus => "Torus",
            Shape::Pipe => "Pipe",
            Shape::Ramp => "Ramp",
            Shape::Stairs => "Stairs",
            Shape::Arch => "Arch",
            Shape::RoundedBlock => "RoundedBlock",
            Shape::Capsule => "Capsule",
            Shape::Hemisphere => "Hemisphere",
            Shape::HexPrism => "HexPrism",
        }
    }

    /// Looks a shape up by name. Accepts [`Shape::name`] plus the aliases `Sphere`, `Prism`, `Tube`,
    /// `QuarterPipe` and `Hexagon`. Matching is exact (case-sensitive), like property values in scene files.
    pub fn from_name(name: &str) -> Option<Shape> {
        let alias = match name {
            "Sphere" => Some(Shape::Ball),
            "Prism" => Some(Shape::TriangularPrism),
            "Tube" => Some(Shape::Pipe),
            "QuarterPipe" => Some(Shape::Ramp),
            "Hexagon" => Some(Shape::HexPrism),
            _ => None,
        };
        alias.or_else(|| Shape::ALL.into_iter().find(|s| s.name() == name))
    }

    /// A good size for a freshly inserted part of this shape, in studs.
    pub fn default_size(self) -> [f32; 3] {
        match self {
            Shape::Block | Shape::Wedge | Shape::RoundedBlock => [4.0, 1.0, 2.0],
            Shape::CornerWedge => [2.0, 2.0, 2.0],
            Shape::Ball | Shape::Cylinder | Shape::Cone | Shape::Hemisphere => [2.0, 2.0, 2.0],
            Shape::Pyramid => [2.0, 2.0, 2.0],
            Shape::TriangularPrism => [4.0, 2.0, 4.0],
            Shape::Torus => [1.0, 4.0, 4.0],
            Shape::Pipe => [4.0, 2.0, 2.0],
            Shape::Ramp | Shape::Stairs => [4.0, 4.0, 4.0],
            Shape::Arch => [6.0, 5.0, 2.0],
            Shape::Capsule => [4.0, 2.0, 2.0],
            Shape::HexPrism => [2.0, 2.0, 2.0],
        }
    }

    /// True when the solid is convex (ambient occlusion is then always 1 and a convex collider is exact).
    pub fn is_convex(self) -> bool {
        !matches!(
            self,
            Shape::Torus | Shape::Pipe | Shape::Ramp | Shape::Stairs | Shape::Arch
        )
    }

    /// True when the shape has curved surfaces, so [`Lod`] changes its triangle count even without a bevel.
    pub fn is_curved(self) -> bool {
        matches!(
            self,
            Shape::Ball
                | Shape::Cylinder
                | Shape::Cone
                | Shape::Torus
                | Shape::Pipe
                | Shape::Ramp
                | Shape::Arch
                | Shape::RoundedBlock
                | Shape::Capsule
                | Shape::Hemisphere
        )
    }

    /// True when the shape has edges that [`Bevel`] can soften (spheres, capsules and tori have none).
    pub fn takes_bevel(self) -> bool {
        !matches!(self, Shape::Ball | Shape::Capsule | Shape::Torus)
    }

    /// Stable small number used in [`MeshKey`]. Never reuse a number for a different shape.
    pub(crate) fn id(self) -> u8 {
        match self {
            Shape::Block => 0,
            Shape::Wedge => 1,
            Shape::CornerWedge => 2,
            Shape::Ball => 3,
            Shape::Cylinder => 4,
            Shape::Cone => 5,
            Shape::Pyramid => 6,
            Shape::TriangularPrism => 7,
            Shape::Torus => 8,
            Shape::Pipe => 9,
            Shape::Ramp => 10,
            Shape::Stairs => 11,
            Shape::Arch => 12,
            Shape::RoundedBlock => 13,
            Shape::Capsule => 14,
            Shape::Hemisphere => 15,
            Shape::HexPrism => 16,
        }
    }
}

impl FromStr for Shape {
    type Err = MeshGenError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Shape::from_name(s).ok_or_else(|| MeshGenError::UnknownShape(s.to_string()))
    }
}

/// How an edge is softened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BevelStyle {
    /// A rounded fillet; its segment count follows the [`Lod`].
    Round,
    /// A single flat cut at 45° between the faces.
    Chamfer,
}

/// Edge softening, measured in studs so it looks the same on a tiny brick and on a huge wall.
///
/// The width is clamped per mesh so it never eats a whole face (for example a 0.3 stud bevel on a 0.2 stud
/// plate). Sharp angles recede a little: a rounded knife edge sits slightly inside the bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bevel {
    /// Radius (round) or setback (chamfer) in studs. 0 means sharp edges.
    pub width: f32,
    /// Round or flat.
    pub style: BevelStyle,
}

impl Bevel {
    /// Sharp edges.
    pub const NONE: Bevel = Bevel {
        width: 0.0,
        style: BevelStyle::Round,
    };
    /// The default: a 0.05 stud round that catches the light on every edge without looking soft.
    pub const SUBTLE: Bevel = Bevel {
        width: 0.05,
        style: BevelStyle::Round,
    };

    /// A rounded bevel of `width` studs.
    pub fn round(width: f32) -> Bevel {
        Bevel {
            width,
            style: BevelStyle::Round,
        }
    }

    /// A flat 45° chamfer of `width` studs.
    pub fn chamfer(width: f32) -> Bevel {
        Bevel {
            width,
            style: BevelStyle::Chamfer,
        }
    }
}

impl Default for Bevel {
    fn default() -> Self {
        Bevel::SUBTLE
    }
}

/// Level of detail for curved surfaces. Segment counts also grow with the real size (a fixed chord error in
/// studs), so a 50-stud ball stays smooth while a 0.5-stud one stays cheap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lod {
    /// Far away: 8-32 segments per circle, chamfer-like bevels.
    Low,
    /// Middle distance: 16-64 segments per circle.
    Medium,
    /// Close up (default): 24-128 segments per circle.
    #[default]
    High,
}

impl Lod {
    /// All levels, coarse to fine.
    pub const ALL: [Lod; 3] = [Lod::Low, Lod::Medium, Lod::High];

    pub(crate) fn id(self) -> u8 {
        match self {
            Lod::Low => 0,
            Lod::Medium => 1,
            Lod::High => 2,
        }
    }
}

/// How vertex normals are made.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Shading {
    /// Exact normals from the surface: curved parts are smooth, edges between faces are hard (default).
    #[default]
    Smooth,
    /// Every triangle flat (a faceted, low-poly look).
    Flat,
    /// Normals recomputed from the triangles: neighbours closer than `angle_deg` are averaged, sharper edges
    /// stay hard.
    Crease {
        /// Crease angle in degrees (0..=180).
        angle_deg: f32,
    },
}

/// Extra numbers some shapes need. Lengths are in studs. Fields a shape does not use are ignored (and do not
/// split the mesh cache).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeParams {
    /// `Stairs`: number of steps. 0 picks one step per stud of height (at least 2, at most 64).
    pub steps: u32,
    /// `Pipe`: wall thickness. Clamped so the hole stays open.
    pub wall: f32,
    /// `Arch`: thickness of the legs and of the crown. Clamped to the size.
    pub thickness: f32,
    /// `RoundedBlock`: edge radius (the bevel is the larger of this and [`MeshDesc::bevel`]).
    pub corner_radius: f32,
}

impl Default for ShapeParams {
    fn default() -> Self {
        ShapeParams {
            steps: 0,
            wall: 0.4,
            thickness: 1.0,
            corner_radius: 0.4,
        }
    }
}

/// Everything that decides one mesh. Build with [`MeshDesc::new`] and the `with_*` methods.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshDesc {
    /// The shape.
    pub shape: Shape,
    /// Full size in studs (the Part's `Size`).
    pub size: [f32; 3],
    /// Edge softening (default [`Bevel::SUBTLE`]).
    pub bevel: Bevel,
    /// Detail of curved surfaces (default [`Lod::High`]).
    pub lod: Lod,
    /// Normal style (default [`Shading::Smooth`]).
    pub shading: Shading,
    /// Bake ambient occlusion into [`crate::MeshData::ao`] (default true; convex shapes get all 1.0 for free).
    pub ao: bool,
    /// Shape-specific numbers.
    pub params: ShapeParams,
}

impl MeshDesc {
    /// A description with default bevel, LOD, shading and parameters.
    pub fn new(shape: Shape, size: [f32; 3]) -> MeshDesc {
        MeshDesc {
            shape,
            size,
            bevel: Bevel::default(),
            lod: Lod::default(),
            shading: Shading::default(),
            ao: true,
            params: ShapeParams::default(),
        }
    }

    /// Sets the bevel.
    pub fn with_bevel(mut self, bevel: Bevel) -> MeshDesc {
        self.bevel = bevel;
        self
    }

    /// Sets the level of detail.
    pub fn with_lod(mut self, lod: Lod) -> MeshDesc {
        self.lod = lod;
        self
    }

    /// Sets the shading.
    pub fn with_shading(mut self, shading: Shading) -> MeshDesc {
        self.shading = shading;
        self
    }

    /// Turns ambient occlusion baking on or off.
    pub fn with_ao(mut self, ao: bool) -> MeshDesc {
        self.ao = ao;
        self
    }

    /// Sets the shape parameters.
    pub fn with_params(mut self, params: ShapeParams) -> MeshDesc {
        self.params = params;
        self
    }

    /// Validates and quantises this description into a cache key (see [`MeshKey`]).
    pub fn key(&self) -> Result<MeshKey, MeshGenError> {
        MeshKey::from_desc(self)
    }

    /// Half the size, as a vector.
    pub(crate) fn half(&self) -> glam::Vec3 {
        glam::Vec3::from(self.size) * 0.5
    }
}
