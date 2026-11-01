# Task 3: Renderer (`metallic_render`)

Written after the fact for #107 (the task shipped without a progress note). State as of main on 2026-10-31.

## Goal
Issue #3: draw the Parts of a `World` with wgpu, with shadows, sky and fog, and an offscreen capture for tests and
screenshots, plus a way for systems to see what changed in a `World` without walking the whole tree.

## Done
- PR #59 (`studio/3-renderer`, merged 2026-10-30) delivered:
  - `metallic_render`: an instanced renderer for Box, Ball, Cylinder and Wedge parts, with cascaded shadows, PBR
    materials, fog, a gradient sky with a sun disc, and offscreen `capture()`.
  - `metallic_core`: the change journal (`World::change_cursor`, `changes_since`, `Changes::Ids` / `Changes::Overflow`,
    bounded at 16,384 entries). `Scene::sync` reads it, so only changed parts are re-read; a full rebuild happens on the
    first sync or after an overflow.
- The implementer's numbers at the time: the stress example (5,000 parts, 500 moved per frame) averaged 349 fps, with
  a 99th-percentile frame of 4.3 ms. Syncing 50 changed parts out of 5,000 took 97.5 µs; a release test asserts it
  stays under 1 ms.
- Deferred items from that PR became their own issues: instance buffers only grow, then partial uploads (#105);
  blending on non-sRGB targets and the HDR pipeline (#62); Studio drawing the offscreen texture (Task 10, #10).
- Later work on the crate has its own notes, e.g. `109-outline-msaa.md`, `149-lighting-presets.md`,
  `151-auto-exposure.md`, `276-render-heap-corruption.md`.

## Next
Nothing for Task 3 itself. Use the crate's current issues (`gh issue list -R Metallic-Club/studio -l area:render`).

## Commands that matter
```
cargo test -p metallic_render -j 4
cargo run -p metallic_render --example stress -j 4 -- --screenshot <png>
```
GPU tests skip when there is no adapter. Under heavy machine load the headless binary has crashed (#276): rerun it
alone before calling a failure real.

## Where to read
`ARCHITECTURE.md`: the metallic_core bullet (change journal) and the metallic_render bullet.
