# Asset library

Free, permissively licensed textures, HDRI skies and models so Metallic games look good out of the box. Git holds
only the **manifest** (what to download, from where, under which licence, with which SHA-256). The files themselves
are downloaded into a local cache by `tools/assets/fetch.py`, verified, and converted for the engine.

Status: **implemented** (manifest, fetch/convert, tests). Not yet consumed by the renderer or Studio.

## Layout
| Path | What |
|---|---|
| `assets/manifest/sources.toml` | Each source: licence proof URL, what we saw, bundling/CDN terms, allowed download hosts, crawl delay |
| `assets/manifest/textures.toml` | PBR texture sets: one `primary` + at least two `alternate` per Part material, plus `extra`, `stylized` and `pixel` sets, and whole pixel-art texture packs |
| `assets/manifest/hdris.toml` | HDRI skies (day, sunset, overcast, night) and interiors; each has a 4k `.hdr` (sky) and a 1k `.hdr` (preview, irradiance) |
| `assets/manifest/models.toml` | glTF/GLB models by `category`: platformer, nature, prop, building, weapon, horror, character, animal |
| `LICENSES/ASSETS.md` | Generated attribution list (do not edit; `fetch.py --licenses-only` rewrites it) |
| `tools/assets/fetch.py` | Download, verify, unpack, convert; writes `LICENSES/ASSETS.md` |
| `tools/assets/curate.py` | Add an asset (checks the licence on its own page, hashes the files) or audit all licences |
| `tools/assets/previews.py`, `blender_render.py` | Contact sheets (`docs/agents/36-asset-library/*.jpg`) |
| `tools/assets/test_fetch.py` | Tests (`python -I tools/assets/test_fetch.py`) |

## Use it
```
python -m venv %LOCALAPPDATA%\Metallic\asset-venv
%LOCALAPPDATA%\Metallic\asset-venv\Scripts\pip install -r tools/assets/requirements.txt
set PY=%LOCALAPPDATA%\Metallic\asset-venv\Scripts\python -I
%PY% tools/assets/fetch.py --check                 # validate the manifest (no network)
%PY% tools/assets/fetch.py                         # everything (several GB; resumable)
%PY% tools/assets/fetch.py --kind texture --sizes 512,1024
%PY% tools/assets/fetch.py --only "tex-brick-*" "hdri-*"
%PY% tools/assets/test_fetch.py                    # tests
```
Run Python with `-I` (isolated mode) so a file in the current folder can never shadow a standard module; Pillow is
found because it lives in the venv. Downloads are data: the tools hash, unzip (allow-listed file types only, no
`..`/absolute paths/symlinks, zip-bomb limits) and decode images; nothing downloaded is ever executed.

