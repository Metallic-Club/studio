# #226 Building surfaces: header, contextual toolbar, status line, hints

Branch `studio/226-building-surfaces`, from main after #337.

## Done
- Viewport header (spec 3.1): a 36 px strip over the viewport with the tools, the increment chips ("1 stud", "15°",
  click for the list, scroll to step), World/Local, Pivot/Centre, Grid/Surface/Vertex/Edge/Align chips and
  Collisions at the right. The top bar keeps Edit and Play. `viewport_header.rs` never scrolls: tools and
  increments always show, the rest stay by priority (Grid, Surface, World/Local, Vertex, Edge, Collisions,
  Pivot/Centre, Align) using last frame's measured widths, and what does not fit goes into a "»" menu; when even
  that is too narrow the tools drop their keycaps. Kittest checks 1280 and 1600 px (every header button inside the
  strip). Screenshots: `226-header-1280.png`, `226-header-1600.png`.
- Collisions (`snap.collisions`, off by default): a `collisions::Guard` is built once per drag with a uniform
  8-stud grid of the parts that are not moving (scales to 10k parts and big selections); parts the selection
  already overlaps at the start are ignored (a slab sunk in its rim, an Alt-drag copy on its source). Each frame
  `GizmoDrag::limit` sweeps the motion in steps of half the thinnest moved part, then bisects in the first
  blocked step, so parts stop at contact and never tunnel. Past 50,000 box tests it fails safe (held) with a
  status note. Review fixes (#384 FIX-FIRST): this, focus requested once by the bar and released on an outside
  click, docked bar avoids the gizmo, Esc rule in spec 14.1, corrupt `hints.ron` moved aside with a warning.
- Contextual toolbar (spec 3.2) in `context_bar.rs`: placement above / below / docked, never over the selection or
  gizmo (test with 20 seeded camera poses), 200 ms still-pointer delay after a selection change, hidden during drags,
  box select, camera flight and Play, 90 ms fade (`theme::motion`), Ctrl+. focus with Left/Right/Esc, View menu
  toggle. Items: tool (cycles), increment, colour swatch (split when mixed), Align, Mirror, Array when registered,
  More (Anchored, CanCollide, Group).
- Status line (spec 3.3): drag readout moved to the right side next to count and size, in mono.
- Hints (spec 14.1) in `hints.rs`: 8 hints, detectors fed by `EditorUi::watch_hints`, rules tested; card bottom-left (Esc dismisses it when the pointer is on the card or a card button has focus; otherwise Esc keeps its editor meaning)
  with keycap, sentence, "Show me", "Got it"; Esc dismisses; `hints.ron`; Settings "Show tips" and "Reset tips".

## Coordination with #380 (`studio/wire-editor-panels`)
The bar looks actions up by id (`colour.open`, `build.align`, `build.mirror`, `build.array`, later `material.open`,
`style.open`) and skips missing ones; it registers none of them. Whichever of #380 and this PR merges second
resolves the toolbar hunk in `editor_ui.rs` (this PR moved Tools/Settings/Snap into the viewport header).

## Open
- Style hint: `HintEvent::LookPicker` exists but nothing sends it until the Look picker lands.
- Collisions in Properties edits and keyboard nudges is not covered (drags only, as the spec says).
