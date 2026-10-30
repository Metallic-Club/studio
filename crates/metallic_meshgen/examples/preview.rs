//! Renders every shape into one PNG contact sheet with a tiny CPU rasterizer (no GPU, no window).
//!
//! ```text
//! cargo run -p metallic_meshgen --example preview -- [OUT.png] [--lod low|medium|high]
//!     [--bevel WIDTH] [--chamfer] [--flat] [--no-ao] [--tile PIXELS]
//! ```
//!
//! Shapes are drawn in `Shape::ALL` order, left to right, top to bottom, at their default size. Lighting is
//! Lambert plus the baked `ao`. A faint 1-stud checker from `uv0` shows that UV density stays the same on
//! every face. The order is printed to stdout because the sheet has no text.
use glam::{Mat3, Vec3};
use metallic_meshgen::{Bevel, Lod, MeshData, MeshDesc, Shading, Shape, generate};
use std::error::Error;
use std::fs::File;
use std::io::BufWriter;

struct Options {
    out: String,
    lod: Lod,
    bevel: Option<Bevel>,
    chamfer: bool,
    flat: bool,
    ao: bool,
    tile: usize,
}

fn parse_args() -> Result<Options, Box<dyn Error>> {
    let mut o = Options {
        out: "meshgen-preview.png".into(),
        lod: Lod::High,
        bevel: None,
        chamfer: false,
        flat: false,
        ao: true,
        tile: 256,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--lod" => {
                o.lod = match value()?.to_ascii_lowercase().as_str() {
                    "low" => Lod::Low,
                    "medium" => Lod::Medium,
                    "high" => Lod::High,
                    other => return Err(format!("unknown LOD {other:?}").into()),
                }
            }
            "--bevel" => o.bevel = Some(Bevel::round(value()?.parse()?)),
            "--chamfer" => o.chamfer = true,
            "--flat" => o.flat = true,
            "--no-ao" => o.ao = false,
            "--tile" => o.tile = value()?.parse::<usize>()?.clamp(64, 1024),
            "-h" | "--help" => {
                println!(
                    "usage: preview [OUT.png] [--lod low|medium|high] [--bevel WIDTH] [--chamfer] [--flat] [--no-ao] [--tile PIXELS]"
                );
                std::process::exit(0);
            }
            s if s.starts_with('-') => return Err(format!("unknown option {s}").into()),
            s => o.out = s.into(),
        }
    }
    Ok(o)
}

/// Linear RGB render target with a depth buffer.
struct Canvas {
    w: usize,
    h: usize,
    color: Vec<Vec3>,
    depth: Vec<f32>,
}

impl Canvas {
    fn new(w: usize, h: usize, background: Vec3) -> Canvas {
        Canvas {
            w,
            h,
            color: vec![background; w * h],
            depth: vec![f32::NEG_INFINITY; w * h],
        }
    }

