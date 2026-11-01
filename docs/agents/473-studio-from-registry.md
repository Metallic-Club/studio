# #473 Studio reads the class registry

Branch `studio/473-studio-from-registry`. Design: `docs/design/class-system.md` (sections 3.2, 11, 14 row 16, 15).

## Goal
Studio reads the class registry instead of hard-coding class and property lists, so a new class or property shows up in
the editor with no Studio change.

## Done
- **Core (`classes/desc.rs`, `resolve.rs`).** `PropDesc::editor` (`Editor`: `Section`, `order`, `label`, `Widget`,
  `VisibleWhen`, `readonly`), `ClassDesc::insert` (`InsertEntry`: `.insert_order(n)`, `.no_insert()`), `ClassDesc::icon_like`,
  `Registry::insertable()`, `SchemaError::VisibleWhenWithoutProperty`. None of it is hashed: `registry_hash_is_pinned`
  passes with the golden unchanged.
- **Class files.** Sections, number units, the Style label and the Script and StylePicker widgets moved into them; a
  description on all 280 class-property pairs is a test now (hidden ones excepted). 13 classes have an insert order and
  19 have `.no_insert()`. The abstract classes' icon rows moved from `class_icon` to `.icon_like(...)`.
- **Studio.** `properties.rs` lost `section`, `SECTION_NAMES` and the name matches in `number_kind` and `row`. `sections()`
  groups the rows, `row_label()` and `row_tooltip()` come from the descriptor. `INSERTABLE` is gone: `editor::insertable()`
  reads the registry for the Insert menu, Explorer's Insert, Shift+A and Ctrl+K. Tooltips: the Properties class name and rows,
  and every Insert list.
- **Docs.** ARCHITECTURE "How to add a class" says what to set; the design's `editor` row and example match the code.

## Visible lists: no change
Same 13 classes in the same order. All 13 are `CREATABLE`, so none was wrong. The registry's other 19 creatable classes stay
out with `.no_insert()`: Camera, Frame, GuiButton, Atmosphere, BloomEffect, Clouds, ColorCorrectionEffect, ColorGradingEffect,
Sky, InputAction, InputBinding, InputContext, RemoteFunction, UnreliableRemoteEvent, Humanoid, Player, PlayerGui,
PlayerScripts, StarterPlayerScripts. Turning one on is deleting that line. The list stays flat (no category headers), so
it looks as before; the categories only decide the order. `Instance.new` is unchanged.

## How "looks the same" was checked
- `tests/panel_pictures.rs` (ignored) renders with no window; `before/` came from the base code, a commit before any change.
  `after/` compared with `cmp`: Part, Model, Script, TextLabel, the Insert menu and Shift+A are byte-identical. The Ctrl+K
  picture differs only by the new tooltip, because the pointer rests on a row. Looked at them.
- A temporary test, not committed, held the old `section` and `number_kind` against every class property of the registry
  (280 pairs), with Style, Script and StylePicker: all matched. It was then deleted.

## Acceptance: one class in one file
- **Committed test:** `a_new_class_shows_in_insert_and_properties_from_its_descriptor_alone` (`properties.rs`) feeds fixture
  descriptors through `Registry::insertable` and `sections`, the functions Studio uses.
- **Real run, not committed:** `src/classes/effects/test_beacon.rs` alone, built and captured, then removed. Pictures in
  `473-studio-from-registry/test-class/`: it is in the Insert menu and Shift+A, and Properties shows its own section, slider and
  tooltip. In `cargo test -p metallic_core` and Studio's unit tests only `registry_hash_is_pinned` failed, as it should (the
  full workspace was not run with that class in place).

## Decisions where the brief left room
- `group` in the design is `section`, the issue's word. A `Section` carries its own order, so Studio keeps no list of them.
- A `CREATABLE` class is listed unless it opts out, so a new class needs nothing to show up; the existing extras opt out.
- `icon_like` was added beyond the brief: `every_registry_class_has_its_own_icon` must stay unedited and fails a class under
  `Instance`, so without it such a class would have needed a line in `icons.rs`.
- `Widget::Script` and `Widget::StylePicker` were added: `Source` and `LookOverride` open Studio tools, and a name match is what
  this issue removes. Slider, multiline, enum and read-only have no built-in user yet (the panel must look the same); a
  test draws each one. `Widget::Asset` draws a plain text field: Studio has no asset picker yet.
- A slider draws in front of the usual number field, so it shows the exact value and accepts typed expressions.
- The filter also matches the label, so "style" finds LookOverride. Nothing changes until someone types.
- `read()` and `commit()` keep `WorldPivot` and `Size`: those are behaviour, for computed properties and `script_write`
  (#464), not display.

## Noticed, not fixed
- `BloomEffect.Size` sits in Transform, an accident of the old name match, kept so the panel stays the same.
- `FieldOfView` and `MaxSlopeAngle` are degrees but show as studs, also as before. `.drag(NumberUnit::Degrees)` fixes each.
- Number fields have no accessible name. egui_kittest's check fails if a field is drawn on a harness's first run.
- Methods, signals and callbacks have descriptions but no Studio surface to show them on. Explorer rows have no class tooltip.
- `class_icon`'s name table is still hard-coded. A new class falls back to its superclass, so nothing is required of it.
- `Frame` is not in Insert although `TextLabel` and the buttons are; lighting and input classes cannot be inserted either.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_core -j 4 --test class_registry --test replication
cargo test -p metallic_studio -j 4 --lib -- properties:: palette:: icons::
METALLIC_PICTURES_DIR=/full/path cargo test -p metallic_studio --test panel_pictures -j 4 -- --ignored
find crates/metallic_core/src/classes -mindepth 2 -name '*.rs' -print0 | xargs -0 rustfmt --edition 2024 --check
```
PR #501 (`studio/481-properties-label-align`) changes row layout in `properties.rs`: `git merge-tree --write-tree HEAD
studio/481-properties-label-align` is clean. Merged, its dropdown and short rows change height, so re-capture the pictures.

## Next
Nothing for the issue. Not pushed; the PR is the conductor's batch.

## Open questions
None blocking. Which of the 19 hidden classes should be offered (`Frame` first, perhaps) is for the owner.

## Last check results
See the section below, kept current by the last commit.
