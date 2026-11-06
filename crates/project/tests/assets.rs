//! Decoding a project's models and textures (#189): the committed test assets, a `.gltf` with its buffer in another
//! member or a data URI, broken files, and reading assets by id through an [`AssetReader`] on another thread.
//!
//! # The test assets
//!
//! `tests/fixtures/textured_cube.glb`, `quadrants.png` and `quadrants.jpg` were made for this repository and are
//! under its licence; nothing in them comes from anywhere else. The GLB and the PNG are made by the generator below
//! (`METALLIC_UPDATE_FIXTURES=1 cargo test -p metallic_project --test assets` writes them again); the JPEG is the
//! PNG saved as a JPEG at quality 95 by Windows' own encoder (System.Drawing), since no JPEG encoder is a
//! dependency. The cube is 2 m on a side, placed by its node 3 m along X, and each face shows the whole image: four
//! coloured quadrants, so a test can tell which way up a texture is drawn.

use std::fs;
use std::path::{Path, PathBuf};

use metallic_project::{AssetError, Project, Template, decode_image, decode_mesh};

/// The quadrants' colours: top left, top right, bottom left, bottom right.
const QUADRANTS: [[u8; 3]; 4] = [[220, 40, 40], [40, 180, 60], [40, 80, 220], [240, 200, 40]];
/// The image's side, in pixels.
const IMAGE_SIZE: u32 = 16;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// The cube's buffer: positions, normals, texture coordinates (24 vertices, four per face) and 36 `u16` indices.
fn cube_buffer() -> Vec<u8> {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut normal = [0.0f32; 3];
            normal[axis] = sign;
            let mut u = [0.0f32; 3];
            u[(axis + 1) % 3] = 1.0;
            let mut v = [0.0f32; 3];
            v[(axis + 2) % 3] = 1.0;
            let base = (positions.len() / 3) as u16;
            // Corners (−u −v), (+u −v), (+u +v), (−u +v) of the face, the image's bottom left, bottom right, top
            // right and top left.
            for (su, sv, uv) in [
                (-1.0, -1.0, [0.0, 1.0]),
                (1.0, -1.0, [1.0, 1.0]),
                (1.0, 1.0, [1.0, 0.0]),
                (-1.0, 1.0, [0.0, 0.0]),
            ] {
                for i in 0..3 {
                    positions.push(normal[i] + su * u[i] + sv * v[i]);
                }
                normals.extend(normal);
                uvs.extend::<[f32; 2]>(uv);
            }
            // u × v is +normal on the positive faces: counter-clockwise from outside as listed; flipped on the others.
            let cross = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let outward = cross.iter().zip(normal).map(|(a, b)| a * b).sum::<f32>() > 0.0;
            let order: [u16; 6] = if outward {
                [0, 1, 2, 0, 2, 3]
            } else {
                [0, 2, 1, 0, 3, 2]
            };
            indices.extend(order.map(|i| base + i));
        }
    }
    let mut buffer = Vec::new();
    for value in positions.iter().chain(&normals).chain(&uvs) {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
    for index in indices {
        buffer.extend_from_slice(&index.to_le_bytes());
    }
    buffer
}

/// The cube's glTF JSON, its one buffer at `uri` (`None`: a GLB's binary chunk).
fn cube_json(uri: Option<&str>) -> String {
    let uri = uri.map_or(String::new(), |uri| format!(r#","uri":"{uri}""#));
    format!(
        concat!(
            r#"{{"asset":{{"version":"2.0","generator":"Metallic test fixture (crates/project/tests/assets.rs)"}},"#,
            r#""scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0,"translation":[3,1,-2]}}],"#,
            r#""meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2}},"indices":3}}]}}],"#,
            r#""buffers":[{{"byteLength":840{uri}}}],"#,
            r#""bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":288,"target":34962}},"#,
            r#"{{"buffer":0,"byteOffset":288,"byteLength":288,"target":34962}},"#,
            r#"{{"buffer":0,"byteOffset":576,"byteLength":192,"target":34962}},"#,
            r#"{{"buffer":0,"byteOffset":768,"byteLength":72,"target":34963}}],"#,
            r#""accessors":[{{"bufferView":0,"componentType":5126,"count":24,"type":"VEC3","min":[-1,-1,-1],"max":[1,1,1]}},"#,
            r#"{{"bufferView":1,"componentType":5126,"count":24,"type":"VEC3"}},"#,
            r#"{{"bufferView":2,"componentType":5126,"count":24,"type":"VEC2"}},"#,
            r#"{{"bufferView":3,"componentType":5123,"count":36,"type":"SCALAR"}}]}}"#
        ),
        uri = uri
    )
}

/// The cube as a GLB: the header, the JSON chunk (padded with spaces) and the binary chunk.
fn cube_glb() -> Vec<u8> {
    let mut json = cube_json(None).into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let bin = cube_buffer();
    assert_eq!(bin.len(), 840);
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    glb
}

/// The quadrants image's pixels.
fn quadrant_pixels() -> Vec<u8> {
    let half = IMAGE_SIZE / 2;
    (0..IMAGE_SIZE)
        .flat_map(|y| (0..IMAGE_SIZE).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let [r, g, b] = QUADRANTS[usize::from(y >= half) * 2 + usize::from(x >= half)];
            [r, g, b, 255]
        })
        .collect()
}

fn quadrants_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, IMAGE_SIZE, IMAGE_SIZE);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("PNG header");
    writer.write_image_data(&quadrant_pixels()).expect("PNG data");
    writer.finish().expect("PNG end");
    bytes
}

