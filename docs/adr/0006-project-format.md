# ADR 0006: Project format and project types

- **Status:** Accepted (2026-11-02, sign-off #9)
- **Issue:** #7
- **Amended 2026-11-05 (#193): owner decision.** A project is one `.mtproj` file (a zip archive), not a folder. The
  owner's words: project files "will be stored in a .mtproj file which is a zipped file that contains project files
  like textures, materials, scripts, scenes, etc." The details below (asset index, saving, working model, zip crate)
  are the conductor's working assumptions; the owner can change them.

## Context

The engine saves and manages projects. A project is 3D or 2D, each with its own physics, rules and editor tools. **Only 3D is built now.** A project is either a **standalone** game (shipped on its own) or a **platform** game (runs inside the Metallic platform, loads assets from platform servers, shows the platform UI). Projects are edited by people and by agents. A single file is easy to share, move and back up; the files inside stay text where they can (TOML, JSON, Luau) so they still read and diff well once unpacked.

## Decision

### A project is one `.mtproj` file

`MyGame.mtproj` is a standard zip archive, so any zip tool can open it. Inside:

```
MyGame.mtproj
├── metallic.toml        manifest (below), at the archive root
├── assets.json          asset index: stable asset ids by member path (#188)
├── Scenes/Main.scene    scenes: versioned JSON, one instance per object, stable key order
├── Scripts/*.luau       script sources; script objects in a scene point at these members
└── Assets/              models (glTF/GLB first), textures (PNG/JPEG), materials, other files
```

- **Member paths** use `/`, are relative to the archive root and are case-sensitive. A member whose path is absolute
  or contains `..` is refused, so extracting can never write outside its folder.
- **Asset ids live in the archive** (`assets.json`), so they survive moving the file to another machine. Scenes
  reference assets by id. Every member but `metallic.toml` and `assets.json` is an asset. An id is 128 random bits
  (32 hex digits), never reused. Each asset's entry also records its size and CRC-32 (the zip checksum), so a member
  that moved or was renamed is recognised and keeps its id; an asset whose member is gone is reported missing (#188).
- **Machine-local caches** (thumbnails, imported or derived data, extracted members) live in the user cache folder
  (`metallic_platform`'s `Paths::cache_dir`), in a folder per project, keyed by a hash of the project file's path. The
  engine or Studio passes that folder in; `metallic_project` does not depend on `metallic_platform` (ADR 0001).
  Deleting the cache is always safe. Nothing machine-local is written into the `.mtproj`.
- **Compression:** text members (manifest, index, scenes, scripts) are deflated. Formats that are already compressed
  (PNG, JPEG, GLB, audio) are **stored** uncompressed, so they can be read in place without inflating.
- **Zip crate:** [`zip`](https://crates.io/crates/zip) (9.x, MIT) with `default-features = false` and
  `features = ["deflate-flate2-zlib-rs"]`: deflate and stored entries only, pure Rust, no C libraries. Checked with
  `cargo deny check` and `cargo check --target aarch64-linux-android` / `aarch64-apple-ios`. It is added to the
  workspace by the first issue that reads or writes a `.mtproj` (#187).

### Working model

- Opening a project reads the central directory, `metallic.toml` and `assets.json`. Other members are read when
  needed, straight from the archive.
- Edits (scenes, scripts, imported assets, renames) are kept in memory until save. The `.mtproj` on disk does not
  change until then.
- **Large assets are streamed from the archive, not extracted.** Because already-compressed formats are stored,
  reading one is a seek to its offset and a plain read. That avoids a second copy on disk and a cache that can go
  stale. A member is extracted to the cache only when something needs a real file path (for example an importer that
  only takes a path); the extracted copy is keyed by the member's content hash.
- **Saving is crash-safe:** write a complete new archive beside the old one (`MyGame.mtproj.tmp`, same folder), flush
  it to disk, then atomically rename it over the old file. Unchanged members are copied raw, without recompressing. A
  crash or failed save leaves the old project intact.

### `metallic.toml`

```toml
[project]
name = "My Game"
description = ""
tags = []
kind = "3d"            # "2d" is reserved and refused until 2D exists
mode = "standalone"    # or "platform"
start_scene = "Scenes/Main.scene"   # a member path inside the archive
engine = "0.1"         # engine version the project was last saved with
```

### Scenes

- Every scene has a `format` version. An unsupported version is refused before anything loads.
- Unknown classes or properties (from a newer or older engine) are skipped and reported as warnings, never crash, and do not shift references.
- References between instances are stored by a stable per-scene id, not by position, so a merge does not silently re-point them.
- Default values are not written, which keeps files small and diffs meaningful.
- Loading never leaves the data model half-loaded.

### Settings live in two places

| What | Where |
|---|---|
| Project settings: name, description, tags, standalone/platform, start scene | `metallic.toml` inside the `.mtproj` (Studio's Project Settings window edits it) |
| Studio settings: keybinds, theme, layout, editor behaviour | the user's config folder, per user, never in the project |
| Player settings: graphics, sensitivity | the user's config folder, per game (#5) |

### Export

`.mtproj` is the source; `game.pak` is the build.

- **Standalone:** the `.mtproj` is packed into one binary file (`game.pak`, read-only, built by `xtask` or Studio) and shipped next to the player executable.
- **Platform:** the project is uploaded to the platform and the player streams it. Designed in M10, but the pak format is shared.

#### `game.pak` (#191)

A pak holds every member of the `.mtproj` (the manifest, `assets.json`, scenes, scripts and assets), so asset ids
resolve as in the project. Nothing is stripped: machine-local data is never in the `.mtproj` to begin with. The
engine reads a pak and a project through one interface (`metallic_project::GameFiles`), and a pak's `AssetReader`
reads assets by id like a project's, so a game runs the same from either.

**Our own layout, not a zip.** The pak is read, never edited, and read in place: by a seek and a read now,
memory-mapped or by HTTP range requests later (platform streaming). A zip does not give that directly: a member's
bytes start after a local header whose length is only known by reading it, aligning them means padding extra fields,
and an asset id would need a custom extra field. Our layout puts every member at an offset the index states, aligned
to 16 bytes, with its id beside it, and is about 300 lines with no new dependency. Members are stored, never
compressed; the reserved flag bits leave room for compression if a later version needs it.

Every number is little-endian:

| Bytes | What |
|---|---|
| 0..8 | magic `MTLCPAK\0` |
| 8..12 | format version, now 1; a newer version is refused with a clear error |
| 12..16 | number of members |
| 16..24 | where the index starts |
| 24..28 | the index's length |
| 28..32 | the index's CRC-32 |
| 32.. | the members' bytes, each as it is in the project, each starting at a multiple of 16 (zero bytes between) |
| last | the index, at a multiple of 16, ending the file |

The index has one entry per member, sorted by path: offset (8 bytes), length (8), CRC-32 (4, the same checksum
`assets.json` and the zip use), flags (1: bit 0 means the member is an asset with an id), the asset id (16, most
significant byte first; zero without one), the path's length (2) and the path (UTF-8).

- **Reading:** opening a pak reads and checks the header and the index, then `metallic.toml` and `assets.json`. Every
  other member is read when asked for, by path or by asset id, with one seek and one read, and checked against its
  CRC-32. Nothing is unpacked to disk.
- **Errors:** a file that is not a pak (bad magic), a pak from a newer engine, a truncated pak, a damaged index (its
  checksum, an unsorted or duplicated path, an unsafe path), an entry outside the members' bytes and a member that
  fails its checksum each give a clear error naming the file, never a panic or wrong data.
- **Deterministic:** packing the same project twice gives the same bytes (members sorted, no dates or machine data).
- **Writing** is crash-safe like a project save: a complete `game.pak.tmp`, flushed, then renamed over the old pak.

### Mode changes player behaviour, not the file format

Standalone and platform projects use the same `.mtproj` format. Only the player's behaviour differs (platform UI, server connection, asset source).

### Version control

A zip does not diff or merge in git. A possible later addition is an unpacked-folder export and import (in `xtask`
or Studio) that writes the members as plain files for teams using git, and packs them back. It is not built now.
