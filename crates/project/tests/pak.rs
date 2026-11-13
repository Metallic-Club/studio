//! `game.pak` (#191): packing a project, reading it back without unpacking it, packing it twice to the same bytes,
//! and every way a damaged or foreign pak is refused.

use std::fs;
use std::path::{Path, PathBuf};

use metallic_project::{
    AssetError, GameFiles, PAK_FORMAT, PAK_MAGIC, Pak, PakError, Project, ProjectError, SceneIds, Template, save_scene,
};

const MODEL: &str = "Assets/Models/textured_cube.glb";
const TEXTURE: &str = "Assets/Textures/quadrants.png";
const SCRIPT: &str = "Scripts/Main.luau";

/// An empty folder for one test.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("pak").join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).expect("a fixture")
}

/// A saved project in `dir`: the Baseplate template with a script, a model and a texture, indexed.
fn project(dir: &Path) -> Project {
    let template = Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d"))
        .expect("the Baseplate template");
    let mut project = Project::create(dir.join("Game.mtproj"), &template, "Packed").expect("a project");
    project.write(SCRIPT, "print('hello from the pak')").expect("a script");
    project.write(MODEL, fixture("textured_cube.glb")).expect("a model");
    project.write(TEXTURE, fixture("quadrants.png")).expect("a texture");
    project.refresh_assets().expect("indexed");
    project.save().expect("saved");
    project
}

/// A project packed into `dir/game.pak`, and the pak's bytes.
fn packed(dir: &Path) -> (Project, PathBuf, Vec<u8>) {
    let mut project = project(dir);
    let path = dir.join("game.pak");
    Pak::pack(&mut project, &path).expect("packed");
    let bytes = fs::read(&path).expect("the pak");
    (project, path, bytes)
}

/// The scene a game's start scene loads into a fresh game, saved again as text.
fn start_scene(files: &mut dyn GameFiles) -> String {
    let mut dm = metallic_classes::new_game().expect("a game");
    let loaded = files.load_start_scene(&mut dm).expect("the start scene");
    assert_eq!(loaded.warnings, []);
    save_scene(&dm, &mut SceneIds::new()).expect("saved").text
}

/// One index entry as the layout says: where its 8-byte offset field is, the member's offset and length, its path.
struct Entry {
    at: usize,
    offset: u64,
    length: u64,
    path: String,
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
}

/// The index of a pak, read by hand from the documented layout.
fn entries(bytes: &[u8]) -> Vec<Entry> {
    let count = u32_at(bytes, 12);
    let mut at = usize::try_from(u64_at(bytes, 16)).expect("an offset");
    let mut entries = Vec::new();
    for _ in 0..count {
        let path_len = usize::from(u16::from_le_bytes([bytes[at + 37], bytes[at + 38]]));
        entries.push(Entry {
            at,
            offset: u64_at(bytes, at),
            length: u64_at(bytes, at + 8),
            path: String::from_utf8(bytes[at + 39..at + 39 + path_len].to_vec()).expect("a path"),
        });
        at += 39 + path_len;
    }
    assert_eq!(at, bytes.len(), "the index ends the file");
    entries
}

/// Writes the CRC-32 of the (edited) index into the header, so only the edit is wrong.
fn fix_index_crc(bytes: &mut [u8]) {
    let start = usize::try_from(u64_at(bytes, 16)).expect("an offset");
    let crc = crc32fast::hash(&bytes[start..]);
    bytes[28..32].copy_from_slice(&crc.to_le_bytes());
}

/// What opening `bytes` as a pak gives, written to `dir/name`.
fn open_bytes(dir: &Path, name: &str, bytes: &[u8]) -> Result<Pak, ProjectError> {
    let path = dir.join(name);
    fs::write(&path, bytes).expect("written");
    Pak::open(path)
}

fn pak_error(result: Result<Pak, ProjectError>) -> PakError {
    match result {
        Err(ProjectError::Pak { source, .. }) => source,
        other => panic!("expected a pak error, got {other:?}"),
    }
}

