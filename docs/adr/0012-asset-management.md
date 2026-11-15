# ADR 0012: Asset management: the browser, materials as assets, linked scenes

- **Status:** Accepted (2026-11-15, the owner's asset management spec)
- **Issues:** #676 (the browser), #677 (script files), #678 (material assets), #679 (texture channels), #680 (the
  material editor), #681 (thumbnails and the image preview), #682 (several scenes), #199 (linked scenes)
- **Builds on:** [ADR 0006](0006-project-format.md) (projects, the asset index, scenes), [ADR 0008](0008-rendering-performance.md)
  (the material table), [ADR 0009](0009-studio-foundation.md) (Studio).

## Context

The owner wants Roblox Studio's ease of use with Godot's resource and scene management: a project browser, files of
every kind that keep their references when they move, materials that are plain editable assets rather than a fixed
list with variants, and scenes that can be placed inside other scenes as linked instances that follow their source.
Studio's look stays its own; the two editors are references for workflows only.

ADR 0006 already gives the foundation: a project is one zip archive; every member but the manifest and the index has
a stable asset id in `assets.json`; scenes refer to assets by id, and the file operations rewrite the few references
that are paths (a script's `Source`, the start scene).

## Decision

### 1. The browser shows the project's members

- The Assets panel is a folder tree on the left and the open folder's contents as tiles on the right, with a back
  button, breadcrumbs and a search across the project. Both sides read the project's members every frame, so they
  follow every change, whoever made it.
- **Folders exist of their own.** A folder is a zip directory entry, so an empty folder survives a save. A folder
  stays when its last file leaves it. Moving or deleting a folder moves or deletes everything in it.
- The editor's own data (`.studio/`) and the project's metadata (`metallic.toml`, `assets.json`) are not shown.
- A tile shows a file's name without its type's extension; renaming keeps the extension, so a file never changes
  type by accident.
- File operations stay in memory until Save, like every edit. What the editor holds follows a move: the scene being
  edited (a script's `Source`), the script editor's open files, and the browser's own paths.

### 2. Script files carry their type in their name

`Door.server.luau` is a Script, `Door.client.luau` a LocalScript and `Door.luau` a ModuleScript (Rojo's convention,
which Luau tooling knows). The type is part of the extension, so renaming keeps it.

### 3. Materials are assets

- A material is a file (`Assets/Materials/Wood.material`, versioned JSON): base colour tint, metallic, roughness,
  emission colour and strength, normal strength, metres per tile, UV scale and offset, filtering and wrapping, and a
  texture per channel (base colour, normal, roughness, metallic, emission, ambient occlusion) by asset id, with an
  optional packed map and its channel assignment.
- `BasePart.Material` names a material asset by id. The fixed `Material` enum goes, and there are no variants: a
  project's materials are whatever files it has, all editable, replaceable and removable. Empty means the default
  look. The Baseplate template ships the familiar set as plain files.
- The engine loads each material once, whatever number of parts use it, and gives it a row of the renderer's
  material table (ADR 0008). Editing the file changes every part that uses it.
- Textures are packed on load into at most four layers of one texture array per material (albedo, normal, packed
  ambient occlusion, roughness and metallic, emission), so a material costs at most four samples and parts keep
  their draws.

### 4. Scenes

- A project holds many scenes; Studio edits one at a time and shows which.
- **Loading** a scene adds its objects to a world beside what is there; **unloading** removes exactly what it brought.
- **Linking** a scene places it as one object, a linked instance: its children come from the source scene and are
  not saved in the scene that holds the link. The link saves its own transform and a list of overrides (a property of
  a linked child, by the child's scene id), which win over the source until reverted. A change to the source shows
  in every link to it once saved. A link can be made independent: its children become ordinary instances.
- A link names its source by asset id, so renaming or moving the source keeps it. A scene that would link itself,
  directly or through others, is refused.

### 5. Thumbnails are a cache

Tiles show a picture's image, a rendered sphere for a material and a rendered view for a scene. Thumbnails are made
a few a frame and kept in memory by the asset's content; nothing is written into the `.mtproj`.

## Consequences

- Scenes from before #678 hold `Material` enum items; they migrate to the project's material of that name, created
  from the built-in defaults when the project lacks it.
- A deleted material, texture or linked scene is reported by name where it was used, and the part or link falls
  back to the default look or to an empty link; nothing fails to load.