Cache (default `%LOCALAPPDATA%/Metallic/asset-cache`, or `--cache`, or `METALLIC_ASSET_CACHE`):
```
blobs/<aa>/<sha256>              verified downloads, content-addressed (shared between assets)
assets/<id>/...                  files laid out by their manifest path, archives unpacked
converted/<id>/<size>/albedo.png sRGB colour (RGBA when the set has opacity)
converted/<id>/<size>/normal.png tangent-space normal, OpenGL convention (+Y up); DirectX sources are flipped
converted/<id>/<size>/orm.png    linear: R = ambient occlusion, G = roughness, B = metalness (glTF order)
converted/<id>/<size>/height.png linear height, when the set has one
converted/<id>/material.json     sizes, maps present, ORM defaults used, tile size, tint mode, filter
```
Sizes default to 512, 1024 and 2048 and are never upscaled. Missing maps are filled in ORM from the Part material's
preset (`MATERIAL_DEFAULTS` in `fetch.py`, matching the renderer's planned material table). Pixel sets
(`nearest = true`) keep their native size and say `"filter": "nearest"`. Re-running skips everything already verified
or converted (stamp files hold input hashes and the converter version); interrupted downloads resume with HTTP Range.
Output is deterministic (fixed PNG settings, sorted archive members, no timestamps).

## Fields
Every `[[asset]]` has: `id` (unique, `kind-...`), `kind` (`texture`, `texture_pack`, `hdri`, `model`, `model_pack`),
`name`, `source` (key in `sources.toml`), `source_id`, `page_url`, `download_url` (first file), `licence` (SPDX id),
`licence_proof_url`, `licence_evidence` (what we saw on the asset's own page or API record), `licence_checked`
(date), `author`, `attribution` (ready-to-print credit line), `format`, `resolution`, `size` (bytes, sum of files),
`tags`, `look` (look preset: `realistic`, `stylized`, `toon`, `pixel`, `lowpoly`), and one or more `[[asset.files]]`
with `url`, `sha256`, `size`, `map` (`albedo`, `normal_gl`, `normal_dx`, `roughness`, `ao`, `metalness`, `height`,
`opacity`, `archive`, `hdri`, `model`, `include`, `other`) and `path` (where it lands under `assets/<id>/`).
Textures add `material` (a Part material from `metallic_core::classes::MATERIALS`) and `role` (`primary`,
`alternate`, `extra`, `stylized`, `pixel`), optionally `tile_size_m` and an `[asset.texture]` table (`members` to pick
maps inside an archive, `nearest`, `convert = "none"`, `tint = "multiply" | "overlay"`, `roughness`/`metalness`
defaults). Models add `category`, and when known `polycount` and `animated`.

## Licence policy
1. **CC0 / public domain first.** All current entries are CC0 1.0.
2. **CC-BY only if clearly marked** on the asset's own page, with the `attribution` line recorded (validation enforces
   it and `LICENSES/ASSETS.md` lists it under "Required attributions"). `curate.py` refuses CC-BY unless `--allow-by`.
3. **Never:** CC-BY-SA, NC, ND, GPL/LGPL/AGPL, "personal use only", "free for non-commercial", unclear or missing
   licences, AI-generated or AI-processed packs, or packs that re-upload other people's work.
4. **Per-asset proof.** Before an asset goes in, its own page (or, for Poly Haven, its API record, because Poly Haven
   asks for the API instead of page scraping) is fetched and the licence statement is recorded in
   `licence_evidence`. If it can't be verified, it doesn't go in. `curate.py audit` re-checks every entry.
5. **Bundling and CDN.** CC0 lets us ship files in the installer and mirror them on our CDN without credit. Each
   source's own terms are in `sources.toml` (`bundling`); the important ones: Poly Haven's live API needs a named
   User-Agent and a visible credit when our software shows content fetched live from it (mirrored copies need none),
   and its website renders/thumbnails are not CC0, so we never reuse them.
6. **Be polite.** A descriptive User-Agent, and robots.txt crawl-delays (10 s for OpenGameArt and Poly Pizza pages).
   Sources whose robots.txt forbids automated access are not fetched automatically (see Excluded).

### Excluded and why
- **Kenney (kenney.nl)**: CC0 and excellent, but robots.txt disallows AI user agents (`ClaudeBot`, `Claude-User`) site
  wide and `/assets/` + `/downloads/` for everyone. Not fetched by tools. A human may download packs and add them as a
  new source later (open question for the owner). Kenney models re-uploaded to Poly Pizza were not used either.
- **Quaternius site**: CC0, but downloads go through Google Drive. We use the same models from Poly Pizza, where the
  Quaternius account uploaded them itself with a CC0 licence per model.
- **"Big pack of hand painted tiling textures" (OpenGameArt)**: CC0, but the author says neural style transfer was
  used to make it; excluded under the no-AI rule.
- **Poly Pizza models marked CC-BY 3.0** (for example "Animated Base Character" by Quaternius): not taken (CC0 only for now).
- **Archives in .7z** (some OpenGameArt stylized sets): stdlib can't unpack them safely; skipped.
- **Poly Haven trees** (`pine_tree_01`, `fir_tree_01`, ...): millions of triangles, unsuitable as game props.

## Add an asset
1. Find it on a source in `sources.toml` (or add a new source with its licence page, bundling terms, download hosts
   and crawl delay; check its robots.txt first).
2. Run `curate.py add`, for example:
   ```
   %PY% tools/assets/curate.py add polyhaven brick_wall_10 --kind texture --material Brick --role alternate
   %PY% tools/assets/curate.py add ambientcg Tiles074 --kind texture --material Concrete --role extra --tags tiles
   %PY% tools/assets/curate.py add polypizza <model id from poly.pizza/m/...> --kind model --category prop --look lowpoly
   %PY% tools/assets/curate.py add opengameart <page slug> --kind texture --material Grass --role stylized \
        --look stylized --file https://opengameart.org/sites/default/files/<file>.png=albedo
   ```
   It refuses assets whose page doesn't show an accepted licence, hashes every file, and validates the manifest.
3. Look at it: `previews.py textures|hdris|models` and open the contact sheet. Replace weak picks.
4. `fetch.py --licenses-only` to refresh `LICENSES/ASSETS.md`, run the tests, commit the manifest change.
Never commit downloaded files. Upstream changed a file? `fetch.py` fails with a SHA-256 mismatch; re-run `curate.py add`
for that asset after checking the licence again.