    fn to_srgb8(&self) -> Vec<u8> {
        self.color
            .iter()
            .flat_map(|c| {
                c.to_array()
                    .map(|x| (x.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8)
            })
            .collect()
    }
}

/// Draws `mesh` into the square tile whose top-left pixel is (`x0`, `y0`).
fn draw(canvas: &mut Canvas, mesh: &MeshData, view: Mat3, x0: usize, y0: usize, tile: usize) {
    let light = Vec3::new(-0.45, 0.8, 0.55).normalize();
    let base = Vec3::new(0.62, 0.64, 0.68);
    let radius = Vec3::from(mesh.bounds.size()).length() * 0.5;
    let scale = tile as f32 * 0.45 / radius.max(1e-6);
    let (cx, cy) = (x0 as f32 + tile as f32 * 0.5, y0 as f32 + tile as f32 * 0.5);

    // View space: x right, y up, +z toward the camera (orthographic).
    let pos: Vec<Vec3> = mesh
        .positions
        .iter()
        .map(|p| view * Vec3::from(*p))
        .collect();
    let nrm: Vec<Vec3> = mesh.normals.iter().map(|n| view * Vec3::from(*n)).collect();
    let screen: Vec<[f32; 2]> = pos
        .iter()
        .map(|p| [cx + p.x * scale, cy - p.y * scale])
        .collect();

    for t in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| i as usize);
        let (pa, pb, pc) = (screen[a], screen[b], screen[c]);
        let edge = |p: [f32; 2], q: [f32; 2], x: f32, y: f32| {
            (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0])
        };
        let area = edge(pa, pb, pc[0], pc[1]);
        // Screen y points down, so counter-clockwise front faces have negative area here.
        if area >= 0.0 {
            continue;
        }
        let min_x = pa[0].min(pb[0]).min(pc[0]).floor().max(x0 as f32) as usize;
        let max_x = pa[0].max(pb[0]).max(pc[0]).ceil().min((x0 + tile) as f32) as usize;
        let min_y = pa[1].min(pb[1]).min(pc[1]).floor().max(y0 as f32) as usize;
        let max_y = pa[1].max(pb[1]).max(pc[1]).ceil().min((y0 + tile) as f32) as usize;
        for y in min_y..max_y.min(canvas.h) {
            for x in min_x..max_x.min(canvas.w) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let la = edge(pb, pc, px, py) / area;
                let lb = edge(pc, pa, px, py) / area;
                let lc = edge(pa, pb, px, py) / area;
                if la < 0.0 || lb < 0.0 || lc < 0.0 {
                    continue;
                }
                let z = la * pos[a].z + lb * pos[b].z + lc * pos[c].z;
                let i = y * canvas.w + x;
                if z <= canvas.depth[i] {
                    continue;
                }
                canvas.depth[i] = z;
                let n = (la * nrm[a] + lb * nrm[b] + lc * nrm[c]).normalize_or_zero();
                let ao = la * mesh.ao[a] + lb * mesh.ao[b] + lc * mesh.ao[c];
                let u = la * mesh.uv0[a][0] + lb * mesh.uv0[b][0] + lc * mesh.uv0[c][0];
                let v = la * mesh.uv0[a][1] + lb * mesh.uv0[b][1] + lc * mesh.uv0[c][1];
                let checker = if (u.floor() + v.floor()).rem_euclid(2.0) < 1.0 {
                    1.0
                } else {
                    0.88
                };
                let lambert = n.dot(light).max(0.0);
                canvas.color[i] = base * checker * ao * (0.22 + 0.78 * lambert);
            }
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let o = parse_args()?;
    let cols = 6;
    let rows = Shape::ALL.len().div_ceil(cols);
    let gap = 4;
    let (w, h) = (
        cols * o.tile + (cols + 1) * gap,
        rows * o.tile + (rows + 1) * gap,
    );
    let mut canvas = Canvas::new(w, h, Vec3::new(0.035, 0.04, 0.05));
    // Seen from the front-left and above: wedge slopes (facing -Z) and the -X ends of X-axis shapes are visible.
    let view =
        Mat3::from_rotation_x(30f32.to_radians()) * Mat3::from_rotation_y(150f32.to_radians());

    for (i, shape) in Shape::ALL.into_iter().enumerate() {
        let mut desc = MeshDesc::new(shape, shape.default_size())
            .with_lod(o.lod)
            .with_ao(o.ao);
        if let Some(mut bevel) = o.bevel {
            if o.chamfer {
                bevel = Bevel::chamfer(bevel.width);
            }
            desc = desc.with_bevel(bevel);
        } else if o.chamfer {
            desc = desc.with_bevel(Bevel::chamfer(Bevel::SUBTLE.width));
        }
        if o.flat {
            desc = desc.with_shading(Shading::Flat);
        }
        let mesh = generate(&desc)?;
        let (col, row) = (i % cols, i / cols);
        draw(
            &mut canvas,
            &mesh,
            view,
            gap + col * (o.tile + gap),
            gap + row * (o.tile + gap),
            o.tile,
        );
        println!(
            "row {} col {}: {:<16} {:>6} verts {:>6} tris",
            row + 1,
            col + 1,
            shape.name(),
            mesh.vertex_count(),
            mesh.triangle_count()
        );
    }

    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&o.out)?), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(&canvas.to_srgb8())?;
    println!("wrote {} ({w}x{h})", o.out);
    Ok(())
}
