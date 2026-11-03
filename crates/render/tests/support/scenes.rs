//! Games to draw: the demo scene of the image tests and grids of many parts for the draw-count tests and benches,
//! built through the data model as a game would build them. Also PNG helpers for reference images.

use std::path::Path;

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, ClassId, DataModel, EnumItem, Id, Value};

/// A game with a `Workspace` and its camera.
pub(crate) struct Game {
    pub(crate) dm: DataModel,
    pub(crate) workspace: Id,
    pub(crate) camera: Id,
    part: ClassId,
}

/// How a part looks: its `Shape`, `Material`, sRGB colour and `Transparency`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Look {
    pub(crate) shape: &'static str,
    pub(crate) material: &'static str,
    pub(crate) color: [u8; 3],
    pub(crate) transparency: f64,
}

impl Look {
    pub(crate) const fn new(shape: &'static str, material: &'static str, color: [u8; 3]) -> Self {
        Self {
            shape,
            material,
            color,
            transparency: 0.0,
        }
    }

    pub(crate) const fn transparent(self, transparency: f64) -> Self {
        Self { transparency, ..self }
    }
}

impl Game {
    pub(crate) fn new() -> Self {
        let dm = metallic_classes::new_game().expect("a game");
        let registry = dm.registry().clone();
        let workspace = dm
            .service(registry.class_id("Workspace").expect("Workspace"))
            .expect("a Workspace");
        let Ok(Value::Ref(Some(camera))) = dm.get_by_name(workspace, "CurrentCamera") else {
            panic!("a new game has a CurrentCamera");
        };
        Self {
            dm,
            workspace,
            camera,
            part: registry.class_id("Part").expect("Part"),
        }
    }

    /// Adds a part to `Workspace`.
    pub(crate) fn part(&mut self, look: Look, size: Vec3, cframe: CFrame) -> Id {
        let dm = &mut self.dm;
        let id = dm.create(self.part).expect("a part");
        dm.set_by_name(id, "Shape", Value::Enum(EnumItem::new("PartShape", look.shape)))
            .expect("Shape");
        dm.set_by_name(id, "Material", Value::Enum(EnumItem::new("Material", look.material)))
            .expect("Material");
        dm.set_by_name(id, "Size", Value::Vector3(size)).expect("Size");
        dm.set_by_name(id, "CFrame", Value::CFrame(cframe)).expect("CFrame");
        let [r, g, b] = look.color;
        dm.set_by_name(id, "Color", Value::Color3(metallic_core::Color3::from_rgb_u8(r, g, b)))
            .expect("Color");
        if look.transparency != 0.0 {
            dm.set_by_name(id, "Transparency", Value::Number(look.transparency))
                .expect("Transparency");
        }
        dm.set_parent(id, Some(self.workspace)).expect("parented");
        id
    }

    /// Places the current camera.
    pub(crate) fn set_camera(&mut self, cframe: CFrame, field_of_view: f64) {
        self.dm
            .set_by_name(self.camera, "CFrame", Value::CFrame(cframe))
            .expect("camera CFrame");
        self.dm
            .set_by_name(self.camera, "FieldOfView", Value::Number(field_of_view))
            .expect("FieldOfView");
    }
}

/// The demo scene, shifted by `offset`: every shape on a concrete floor, plastic, smooth plastic, metal, wood,
/// grass, neon, and two see-through parts (a glass pane and a ball) in front, with the camera looking at them.
pub(crate) fn demo(game: &mut Game, offset: Vec3) {
    let at = |x: f32, y: f32, z: f32| offset + Vec3::new(x, y, z);
    let turned =
        |p: Vec3, degrees: f32| CFrame::from_parts(p, metallic_core::glam::Quat::from_rotation_y(degrees.to_radians()));
    game.part(
        Look::new("Block", "Concrete", [128, 126, 120]),
        Vec3::new(26.0, 1.0, 26.0),
        CFrame::new(at(0.0, -0.5, 0.0)),
    );
    game.part(
        Look::new("Block", "Plastic", [196, 40, 28]),
        Vec3::new(3.0, 3.0, 3.0),
        turned(at(-5.5, 1.5, -1.0), 25.0),
    );
    game.part(
        Look::new("Ball", "SmoothPlastic", [13, 105, 172]),
        Vec3::splat(3.5),
        CFrame::new(at(-1.5, 1.75, -3.0)),
    );
    game.part(
        Look::new("Cylinder", "Wood", [160, 95, 53]),
        Vec3::new(5.0, 2.0, 2.0),
        turned(at(2.5, 1.0, -1.0), -30.0),
    );
    game.part(
        Look::new("Wedge", "Grass", [75, 151, 75]),
        Vec3::new(3.0, 2.5, 4.0),
        turned(at(6.5, 1.25, -2.0), 90.0),
    );
    game.part(
        Look::new("Ball", "Metal", [226, 155, 64]),
        Vec3::splat(2.0),
        CFrame::new(at(0.5, 1.0, 2.0)),
    );
    game.part(
        Look::new("Block", "Neon", [0, 255, 191]),
        Vec3::new(1.0, 1.0, 1.0),
        turned(at(-3.0, 0.5, 3.0), 45.0),
    );
    game.part(
        Look::new("Block", "Glass", [175, 221, 255]).transparent(0.6),
        Vec3::new(5.0, 3.0, 0.3),
        CFrame::new(at(-4.0, 1.5, 5.0)),
    );
    game.part(
        Look::new("Ball", "Plastic", [255, 89, 89]).transparent(0.35),
        Vec3::splat(2.0),
        CFrame::new(at(4.0, 3.5, 3.0)),
    );
    // Fully transparent: never drawn.
    game.part(
        Look::new("Block", "Plastic", [0, 0, 0]).transparent(1.0),
        Vec3::splat(4.0),
        CFrame::new(at(0.0, 2.0, 8.0)),
    );
    game.set_camera(CFrame::look_at(at(0.0, 6.0, 13.0), at(0.0, 1.0, 0.0), Vec3::Y), 60.0);
}