#[test]
fn the_committed_assets_are_the_generators() {
    if std::env::var("METALLIC_UPDATE_FIXTURES").is_ok_and(|value| value == "1") {
        fs::write(fixture("textured_cube.glb"), cube_glb()).unwrap();
        fs::write(fixture("quadrants.png"), quadrants_png()).unwrap();
    }
    assert_eq!(fs::read(fixture("textured_cube.glb")).unwrap(), cube_glb());
    // The PNG is compared by its pixels: another `png` version may compress the same pixels differently.
    let png = decode_image(&fs::read(fixture("quadrants.png")).unwrap()).unwrap();
    assert_eq!((png.width, png.height), (IMAGE_SIZE, IMAGE_SIZE));
    assert_eq!(png.rgba, quadrant_pixels());
}

/// Checks a decoded cube: 24 vertices, 12 triangles facing out, fitted to the unit cube, the image's top on +Y's
/// side faces.
fn check_cube(mesh: &metallic_project::MeshAsset) {
    assert_eq!(mesh.positions.len(), 24);
    assert_eq!((mesh.normals.len(), mesh.uvs.len()), (24, 24));
    assert_eq!(mesh.triangles(), 12);
    assert_eq!(mesh.size, [2.0, 2.0, 2.0], "the size before fitting");
    for p in &mesh.positions {
        assert!(
            p.iter().all(|c| (c.abs() - 0.5).abs() < 1e-6),
            "fitted and centred: {p:?}"
        );
    }
    for t in mesh.indices.as_chunks::<3>().0 {
        let p = |i: u32| mesh.positions[i as usize];
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let centroid = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        let outward: f32 = cross.iter().zip(centroid).map(|(x, y)| x * y).sum();
        assert!(outward > 0.0, "triangle {t:?} faces inward");
        let n = mesh.normals[t[0] as usize];
        let along: f32 = cross.iter().zip(n).map(|(x, y)| x * y).sum();
        assert!(along > 0.0, "triangle {t:?} disagrees with its normal");
    }
    for n in &mesh.normals {
        assert!((n.iter().map(|c| c * c).sum::<f32>() - 1.0).abs() < 1e-5);
    }
    // On the +Z face the image's top (v = 0) is the +Y edge.
    for ((p, n), uv) in mesh.positions.iter().zip(&mesh.normals).zip(&mesh.uvs) {
        if n[2] > 0.5 {
            assert_eq!(uv[1] == 0.0, p[1] > 0.0, "{p:?} {uv:?}");
        }
    }
}

#[test]
fn the_glb_cube_decodes_fitted_to_the_unit_cube() {
    let bytes = fs::read(fixture("textured_cube.glb")).unwrap();
    let mesh = decode_mesh(&bytes, |uri| Err(format!("no buffer `{uri}` was expected"))).unwrap();
    check_cube(&mesh);
}

#[test]
fn a_gltf_reads_its_buffer_from_a_data_uri() {
    let encoded = base64(&cube_buffer());
    let json = cube_json(Some(&format!("data:application/octet-stream;base64,{encoded}")));
    let mesh = decode_mesh(json.as_bytes(), |uri| Err(format!("unexpected `{uri}`"))).unwrap();
    check_cube(&mesh);
}

#[test]
fn broken_models_and_images_are_errors_not_panics() {
    let not_a_model = decode_mesh(b"hello", |_| Err(String::new())).unwrap_err();
    assert!(not_a_model.to_string().starts_with("not a glTF model"), "{not_a_model}");
    // The buffer is shorter than it says.
    let short = cube_json(Some("data:application/octet-stream;base64,AAAA"));
    let error = decode_mesh(short.as_bytes(), |_| Err(String::new())).unwrap_err();
    assert!(error.to_string().contains("fewer than the 840"), "{error}");
    let missing = cube_json(Some("cube.bin"));
    let error = decode_mesh(missing.as_bytes(), |_| Err("not there".into())).unwrap_err();
    assert_eq!(error.to_string(), "buffer `cube.bin`: not there");
    // Every truncation of the GLB fails cleanly.
    let glb = cube_glb();
    for len in [0, 4, 12, 20, 100, glb.len() - 1] {
        assert!(decode_mesh(&glb[..len], |_| Err(String::new())).is_err(), "{len} bytes");
    }

    assert_eq!(
        decode_image(b"GIF89a").unwrap_err().to_string(),
        "not a PNG or JPEG image"
    );
    let png = quadrants_png();
    assert!(decode_image(&png[..png.len() / 2]).is_err());
    let jpeg = fs::read(fixture("quadrants.jpg")).unwrap();
    assert!(decode_image(&jpeg[..jpeg.len() / 3]).is_err());
}

