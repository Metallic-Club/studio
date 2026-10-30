# #36 Free asset library: manifest, licence proof, fetch/convert tooling

Branch `studio/36-asset-library`. Status: **implemented, tested** (Python tests; nothing in Rust changed).

## Goal
Lots of free, high-quality, permissively licensed textures, HDRIs and models so games look good out of the box, with
per-asset licence proof and SHA-256, a fetch/convert tool, and nothing large in git.

## Done
- `assets/manifest/` (183 entries, all CC0 1.0, every file hashed):
  - 50 texture sets + 4 pixel texture packs. Every Part material (Plastic, SmoothPlastic, Metal, Wood, Concrete,
    Glass, Grass, Brick, Ice, Sand) has 1 `primary` + 2 or 3 `alternate`; 3 `extra`; 7 `stylized`/toon (hand painted,
    OpenGameArt) and 8 `pixel` (N64-style 32 px) sets mapped to materials.
  - 20 HDRIs from Poly Haven (4 day, 4 sunset, 3 overcast/fog, 3 night, 6 interior), each 4k + 1k `.hdr`.
  - 109 models: 55 Poly Haven (realistic glTF, 1k textures) + 54 Poly Pizza (Quaternius low poly GLB; 16 animated).
    Categories: prop 23, nature 19, building 14, horror 15, weapon 14 (FPS guns), platformer 10, animal 8, character 6.
  - `sources.toml`: licence proof, evidence, bundling/CDN terms, download hosts, crawl delays per source.
- `tools/assets/fetch.py`: download (resume with Range, per-host delay), SHA-256 verify, safe unzip (allow-listed
  types, no traversal/symlinks, bomb limits), texture conversion to 512/1K/2K `albedo`/`normal` (GL)/`orm`/`height`
  PNG + `material.json`, deterministic, stamp-based skip; writes `LICENSES/ASSETS.md`.
- `tools/assets/curate.py`: `add` (checks the licence on the asset's own page/API record, hashes files, writes
  the entry) and `audit` (re-checks every licence statement).
- `tools/assets/test_fetch.py`: 25 tests (manifest validation, licence rules, TOML round trip, download/resume/
  mismatch/oversize against a local HTTP server, zip safety, map-name detection, conversion output, determinism,
  stamps, pixel native size, CLI, committed `LICENSES/ASSETS.md` freshness, real-manifest coverage rules).
- `tools/assets/previews.py` + `blender_render.py`: contact sheets rendered from the CC0 files themselves
  (`docs/agents/36-asset-library/{textures,hdris,models}.jpg`). Weak picks replaced after looking: pixel Brick (blue
  bricks), Grass (yellow) and Ice (pink crystal) swapped for red bricks, green grass and tintable snow; dropped two weak models (Wizard scene, flat Brick Wall) and one near-black toon rock texture.
- `assets/README.md`: how it works, fields, licence policy, exclusions, how to add an asset.

## Commands
```
python -m venv <venv> && <venv>/Scripts/pip install -r tools/assets/requirements.txt
<venv>/Scripts/python -I tools/assets/test_fetch.py
<venv>/Scripts/python -I tools/assets/fetch.py --check
<venv>/Scripts/python -I tools/assets/fetch.py --only "tex-brick-*" --cache <dir>      # full: ~1.3 GB download
<venv>/Scripts/python -I tools/assets/fetch.py --licenses-only                          # after manifest edits
<venv>/Scripts/python -I tools/assets/curate.py audit                                   # re-check licences
<venv>/Scripts/python -I tools/assets/previews.py textures --cache <dir> --cleanup      # contact sheet
```

## Next
- Renderer (Task #3 / `metallic_render`, not touched here): load the primary set per Part material from the cache
  (see the PR body for the proposed table/array layout and world-space tiling with `tile_size_m`).
- Studio Toolbox (#17, #20): browse the manifest, insert models, show attribution; mirror blobs on our CDN by SHA-256.
- Converter: KTX2/BC7 output with mips (needs a permissive encoder), normal renormalisation after resize.
- More stylized sets (Kenney would fill this if the owner downloads packs by hand, see open questions).

## Open questions (owner)
1. Kenney: robots.txt blocks AI agents, so the tools don't fetch it. Download packs by hand and add a `kenney`
   source with locally computed hashes? (CC0, very good for stylized/platformer.)
2. Default texture resolution to ship in the installer: 1K (about 30 MB for one set per material as PNG; less once
   compressed) with 2K downloaded on demand? Assistant's call so far: convert all three, ship decision open.
3. Poly Pizza has no terms page; per-model CC0 is clear, but should we ask them before mirroring ~54 models?
4. The `OpenGameArt` author field is the submitter; for the N64 packs and hand-painted sets the page text says the
   submitter made them. Fine for CC0, but confirm we want "submitter" as credit.

## Assistant's calls
- Poly Haven licence proof comes from its API record per asset plus the site licence page, because Poly Haven's
  ToS forbids page scraping and its llms.txt points to the API.
- Kenney excluded from automation (robots.txt). The no-AI rule excluded one OpenGameArt pack (neural style transfer).
- Converted textures are PNG (lossless, deterministic); ORM is glTF order (R=AO, G=roughness, B=metalness); normals
  are OpenGL convention. Missing maps get the Part material's preset values.
- Poly Haven models at 1k textures, textures at 2k source, HDRIs 4k sky + 1k preview.
- Disk on this PC was nearly full during the session (other sessions' builds); previews fetched one asset at a
  time and deleted raw files right after (`--cleanup`). Nothing was left in `%LOCALAPPDATA%/Metallic/asset-cache`.