#[test]
fn a_pak_reads_back_the_projects_scenes_scripts_and_assets() {
    let dir = scratch("round_trip");
    let (mut project, path, bytes) = packed(&dir);
    let mut pak = Pak::open(&path).expect("opened");

    assert_eq!(GameFiles::members(&pak), project.members());
    assert_eq!(pak.manifest(), project.manifest());
    assert_eq!(pak.assets(), project.assets());
    assert_eq!(pak.warnings(), []);
    for member in project.members().into_iter().map(str::to_owned).collect::<Vec<_>>() {
        assert_eq!(
            GameFiles::read(&mut pak, &member).expect("a pak member"),
            project.read(&member).expect("a project member"),
            "{member}"
        );
    }
    assert_eq!(
        pak.read_text(SCRIPT).expect("the script"),
        "print('hello from the pak')"
    );
    assert_eq!(start_scene(&mut pak), start_scene(&mut project));

    // Assets by id: from the index entry's id, and through an asset reader on another thread.
    let model = project.assets().id(MODEL).expect("indexed");
    let texture = project.assets().id(TEXTURE).expect("indexed");
    assert_eq!(
        pak.read_id(model).expect("by id"),
        (MODEL.to_owned(), fixture("textured_cube.glb"))
    );
    let mut from_project = project.asset_reader();
    let mut from_pak = pak.asset_reader();
    assert_eq!(from_pak.path(), path);
    let worker = std::thread::spawn(move || {
        let mesh = from_pak.load_mesh(&model.to_string()).expect("the mesh");
        let image = from_pak.load_image(&texture.to_string()).expect("the image");
        (mesh, image)
    });
    let (mesh, image) = worker.join().expect("the worker");
    assert_eq!(mesh, from_project.load_mesh(&model.to_string()).expect("the mesh"));
    assert_eq!(mesh.triangles(), 12);
    assert_eq!(image, from_project.load_image(&texture.to_string()).expect("the image"));

    // The layout: the magic and version, every member aligned to 16 bytes and stored as it is.
    assert_eq!(bytes[..8], PAK_MAGIC);
    assert_eq!(u32_at(&bytes, 8), PAK_FORMAT);
    assert_eq!(u64_at(&bytes, 16) % 16, 0);
    let listed = entries(&bytes);
    assert_eq!(
        listed.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
        project.members()
    );
    for entry in &listed {
        assert_eq!(entry.offset % 16, 0, "{}", entry.path);
        let stored = &bytes[entry.offset as usize..(entry.offset + entry.length) as usize];
        assert_eq!(stored, project.read(&entry.path).expect("a member"), "{}", entry.path);
    }
}

#[test]
fn packing_twice_gives_the_same_bytes() {
    let dir = scratch("deterministic");
    let (mut project, _, first) = packed(&dir);
    drop(project);
    project = Project::open(dir.join("Game.mtproj")).expect("opened again");
    let again = dir.join("again.pak");
    Pak::pack(&mut project, &again).expect("packed");
    assert_eq!(fs::read(&again).expect("the pak"), first);
    // Packing over an existing pak replaces it, and leaves no temporary file.
    Pak::pack(&mut project, &again).expect("packed over");
    assert_eq!(fs::read(&again).expect("the pak"), first);
    assert!(!dir.join("again.pak.tmp").exists());
}

#[test]
fn unsaved_edits_are_packed() {
    let dir = scratch("edits");
    let mut project = project(&dir);
    project.write(SCRIPT, "print('edited')").expect("an edit");
    assert!(project.remove(TEXTURE));
    let path = dir.join("game.pak");
    Pak::pack(&mut project, &path).expect("packed");
    let mut pak = Pak::open(&path).expect("opened");
    assert_eq!(pak.read_text(SCRIPT).expect("the script"), "print('edited')");
    assert!(!GameFiles::members(&pak).contains(&TEXTURE));
    // The texture's id is still indexed (it is missing, not forgotten), as in the project.
    let texture = project.assets().id(TEXTURE).expect("indexed").to_string();
    assert!(matches!(
        pak.asset_reader().read(&texture),
        Err(AssetError::Missing { .. })
    ));
}

#[test]
fn missing_members_and_unknown_ids_are_errors() {
    let dir = scratch("missing");
    let (_, path, _) = packed(&dir);
    let mut pak = Pak::open(&path).expect("opened");
    assert!(matches!(
        GameFiles::read(&mut pak, "Scripts/Nope.luau"),
        Err(ProjectError::MissingMember { .. })
    ));
    let unknown = metallic_project::AssetId::from_u128(7);
    let error = pak.read_id(unknown).expect_err("no such asset");
    assert!(matches!(error, ProjectError::UnknownAsset { .. }), "{error}");
    assert!(matches!(
        pak.asset_reader().read(&unknown.to_string()),
        Err(AssetError::UnknownId { .. })
    ));
}

#[test]
fn a_file_that_is_not_a_pak_is_refused() {
    let dir = scratch("not_a_pak");
    let project = project(&dir);
    // The source project is a zip, not a pak.
    let error = pak_error(Pak::open(project.path()));
    assert_eq!(error, PakError::NotAPak);
    assert!(error.to_string().contains("not a game.pak"), "{error}");
    assert_eq!(pak_error(open_bytes(&dir, "short.pak", b"PK")), PakError::NotAPak);
    assert!(matches!(
        Pak::open(dir.join("nowhere.pak")),
        Err(ProjectError::Io { .. })
    ));
}

