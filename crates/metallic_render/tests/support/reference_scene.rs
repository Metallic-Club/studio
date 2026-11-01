//! The 3D reference scene of `docs/design/look-presets.md` 6.1 (the dummy is plain parts), shared
//! by the lighting preset tests and `examples/lighting_presets.rs`.
use metallic_core::{Color3, Value, Vec3 as V3, World};
use metallic_render::Camera;
use metallic_render::glam::Vec3;

struct Part<'a> {
    size: [f64; 3],
    pos: [f64; 3],
    color: [f64; 3],
    material: &'a str,
    shape: &'a str,
    yaw: f64,
    transparency: f64,
}

const fn part<'a>(size: [f64; 3], pos: [f64; 3], color: [f64; 3], material: &'a str) -> Part<'a> {
    Part {
        size,
        pos,
        color,
        material,
        shape: "Block",
        yaw: 0.0,
        transparency: 0.0,
    }
}

fn add(w: &mut World, p: &Part<'_>) {
    let ws = w.root("Workspace").expect("workspace");
    let id = w.create("Part", Some(ws)).expect("part");
    let v = |a: [f64; 3]| Value::Vector3(V3::new(a[0], a[1], a[2]));
    w.set(id, "Size", v(p.size)).expect("size");
    w.set(id, "Position", v(p.pos)).expect("position");
    w.set(id, "Orientation", v([0.0, p.yaw, 0.0]))
        .expect("orientation");
    w.set(
        id,
        "Color",
        Value::Color3(Color3::new(p.color[0], p.color[1], p.color[2])),
    )
    .expect("color");
    w.set(id, "Material", Value::String(p.material.into()))
        .expect("material");
    w.set(id, "Shape", Value::String(p.shape.into()))
        .expect("shape");
    w.set(id, "Transparency", Value::Number(p.transparency))
        .expect("transparency");
    w.set(id, "Anchored", Value::Bool(true)).expect("anchored");
}

/// The reference scene with default lighting.
pub fn reference_world() -> World {
    let mut w = World::new_with_services();
    let parts = [
        part(
            [128.0, 1.0, 128.0],
            [0.0, -0.5, 0.0],
            [163.0, 162.0, 165.0],
            "Concrete",
        ),
        Part {
            shape: "Ball",
            ..part([6.0; 3], [-8.0, 3.0, 0.0], [196.0, 40.0, 28.0], "Plastic")
        },
        Part {
            yaw: 30.0,
            ..part(
                [6.0; 3],
                [0.0, 3.0, 0.0],
                [13.0, 105.0, 172.0],
                "SmoothPlastic",
            )
        },
        Part {
            shape: "Ball",
            ..part([5.0; 3], [8.0, 2.5, 0.0], [200.0, 200.0, 205.0], "Metal")
        },
        Part {
            shape: "Wedge",
            ..part(
                [6.0, 4.0, 8.0],
                [-4.0, 2.0, -10.0],
                [160.0, 95.0, 53.0],
                "Wood",
            )
        },
        Part {
            transparency: 0.5,
            ..part(
                [4.0, 8.0, 1.0],
                [6.0, 4.0, -8.0],
                [180.0, 210.0, 230.0],
                "Glass",
            )
        },
        part(
            [4.0, 20.0, 4.0],
            [14.0, 10.0, -16.0],
            [155.0, 90.0, 70.0],
            "Brick",
        ),
        part(
            [20.0, 1.0, 20.0],
            [-20.0, 0.5, -20.0],
            [75.0, 151.0, 75.0],
            "Grass",
        ),
        // Dummy: legs, torso, arms, head.
        part(
            [1.0, 2.0, 1.0],
            [-2.5, 1.0, 6.0],
            [40.0, 60.0, 110.0],
            "Plastic",
        ),
        part(
            [1.0, 2.0, 1.0],
            [-1.5, 1.0, 6.0],
            [40.0, 60.0, 110.0],
            "Plastic",
        ),
        part(
            [2.0, 2.0, 1.0],
            [-2.0, 3.0, 6.0],
            [52.0, 142.0, 64.0],
            "Plastic",
        ),
        part(
            [1.0, 2.0, 1.0],
            [-3.5, 3.0, 6.0],
            [234.0, 184.0, 146.0],
            "Plastic",
        ),
        part(
            [1.0, 2.0, 1.0],
            [-0.5, 3.0, 6.0],
            [234.0, 184.0, 146.0],
            "Plastic",
        ),
        Part {
            shape: "Ball",
            ..part([2.0; 3], [-2.0, 5.0, 6.0], [234.0, 184.0, 146.0], "Plastic")
        },
    ];
    for p in &parts {
        add(&mut w, p);
    }
    let swatches = [
        [220.0, 40.0, 40.0],
        [240.0, 140.0, 30.0],
        [240.0, 220.0, 40.0],
        [60.0, 180.0, 70.0],
        [40.0, 190.0, 200.0],
        [40.0, 80.0, 220.0],
        [190.0, 60.0, 190.0],
        [240.0, 240.0, 240.0],
    ];
    for (i, c) in swatches.iter().enumerate() {
        let x = -7.0 + 2.0 * i as f64;
        add(
            &mut w,
            &part([2.0, 0.2, 2.0], [x, 0.1, 10.0], *c, "SmoothPlastic"),
        );
    }
    for i in 0..6 {
        let g = 255.0 * (0.1 + 0.18 * i as f64);
        let x = -5.0 + 2.0 * i as f64;
        add(
            &mut w,
            &part([2.0, 0.2, 2.0], [x, 0.1, 13.0], [g; 3], "SmoothPlastic"),
        );
    }
    for i in 0..5 {
        let z = -60.0 - 30.0 * i as f64;
        add(
            &mut w,
            &part(
                [6.0, 12.0, 6.0],
                [0.0, 6.0, z],
                [120.0, 120.0, 130.0],
                "Plastic",
            ),
        );
    }
    w
}

/// The perspective reference camera.
pub fn reference_camera() -> Camera {
    Camera {
        position: Vec3::new(0.0, 9.0, 26.0),
        look_at: Vec3::new(0.0, 2.5, -4.0),
        fov_deg: 70.0,
    }
}

/// Luma (Rec. 709 weights on sRGB-encoded values, 0..1) of every pixel of an RGBA8 image.
pub fn lumas(px: &[u8]) -> Vec<f32> {
    px.as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])) / 255.0
        })
        .collect()
}

/// Mean, 1st and 99th percentile of `values`.
pub fn stats(values: &[f32]) -> (f32, f32, f32) {
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    let at = |q: f32| v[((v.len() - 1) as f32 * q) as usize];
    (mean, at(0.01), at(0.99))
}

/// Mean colourfulness: the average of max(r,g,b) - min(r,g,b) over pixels, 0..1.
pub fn chroma(px: &[u8]) -> f32 {
    let p = px.as_chunks::<4>().0;
    p.iter()
        .map(|q| {
            let hi = q[0].max(q[1]).max(q[2]);
            let lo = q[0].min(q[1]).min(q[2]);
            f32::from(hi - lo) / 255.0
        })
        .sum::<f32>()
        / p.len() as f32
}
