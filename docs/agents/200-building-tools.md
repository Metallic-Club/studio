# Issue #200: building tools research and design (branch `studio/200-building-tools`)

Goal: make building in Studio and in games very easy. Research Roblox plugins, Roblox Studio, building games and
other engines; turn it into one implementation spec with mockups and small follow-up issues. Docs only; no cargo run
(disk and RAM short).

## Done
- `docs/research/2026-10-30-building-tools.md`: 15 Roblox plugins, Roblox Studio's own tools, 13 building games,
  11 engines and editors, 6 colour pickers plus accessible-picker and Oklab notes, a ranked list of 27 ideas, and what
  could not be verified (R7).
- `docs/design/building-tools.md`: gizmos and typed values, snapping, align and gap fill, mirror and arrays, brush,
  selection and measure, colour popover and eyedropper, material picker, `LookOverride` (per-part style) with the
  renderer plan including a masked pixelate pass, Build in Play (F4), `BuildKit`, hints, keymap rows for #129, Luau
  API, budgets.
- `docs/design/building-tools/`: three self-contained HTML mockups with PNGs. The viewport pictures and style
  thumbnails come from a throwaway numpy rasteriser (not committed) that applies the per-part rules of spec 12.3.

## Next
- Follow-up issues listed in the PR; the renderer work waits on #63.
- The other-engines research doc (`docs/research/2026-10-30-building-tools-other-engines.md`) did not exist when this
  was written; cross-check it against research R3 and R4 if it lands.

## Open questions
- Spec section 20, and every "Assistant's call" in section 19.
- Unity default keys for X (space) and Z (pivot) are marked Unverified in section 15.
