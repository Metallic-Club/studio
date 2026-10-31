# #222: `LookOverride` on BasePart and Model

Status: tested.

- `BasePart.LookOverride` and `Model.LookOverride`: String, default `inherit`, allowed `inherit`, `realistic`,
  `soft`, `toon`, `low-poly`, `voxel`, `pixel-3d`, `retro-32` (building-tools spec 12.2). Each is appended last in its
  class, so lookups of the older properties scan no further. On write, a value outside the allowed ids goes through
  the new `PropDef::normalize` (`look_override::look_override_id`): display names and the Look's own name, ignoring
  case, spaces and punctuation ("Retro / PS1", "Retro 32-bit" give `retro-32`). Anything else is refused as before.
  The extra step runs only on that refusal path, so accepted writes cost the same.
- Resolution (`metallic_core::look_override::resolve`): own value, else the nearest ancestor Model with a value that
  is not `inherit` (Folders and other classes pass through, a part does not pass its style to child parts), else
  `None` for the place Look (`Lighting.Look` lands with #64).
- Cache (`LookOverrides`): built by one tree walk, then kept current from `World::changes_since` (the existing coarse
  journal, so no dependency on #317 or #318). Part entries re-resolve when the journal names them; a Model re-resolves
  its subtree only when its own value changed, so dragging or renaming a Model does not walk its parts. Reparenting
  already journals the whole subtree. Overflow rebuilds. Only overridden parts and Models with a value are stored.
- Scene files: saved like any non-default property, so `inherit` is omitted.
- Renderer: `Scene` owns a `LookOverrides`; `PartDraw::style` is the resolved id, and parts below a restyled Model
  are re-read without a rebuild. Drawing them differently is #223.

Tests: `crates/metallic_core/tests/look_override.rs` covers the following, and every step checks the cache against
`resolve` and against a fresh rebuild:
- writes and normalisation;
- nearest Model through nested Models and a Folder;
- falling back as values are cleared;
- reparenting parts and whole groups, clones and destroys;
- unrelated Model edits;
- journal overflow;
- 400 seeded edits;
- the scene round trip.

There is also a renderer `Scene` test and a Luau test (`part.LookOverride = "Retro / PS1"` reads `retro-32`).

Coordination: #317 and #318 also edit `instance.rs` and `lib.rs`. This branch changes only the `allowed` check in
`World::set` and adds one `pub mod` line.

Review follow-ups (folded into #63):
- While no Model has a value, a journaled part reads only its own property (no ancestor walk).
- `visit` checks the class before reading `LookOverride`, so Folders, Scripts and services build no error on rebuild.
- For #223: `apply` returns only parts whose resolved style changed. When the place Look (`Lighting.Look`) changes,
  the renderer must also redraw every part that resolves to `None` (they follow the place Look).
