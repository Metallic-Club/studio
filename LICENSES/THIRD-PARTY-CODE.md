# Third-party code

Source code in Metallic that is ported from, or follows closely, someone else's published code. Each entry names the
files, what was taken, the licence and the copyright notice. Packaging ships this folder with the app.

| Work | Used in | Licence |
| --- | --- | --- |
| Khronos PBR Neutral tone mapper | `crates/metallic_render/shaders/tonecurve.wgsl` (`pbr_neutral`), `crates/metallic_render/src/tonemap.rs` (`pbr_neutral`) | Apache-2.0 |
| AgX minimal (Missing Deadlines / iolite) | `tonecurve.wgsl` (`agx`, `agx_contrast`, `AGX_*`), `tonemap.rs` (`agx`) | MIT |
| ACES fitted (Stephen Hill, from Baking Lab) | `tonecurve.wgsl` (`aces_hill`, `ACES_*`), `tonemap.rs` (`aces_hill`) | MIT |

The Narkowicz ACES fit (`narkowicz`) is a published formula without a code licence; it is credited in the source.

## Khronos PBR Neutral

- Source: https://github.com/KhronosGroup/ToneMapping/tree/main/PBR_Neutral (`pbrNeutral.glsl`).
- Copyright (c) 2024 The Khronos Group Inc. Licensed under the Apache License, Version 2.0; the full text is in
  `LICENSES/Apache-2.0.txt`.
- Changes: ported from GLSL to WGSL and to Rust (CPU mirror for tests and tools); the constants and steps are
  unchanged. The result is pre-scaled by Metallic's curve normalisation in the caller.

## AgX minimal

- Source: https://iolite-engine.com/blog_posts/minimal_agx_implementation ("Minimal AgX Implementation" by Benjamin
  Wrensch), an approximation of Troy Sobotka's AgX.
- Changes: ported from GLSL to WGSL and Rust; the outset result is clamped before the 2.2 power; the Punchy look
  computes luma after the ASC CDL step (as the upstream changelog of 2025-04-20 does).

```text
MIT License

Copyright (c) 2024 Missing Deadlines (Benjamin Wrensch)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## ACES fitted (Baking Lab)

- Source: https://github.com/TheRealMJP/BakingLab (`BakingLab/ACES.hlsl`); the fit is by Stephen Hill.
- Changes: ported from HLSL to WGSL and Rust; matrices kept in published row order.

```text
MIT License

Copyright (c) 2016 MJP

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
