//! Writes every shape as a Wavefront OBJ file, for checking the geometry in Blender or any mesh viewer.
//!
//! ```text
//! cargo run -p metallic_meshgen --example export_obj -- [OUT_DIR] [--lod low|medium|high] [--bevel WIDTH]
//! ```
//!
//! Each shape is built at its default size and saved as `OUT_DIR/<Name>.obj` (default `meshgen-obj/`).
//! OBJ is +Y up and right-handed like the generator. Keep Blender's importer axes at their defaults
//! (Forward -Z, Up Y); it converts +Y up to Blender's +Z up.
use metallic_meshgen::{Bevel, Lod, MeshDesc, Shape, generate};
use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let mut out = PathBuf::from("meshgen-obj");
    let mut lod = Lod::High;
    let mut bevel = Bevel::SUBTLE;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--lod" => {
                lod = match value()?.to_ascii_lowercase().as_str() {
                    "low" => Lod::Low,
                    "medium" => Lod::Medium,
                    "high" => Lod::High,
                    other => return Err(format!("unknown LOD {other:?}").into()),
                }
            }
            "--bevel" => bevel = Bevel::round(value()?.parse()?),
            s if s.starts_with('-') => return Err(format!("unknown option {s}").into()),
            s => out = PathBuf::from(s),
        }
    }

    std::fs::create_dir_all(&out)?;
    for shape in Shape::ALL {
        let desc = MeshDesc::new(shape, shape.default_size())
            .with_lod(lod)
            .with_bevel(bevel);
        let mesh = generate(&desc)?;
        let path = out.join(format!("{}.obj", shape.name()));
        std::fs::write(&path, mesh.to_obj(shape.name()))?;
        println!(
            "{:<16} {:>6} verts {:>6} tris -> {}",
            shape.name(),
            mesh.vertex_count(),
            mesh.triangle_count(),
            path.display()
        );
    }
    Ok(())
}