#[test]
fn a_jpeg_decodes_to_rgba() {
    let image = decode_image(&fs::read(fixture("quadrants.jpg")).unwrap()).unwrap();
    assert_eq!((image.width, image.height), (IMAGE_SIZE, IMAGE_SIZE));
    // Lossy: each quadrant's middle is close to its colour.
    for (quadrant, colour) in QUADRANTS.iter().enumerate() {
        let x = 4 + 8 * (quadrant as u32 % 2);
        let y = 4 + 8 * (quadrant as u32 / 2);
        let at = ((y * IMAGE_SIZE + x) * 4) as usize;
        let pixel = &image.rgba[at..at + 4];
        for (got, want) in pixel.iter().zip(colour) {
            assert!(
                got.abs_diff(*want) <= 24,
                "quadrant {quadrant}: {pixel:?} for {colour:?}"
            );
        }
        assert_eq!(pixel[3], 255);
    }
}

#[test]
fn an_image_too_large_for_the_device_is_halved() {
    let mut image = decode_image(&quadrants_png()).unwrap();
    image.fit(16);
    assert_eq!((image.width, image.height), (16, 16), "it fits already");
    image.fit(5);
    assert_eq!((image.width, image.height), (4, 4));
    assert_eq!(image.rgba.len(), 4 * 4 * 4);
    // Each quadrant is still its colour: the halving averages within quadrants here.
    assert_eq!(&image.rgba[..4], &[220, 40, 40, 255]);
    assert_eq!(&image.rgba[image.rgba.len() - 4..], &[240, 200, 40, 255]);
}

/// A project made from the Baseplate template for one test, with `members` written and indexed.
fn project_with(test: &str, members: &[(&str, Vec<u8>)]) -> Project {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("assets").join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a test project");
    let template = Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d"))
        .expect("a test project");
    let mut project = Project::create(dir.join("Game.mtproj"), &template, "Game").expect("a test project");
    for (member, bytes) in members {
        project.write(member, bytes.clone()).expect("a test project");
    }
    project.refresh_assets().expect("a test project");
    project
}

#[test]
fn assets_are_read_by_id_on_another_thread() {
    let mut project = project_with(
        "by_id",
        &[
            ("Assets/Models/cube.gltf", cube_json(Some("cube.bin")).into_bytes()),
            ("Assets/Models/cube.bin", cube_buffer()),
            ("Assets/quadrants.png", quadrants_png()),
            ("Assets/notes.txt", b"not an image".to_vec()),
        ],
    );
    let id = |project: &Project, member: &str| project.assets().id(member).unwrap().to_string();
    let (model, image, notes) = (
        id(&project, "Assets/Models/cube.gltf"),
        id(&project, "Assets/quadrants.png"),
        id(&project, "Assets/notes.txt"),
    );
    // Saved, so the reader reads the archive on disk; then a removal that is not saved yet.
    project.save().unwrap();
    let removed = id(&project, "Assets/Models/cube.bin");
    assert!(project.remove("Assets/Models/cube.bin"));
    let mut reader = project.asset_reader();
    drop(project);

    let worker = std::thread::spawn(move || {
        let error = reader.load_mesh(&model).unwrap_err();
        assert!(
            error.to_string().contains("buffer `cube.bin`"),
            "the .bin is removed in the reader's view: {error}"
        );
        let image = reader.load_image(&image).unwrap();
        assert_eq!((image.width, image.height), (IMAGE_SIZE, IMAGE_SIZE));
        let notes = reader.load_image(&notes).unwrap_err();
        assert!(
            notes.to_string().starts_with("`Assets/notes.txt` could not be decoded"),
            "{notes}"
        );
        assert!(matches!(reader.read(&removed), Err(AssetError::Missing { .. })));
        assert!(matches!(reader.read("nope"), Err(AssetError::BadId { .. })));
        assert!(matches!(
            reader.read("00000000000000000000000000000000"),
            Err(AssetError::UnknownId { .. })
        ));
    });
    worker.join().unwrap();
}

#[test]
fn a_gltf_reads_its_buffer_from_the_member_beside_it() {
    let project = project_with(
        "beside",
        &[
            ("Assets/Models/cube.gltf", cube_json(Some("cube.bin")).into_bytes()),
            ("Assets/Models/cube.bin", cube_buffer()),
            (
                "Assets/Models/escape.gltf",
                cube_json(Some("../../metallic.toml")).into_bytes(),
            ),
        ],
    );
    let mut reader = project.asset_reader();
    let model = project.assets().id("Assets/Models/cube.gltf").unwrap().to_string();
    check_cube(&reader.load_mesh(&model).unwrap());
    // A URI never reaches outside the project.
    let escape = project.assets().id("Assets/Models/escape.gltf").unwrap().to_string();
    let error = reader.load_mesh(&escape).unwrap_err();
    assert!(error.to_string().contains("`..` path parts are not allowed"), "{error}");
}

/// Standard base64 with padding, for the data URI test.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}
