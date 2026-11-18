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
a few a frame and kept in memory by the asset's content; nothing is written into the `.mtproj`. A material's is the
ball alone (#697): the ball is in the middle of the view, so its outline is a circle, and the tile shows that disc of
the picture, no sky.

### 6. Working in the browser (2026-11-15, #693-#710)

- **A script file's type is its script's (#701)** when a script in the scene runs it, whatever its name says, so a
  Script saved as `Name.luau` before #677 shows as a Script; a file no script runs keeps its name's type (§2).
- **Tiles or a list (#699)**, switched at the top right. The list's header sorts by Name, Type, Date created or Size,
  again the other way, folders first; rows do what tiles do. **Date created** is the asset index's own record (#710):
  `assets.json` keeps, per asset, when it was first indexed (`added`, seconds since 1970), kept through moves; assets
  from before have none. It is the project's fact, the same on every machine, so the index stays free of
  machine-local data.
- **Ctrl and the wheel scale** the tiles (their pictures; names keep their size) or the rows (#696). The tiles are
  squarer (`radius::TILE`, #698) and kept off the area's edges (#700); the search is wider (#695); the line beside the
  folder tree is drawn over it (#694).
- **Menus (#702, #705)**: the New items are a submenu of one New item; every file and folder has Copy path.
- **Buttons on tiles (#703, #704)**: while the pointer is on a material's tile, a button gives it to the selected
  parts; on a script file's, one adds a script running it inside each selected object. Each is one undo step. A
  plain click on a material applied it too (#680) until #735: now it only selects it (§7).
- **Importing (#706-#708)**: File, Import; Import in the browser's menus and its toolbar; and files dragged in from
  the system's file manager. Each copies files, and folders with what is in them, into a project folder as new
  assets, names made unique (up to 10000 files, 32 folders deep). The window does not say where in it files were
  dropped, so a drop goes into the browser's open folder, which the browser shows while files are dragged over.

### 7. The inspector (2026-11-16, #733-#742)

- **An inspector at the browser's right (#740)** shows the one file or folder selected: its picture or icon, its
  name (renamed in place), its path, and its type, size or contents and date created. Its width is dragged; closed,
  it stays closed until something else is selected.
- **Materials are edited there (#742)**, not in a tab of their own: the ball alone, drawn from the thumbnails' camera
  and cut to the ball as a tile is (§5), drawn again only when the material or its size changes; the name with Apply
  and Copy; the path; then Colour, Normal, Roughness, Metallic, Emission, Ambient occlusion, Packed map and Mapping.
  Undo and Redo with the Assets panel focused take back a material's edits (each material keeps its own steps), not
  the scene's. A click on a material only selects it (#735); its tile's button (a pointer clicking, #733, scaled
  with the tiles, #734), the inspector's Apply and a drop on a part give it to parts.
- **Folder colours (#741)** tint a folder's icon in the tree, the tiles and the list. They are Studio's, kept in the
  project's editor data as `.studio/browser.json` by path (as the Explorer's are in `.studio/explorer.json`, #603),
  and follow renames, moves, deletes and copies.
- **The tree (#736, #737)** starts at its left edge, and a press on a caret dragged across others opens or closes
  them all. **The list's columns (#738)** are resized from its header; Name takes what the others leave until its
  own edge is dragged.
- **A part's Material in Properties (#739)** is a field of the material's name, its path while it is edited, with a
  button that opens a window to choose one, as Godot's resource slots do.

### 8. Asset fields and thumbnails (2026-11-16, #756-#762)

- **One asset field** serves a part's Material and the material editor's texture maps (#762): the name, the path
  while it is edited, completions while typing (#760: names that start with the text first; Up, Down, Enter, Tab or
  a click), and a window to choose one as a list or a grid of thumbnails (#761), the choice of view kept for the next
  window. A picture's field shows a small preview at its left. A material's value may be a material's name; a
  picture's is always an asset id, a name typed giving the one picture of that name.
- **The material editor (#762)** puts the texture maps first, in the order Color, Normal, Roughness, Metalness,
  Emissive, Ambient occlusion, each a picture field with no slider; the values (tint, Glass, normal strength,
  roughness, metalness, emissive colour and strength) follow in their own section, then Packed map and Mapping.
- **Thumbnails reach the fields** through `Files::with_pictures`: a field asks for a file's thumbnail, and from the
  next frame on the Properties panel and the inspector have it (a material's ball, asked of the thumbnails' renderer;
  a picture, decoded a couple a frame). The tree shows them too, in place of a material's or a picture's icon (#756).
- **The inspector's ball (#757)** is painted from the part of the preview's target the frame fills: the target keeps
  room beyond the size drawn, as the viewport's does.

### 9. Normal maps (2026-11-17, #773)

Normal maps follow the **OpenGL convention**: red tilts the surface toward +U and green toward the picture's up,
as glTF's and Roblox's maps do. A DirectX map needs its green channel inverted. The renderer builds the tangent
frame from screen-space derivatives (Schüler's cotangent frame); WGSL's `dpdy` runs down the screen where GLSL's
`dFdy` runs up, so the frame is the GLSL one negated, which had left red inverted against either convention until
#773. A GPU test lights a tilted map from each side to hold the convention.

### 10. Assets in the Properties panel (2026-11-17, #778, #779)

The browser's inspector (§7) moved into the **Properties panel**: a file or folder picked in the browser after the
instances were selected shows there in their place (an instance selected afterwards shows again, and a click on the
file brings it back). Its icon, name and type head it, a preview (a picture, a material's ball) follows, and its
properties are categories of the panel's own rows and inputs: File (name, path, type, size, created, a folder's
colour), and a material's Texture Maps, Surface (numbers typed and dragged, colours with swatches: no sliders),
Packed Map and Mapping. Undo in the Assets or the Properties panel takes back a material's edits while one shows.
Every category's heading collapses it (#778).

### 11. A picture's filtering (2026-11-17, #781)

A picture is **Smooth** or **Pixelated**, set in its properties and kept in `assets.json` beside its id
(`"filter": "pixelated"`, absent for smooth, so older indexes read as before and the format stays 1). It moves and
changes with its file. Its uses follow it: the picture window and its thumbnails, a MeshPart's `TextureId` (sampled
nearest), and a material whose Filtering is **From the pictures** (`"filter": "picture"`, the new default), which is
pixelated when the first picture it uses is. A material can still choose Smooth (`"linear"`) or Nearest. The picture
window zooms (Ctrl and the wheel, or its buttons).

## Consequences

- Scenes from before #678 hold `Material` enum items; they migrate to the project's material of that name, created
  from the built-in defaults when the project lacks it.
- A deleted material, texture or linked scene is reported by name where it was used, and the part or link falls
  back to the default look or to an empty link; nothing fails to load.