#[test]
fn a_pak_from_a_newer_engine_is_refused() {
    let dir = scratch("newer");
    let (_, _, mut bytes) = packed(&dir);
    bytes[8..12].copy_from_slice(&(PAK_FORMAT + 1).to_le_bytes());
    let error = pak_error(open_bytes(&dir, "newer.pak", &bytes));
    assert_eq!(
        error,
        PakError::NewerVersion {
            version: PAK_FORMAT + 1,
            supported: PAK_FORMAT
        }
    );
    assert!(error.to_string().contains("newer engine"), "{error}");
}

#[test]
fn a_truncated_pak_is_refused() {
    let dir = scratch("truncated");
    let (_, _, bytes) = packed(&dir);
    let error = pak_error(open_bytes(&dir, "cut.pak", &bytes[..bytes.len() - 10]));
    assert!(matches!(error, PakError::Truncated { part: "index", .. }), "{error}");
    assert!(error.to_string().starts_with("truncated"), "{error}");
    let error = pak_error(open_bytes(&dir, "header.pak", &bytes[..20]));
    assert!(matches!(error, PakError::Truncated { part: "header", .. }), "{error}");
    assert!(matches!(
        pak_error(open_bytes(&dir, "empty.pak", b"")),
        PakError::Truncated { size: 0, .. }
    ));
}

#[test]
fn an_index_entry_outside_the_data_is_refused() {
    let dir = scratch("out_of_range");
    let (_, _, original) = packed(&dir);
    let entry = entries(&original)
        .into_iter()
        .find(|e| e.path == SCRIPT)
        .expect("the script");
    // Past the members' bytes, into the index.
    let mut bytes = original.clone();
    bytes[entry.at + 8..entry.at + 16].copy_from_slice(&u64::MAX.to_le_bytes());
    fix_index_crc(&mut bytes);
    let error = pak_error(open_bytes(&dir, "range.pak", &bytes));
    assert!(
        matches!(&error, PakError::OutOfRange { member, .. } if member == SCRIPT),
        "{error}"
    );
    assert!(error.to_string().contains("outside the members' bytes"), "{error}");

    // Inside the header.
    let mut bytes = original;
    bytes[entry.at..entry.at + 8].copy_from_slice(&4u64.to_le_bytes());
    fix_index_crc(&mut bytes);
    assert!(matches!(
        pak_error(open_bytes(&dir, "header.pak", &bytes)),
        PakError::OutOfRange { .. }
    ));
}

#[test]
fn a_damaged_member_or_index_fails_its_checksum() {
    let dir = scratch("checksum");
    let (_, _, bytes) = packed(&dir);
    let listed = entries(&bytes);
    let model = listed.iter().find(|e| e.path == MODEL).expect("the model");
    let script = listed.iter().find(|e| e.path == SCRIPT).expect("the script");

    // A member: the pak opens, and reading that member fails, by path and through the asset reader.
    let mut damaged = bytes.clone();
    damaged[script.offset as usize] ^= 0xFF;
    damaged[model.offset as usize + 100] ^= 0xFF;
    let mut pak = open_bytes(&dir, "member.pak", &damaged).expect("the index is intact");
    let error = GameFiles::read(&mut pak, SCRIPT).expect_err("damaged");
    match &error {
        ProjectError::Pak {
            source:
                PakError::Checksum {
                    member,
                    expected,
                    found,
                },
            ..
        } => {
            assert_eq!(member, SCRIPT);
            assert_ne!(expected, found);
        }
        other => panic!("expected a checksum error, got {other:?}"),
    }
    assert!(error.to_string().contains("is damaged"), "{error}");
    let id = pak.assets().id(MODEL).expect("indexed").to_string();
    let error = pak.asset_reader().load_mesh(&id).expect_err("damaged");
    assert!(
        matches!(error, AssetError::Read { .. }) && error.to_string().contains("is damaged"),
        "{error}"
    );
    // The members around them are fine.
    assert!(pak.read_text("Scenes/Main.scene").is_ok());

    // The index: refused on open.
    let mut damaged = bytes.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 0x20;
    let error = pak_error(open_bytes(&dir, "index.pak", &damaged));
    assert!(
        matches!(&error, PakError::Damaged { reason } if reason.contains("CRC-32")),
        "{error}"
    );
}

/// #603: the editor's own data (`.studio/`) is not packed into the game, and is not an asset.
#[test]
fn a_pak_leaves_out_the_editor_s_data() {
    let dir = scratch("editor_data");
    let mut project = project(&dir);
    project.write(".studio/explorer.json", "{}").expect("the editor's data");
    project.refresh_assets().expect("indexed");
    assert_eq!(project.assets().id(".studio/explorer.json"), None);
    let path = dir.join("game.pak");
    Pak::pack(&mut project, &path).expect("packed");
    let pak = Pak::open(&path).expect("opened");
    let members = GameFiles::members(&pak);
    assert!(
        !members.iter().any(|member| member.starts_with(".studio/")),
        "{members:?}"
    );
    assert_eq!(members.len() + 1, project.members().len());
}
