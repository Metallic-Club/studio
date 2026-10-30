# 180 input actions (design)

Goal: design spec for one input binding model (keyboard and mouse, gamepad, touch). Docs only; no cargo run.

Done: `docs/design/input-actions.md`, mockup `docs/design/input-actions/touch-layout.html`, PR and five follow-up issues
(numbers in the spec, section 15).

Next: owner review of section 17 (decisions and unverified items). Then phase 1 (runtime core) can start.

Commands: none needed. Mockup is a static HTML file (fonts and tokens inlined from `docs/design/visual-language/assets/`).

Open questions: see spec section 17 (shape glyph legality, winit safe-area API, one-handed preset keys).