/// The shapes and materials grids cycle through.
const GRID_LOOKS: [Look; 4] = [
    Look::new("Block", "Plastic", [163, 162, 165]),
    Look::new("Ball", "SmoothPlastic", [13, 105, 172]),
    Look::new("Cylinder", "Metal", [226, 155, 64]),
    Look::new("Wedge", "Wood", [160, 95, 53]),
];

/// Which see-through parts a [`grid`] has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeeThrough {
    /// None.
    None,
    /// Every `n`th part is a see-through block.
    Blocks(usize),
    /// Every `n`th part is see-through, blocks and balls in turn (so sorted, they need several draws).
    Mixed(usize),
}

/// `count` parts on a square grid 4 m apart, centred on the origin: one part in `rare` of each round shape and
/// wedge (the rest blocks, so the GPU work stays small), and see-through parts as `see_through` says. The camera
/// looks at the grid from above. Returns the parts.
pub(crate) fn grid(game: &mut Game, count: usize, rare: usize, see_through: SeeThrough) -> Vec<Id> {
    let side = (count as f32).sqrt().ceil() as usize;
    game.dm.reserve(count);
    let mut parts = Vec::with_capacity(count);
    for i in 0..count {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        let place = Vec3::new((x - side as f32 / 2.0) * 4.0, 1.0, (z - side as f32 / 2.0) * 4.0);
        let look = match see_through {
            SeeThrough::Blocks(n) if i % n == n - 1 => GRID_LOOKS[0].transparent(0.5),
            SeeThrough::Mixed(n) if i % n == n - 1 => GRID_LOOKS[(i / n) % 2].transparent(0.5),
            _ if i % rare < 3 => GRID_LOOKS[1 + i % rare],
            _ => GRID_LOOKS[0],
        };
        parts.push(game.part(look, Vec3::new(2.0, 2.0, 2.0), CFrame::new(place)));
    }
    let extent = side as f32 * 2.0;
    game.set_camera(
        CFrame::look_at(Vec3::new(0.0, extent * 1.2, extent * 1.2), Vec3::ZERO, Vec3::Y),
        70.0,
    );
    parts
}

/// Writes 8-bit RGBA pixels as a PNG.
pub(crate) fn write_png(path: &Path, width: u32, height: u32, pixels: &[[u8; 4]]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the PNG's folder");
    }
    let file = std::fs::File::create(path).expect("create the PNG");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header().expect("PNG header");
    writer.write_image_data(pixels.as_flattened()).expect("PNG data");
}

/// Reads an 8-bit RGBA PNG: its size and pixels. `None` when the file does not exist.
pub(crate) fn read_png(path: &Path) -> Option<(u32, u32, Vec<[u8; 4]>)> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().expect("PNG header");
    let mut data = vec![0; reader.output_buffer_size().expect("PNG size")];
    let info = reader.next_frame(&mut data).expect("PNG data");
    assert_eq!(
        (info.color_type, info.bit_depth),
        (png::ColorType::Rgba, png::BitDepth::Eight),
        "{} is not 8-bit RGBA",
        path.display()
    );
    let pixels = data[..info.buffer_size()].as_chunks::<4>().0.to_vec();
    Some((info.width, info.height, pixels))
}

/// How two images differ, per pixel by the largest channel difference.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImageDiff {
    /// The largest difference.
    pub(crate) max: u8,
    /// The mean difference over all pixels.
    pub(crate) mean: f64,
    /// The share of pixels that differ by more than the threshold [`compare`] was given.
    pub(crate) over: f64,
}

/// Compares two images of the same size.
pub(crate) fn compare(a: &[[u8; 4]], b: &[[u8; 4]], threshold: u8) -> ImageDiff {
    assert_eq!(a.len(), b.len(), "the images differ in size");
    let mut max = 0u8;
    let mut sum = 0u64;
    let mut over = 0usize;
    for (pa, pb) in a.iter().zip(b) {
        let d = pa.iter().zip(pb).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        max = max.max(d);
        sum += u64::from(d);
        if d > threshold {
            over += 1;
        }
    }
    let n = a.len().max(1) as f64;
    ImageDiff {
        max,
        mean: sum as f64 / n,
        over: over as f64 / n,
    }
}
