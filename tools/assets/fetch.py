#!/usr/bin/env python3
"""Fetch, verify and convert the Metallic free asset library.

Reads the manifest (`assets/manifest/*.toml`), downloads every file into a cache outside git, checks each file's
SHA-256 against the manifest, unpacks archives safely, converts PBR texture sets to 512/1K/2K PNGs plus a packed
ORM texture, and writes `LICENSES/ASSETS.md` (attributions).

    python -I tools/assets/fetch.py                      # everything, default cache
    python -I tools/assets/fetch.py --kind texture --sizes 512
    python -I tools/assets/fetch.py --only "tex-brick-*" --cache D:/metallic-cache
    python -I tools/assets/fetch.py --licenses-only      # just rewrite LICENSES/ASSETS.md
    python -I tools/assets/fetch.py --check              # validate the manifest, no network

Downloads are untrusted data: this script only reads them (hashing, unzipping into the cache, decoding images with
Pillow). It never executes anything it downloads. Run it with `python -I` so nothing in the current directory can
shadow a standard module.

Deterministic: the same manifest and inputs give byte-identical outputs (fixed PNG settings, sorted archive members,
no timestamps). Resumable: verified blobs are kept by hash, partial downloads continue with HTTP Range, and a
converted asset is skipped when its stamp matches the inputs and converter version.

Converted texture layout, per asset and size (`<cache>/converted/<id>/<size>/`):
    albedo.png  sRGB colour (RGB, or RGBA when the set has an opacity map)
    normal.png  tangent-space normal, OpenGL convention (+Y up; DirectX sources get green flipped)
    orm.png     packed linear data: R = ambient occlusion, G = roughness, B = metalness (glTF ORM order)
    height.png  linear height/displacement, 8-bit grey (only when the set has one)
Missing maps get defaults in ORM: AO = 1.0, roughness and metalness from the Part material's preset (see
MATERIAL_DEFAULTS) or the asset's `[asset.texture]` overrides. `material.json` next to the size folders lists what was
produced. Sizes larger than the source are skipped (never upscaled). Pixel-art and other `convert = "none"` assets are
only unpacked.

Pillow is needed only for conversion (`pip install -r tools/assets/requirements.txt`, in a venv so `-I` still sees it).
"""
from __future__ import annotations

import argparse
import fnmatch
import hashlib
import io
import json
import os
import re
import shutil
import sys
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from dataclasses import dataclass, field
from pathlib import Path, PurePosixPath

CONVERTER_VERSION = 1
USER_AGENT = "MetallicAssetFetch/1.0 (+https://github.com/Metallic-Club/studio)"
DEFAULT_SIZES = (512, 1024, 2048)
CHUNK = 1 << 20

KINDS = ("texture", "texture_pack", "hdri", "model", "model_pack")
ROLES = ("primary", "alternate", "stylized", "pixel", "extra")
# Licences we accept. Anything else (BY-SA, NC, ND, GPL, "personal use", unclear) is rejected by validation.
ALLOWED_LICENCES = {"CC0-1.0": False, "CC-BY-4.0": True, "CC-BY-3.0": True}  # value = attribution required
TEXTURE_MAPS = ("albedo", "normal_gl", "normal_dx", "roughness", "ao", "metalness", "height", "opacity")
FILE_MAPS = TEXTURE_MAPS + ("archive", "hdri", "model", "include", "other")
# Extensions we unpack from archives. Everything else (scripts, executables, unknown) is skipped and reported.
SAFE_EXTRACT_EXT = {
    ".png", ".jpg", ".jpeg", ".tga", ".bmp", ".gif", ".webp", ".tif", ".tiff", ".exr", ".hdr",
    ".gltf", ".glb", ".bin", ".obj", ".mtl", ".fbx", ".txt", ".md", ".pdf", ".json", ".mtlx", ".usda", ".usdc",
}
ZIP_MAX_MEMBERS = 5000
ZIP_MAX_TOTAL = 4 << 30
ZIP_MAX_RATIO = 250

# Default roughness/metalness per Part material, used when a texture set has no map for it.
# Values follow the renderer's planned material table (metallic_render/src/materials.rs on studio/3-renderer).
MATERIAL_DEFAULTS = {
    "Plastic": (0.55, 0.0), "SmoothPlastic": (0.28, 0.0), "Metal": (0.34, 1.0), "Wood": (0.72, 0.0),
    "Concrete": (0.92, 0.0), "Glass": (0.06, 0.0), "Grass": (0.95, 0.0), "Brick": (0.85, 0.0),
    "Ice": (0.12, 0.0), "Sand": (0.98, 0.0),
}

# Map-name keywords found in archive member names (last matching token wins, scanning from the end).
_ROLE_TOKENS = {
    "albedo": {"color", "colour", "col", "diff", "diffuse", "albedo", "basecolor", "basecolour"},
    "normal_gl": {"normalgl", "norgl", "normal", "nor", "nrm", "norm", "normals"},
    "normal_dx": {"normaldx", "nordx"},
    "roughness": {"roughness", "rough", "rgh"},
    "ao": {"ambientocclusion", "ao", "occlusion"},
    "metalness": {"metalness", "metallic", "metal", "mtl"},
    "height": {"displacement", "disp", "height", "bump"},
    "opacity": {"opacity", "alpha"},
}
_TOKEN_ROLE = {tok: role for role, toks in _ROLE_TOKENS.items() for tok in toks}
_IMAGE_EXT = {".png", ".jpg", ".jpeg", ".tga", ".bmp", ".tif", ".tiff", ".webp"}


class FetchError(Exception):
    """A manifest, download, verification or conversion problem."""


# --------------------------------------------------------------------------------------------- manifest


@dataclass
class AssetFile:
    url: str
    sha256: str
    size: int
    map: str
    path: str


@dataclass
class Asset:
    id: str
    kind: str
    data: dict
    files: list[AssetFile] = field(default_factory=list)

    @property
    def texture(self) -> dict:
        return self.data.get("texture", {}) or {}


@dataclass
class Manifest:
    sources: dict
    assets: list[Asset]


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def load_manifest(directory: Path) -> Manifest:
    """Loads `sources.toml` and every other `*.toml` in `directory` (sorted), then validates."""
    directory = Path(directory)
    src_path = directory / "sources.toml"
    if not src_path.is_file():
        raise FetchError(f"missing {src_path}")
    with open(src_path, "rb") as f:
        sources = tomllib.load(f).get("source", {})
    assets: list[Asset] = []
    for path in sorted(directory.glob("*.toml")):
        if path.name == "sources.toml":
            continue
        with open(path, "rb") as f:
            doc = tomllib.load(f)
        for raw in doc.get("asset", []):
            files = [AssetFile(url=x.get("url", ""), sha256=x.get("sha256", ""), size=int(x.get("size", -1)),
                               map=x.get("map", ""), path=x.get("path", "")) for x in raw.get("files", [])]
            assets.append(Asset(id=raw.get("id", ""), kind=raw.get("kind", ""), data=raw, files=files))
    manifest = Manifest(sources=sources, assets=assets)
    errors = validate(manifest)
    if errors:
        raise FetchError("manifest is invalid:\n  " + "\n  ".join(errors))
    return manifest


_ID_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
_SHA_RE = re.compile(r"^[0-9a-f]{64}$")
REQUIRED = ("id", "kind", "name", "source", "source_id", "page_url", "download_url", "licence", "licence_proof_url",
            "licence_evidence", "licence_checked", "author", "attribution", "format", "resolution", "size", "tags",
            "look")


def _safe_rel_path(p: str) -> bool:
    if not p or "\\" in p or ":" in p:
        return False
    pp = PurePosixPath(p)
    return not pp.is_absolute() and ".." not in pp.parts and "" not in pp.parts and "." not in pp.parts


def validate(manifest: Manifest) -> list[str]:
    """Returns every problem found; empty means valid."""
    errs: list[str] = []
    seen: set[str] = set()
    for name, src in manifest.sources.items():
        for key in ("name", "homepage", "licence_proof_url", "hosts", "bundling"):
            if key not in src:
                errs.append(f"source {name}: missing {key}")
    for a in manifest.assets:
        where = f"asset {a.id or '?'}"
        for key in REQUIRED:
            if key not in a.data:
                errs.append(f"{where}: missing {key}")
        if not _ID_RE.match(a.id):
            errs.append(f"{where}: id must be lowercase words joined by '-'")
        if a.id in seen:
            errs.append(f"{where}: duplicate id")
        seen.add(a.id)
        if a.kind not in KINDS:
            errs.append(f"{where}: kind {a.kind!r} not in {KINDS}")
        lic = a.data.get("licence")
        if lic not in ALLOWED_LICENCES:
            errs.append(f"{where}: licence {lic!r} is not allowed (allowed: {sorted(ALLOWED_LICENCES)})")
        elif ALLOWED_LICENCES[lic] and not str(a.data.get("attribution", "")).strip():
            errs.append(f"{where}: {lic} needs an attribution line")
        src = manifest.sources.get(a.data.get("source", ""))
        if src is None:
            errs.append(f"{where}: unknown source {a.data.get('source')!r}")
        if a.kind == "texture":
            mat = a.data.get("material")
            if mat not in MATERIAL_DEFAULTS:
                errs.append(f"{where}: texture needs material = one of {sorted(MATERIAL_DEFAULTS)}")
            if a.data.get("role") not in ROLES:
                errs.append(f"{where}: role must be one of {ROLES}")
        if a.kind in ("model", "model_pack") and not a.data.get("category"):
            errs.append(f"{where}: models need a category")
        if not a.files:
            errs.append(f"{where}: no files")
        paths: set[str] = set()
        total = 0
        for i, f in enumerate(a.files):
            fw = f"{where} file {i}"
            u = urllib.parse.urlparse(f.url)
            if u.scheme != "https":
                errs.append(f"{fw}: url must be https")
            elif src is not None and u.hostname not in src.get("hosts", []):
                errs.append(f"{fw}: host {u.hostname} not listed for source {a.data.get('source')}")
            if not _SHA_RE.match(f.sha256):
                errs.append(f"{fw}: sha256 must be 64 lowercase hex chars")
            if f.size <= 0:
                errs.append(f"{fw}: size must be positive")
            total += max(f.size, 0)
            if f.map not in FILE_MAPS:
                errs.append(f"{fw}: map {f.map!r} not in {FILE_MAPS}")
            if not _safe_rel_path(f.path):
                errs.append(f"{fw}: path {f.path!r} must be a relative path without '..'")
            if f.path in paths:
                errs.append(f"{fw}: duplicate path {f.path}")
            paths.add(f.path)
        if a.files and a.data.get("size") != total:
            errs.append(f"{where}: size {a.data.get('size')} != sum of file sizes {total}")
    return errs


def select(manifest: Manifest, only: list[str] | None, kinds: list[str] | None) -> list[Asset]:
    out = []
    for a in manifest.assets:
        if kinds and a.kind not in kinds:
            continue
        if only and not any(fnmatch.fnmatchcase(a.id, pat) for pat in only):
            continue
        out.append(a)
    return out


# --------------------------------------------------------------------------------------------- TOML writer


def _toml_key(k: str) -> str:
    return k if re.fullmatch(r"[A-Za-z0-9_-]+", k) else json.dumps(k, ensure_ascii=False)


def _toml_value(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        return repr(v)
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    if isinstance(v, (list, tuple)):
        return "[" + ", ".join(_toml_value(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{ " + ", ".join(f"{_toml_key(k)} = {_toml_value(x)}" for k, x in v.items()) + " }"
    raise TypeError(f"cannot write {type(v).__name__} to TOML")


def dump_assets_toml(assets: list[dict], header: str = "") -> str:
    """Writes `[[asset]]` tables (scalars first, then `[asset.<table>]`, then `[[asset.files]]`)."""
    lines: list[str] = []
    if header:
        lines += [f"# {h}" if h else "#" for h in header.splitlines()] + [""]
    for a in assets:
        lines.append("[[asset]]")
        tables, arrays = {}, {}
        for k, v in a.items():
            if isinstance(v, dict):
                tables[k] = v
            elif isinstance(v, list) and v and isinstance(v[0], dict):
                arrays[k] = v
            else:
                lines.append(f"{k} = {_toml_value(v)}")
        for k, t in tables.items():
            lines.append(f"[asset.{k}]")
            lines += [f"{kk} = {_toml_value(vv)}" for kk, vv in t.items()]
        for k, arr in arrays.items():
            for item in arr:
                lines.append(f"[[asset.{k}]]")
                lines += [f"{kk} = {_toml_value(vv)}" for kk, vv in item.items()]
        lines.append("")
    return "\n".join(lines)


# --------------------------------------------------------------------------------------------- download


class Throttle:
    """Keeps a minimum interval between requests to the same host (robots.txt crawl-delay)."""

    def __init__(self, intervals: dict[str, float]):
        self.intervals = intervals
        self.last: dict[str, float] = {}

    def wait(self, host: str) -> None:
        gap = self.intervals.get(host, 0.0)
        prev = self.last.get(host)
        if prev is not None and gap > 0:
            delay = prev + gap - time.monotonic()
            if delay > 0:
                time.sleep(delay)
        self.last[host] = time.monotonic()


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(CHUNK), b""):
            h.update(chunk)
    return h.hexdigest()


def blob_path(cache: Path, sha: str) -> Path:
    return cache / "blobs" / sha[:2] / sha


def download(f: AssetFile, cache: Path, throttle: Throttle, opener=None, log=print) -> Path:
    """Returns the verified blob for `f`, downloading (and resuming) only when needed."""
    final = blob_path(cache, f.sha256)
    if final.is_file():
        if final.stat().st_size == f.size and sha256_file(final) == f.sha256:
            return final
        final.unlink()
    final.parent.mkdir(parents=True, exist_ok=True)
    part = final.with_name(final.name + ".part")
    have = part.stat().st_size if part.is_file() else 0
    if have > f.size:
        part.unlink()
        have = 0
    opener = opener or urllib.request.build_opener()
    host = urllib.parse.urlparse(f.url).hostname or ""
    for attempt in range(4):
        if have < f.size:
            throttle.wait(host)
            headers = {"User-Agent": USER_AGENT}
            if have:
                headers["Range"] = f"bytes={have}-"
            try:
                with opener.open(urllib.request.Request(f.url, headers=headers), timeout=120) as resp:
                    status = getattr(resp, "status", 200)
                    mode = "ab" if (have and status == 206) else "wb"
                    if mode == "wb":
                        have = 0
                    with open(part, mode) as out:
                        while True:
                            chunk = resp.read(CHUNK)
                            if not chunk:
                                break
                            have += len(chunk)
                            if have > f.size:
                                raise FetchError(f"{f.url}: larger than the manifest size {f.size}")
                            out.write(chunk)
            except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
                log(f"  retry {attempt + 1}: {f.url}: {e}")
                time.sleep(2 * (attempt + 1))
                have = part.stat().st_size if part.is_file() else 0
                continue
        if have == f.size:
            got = sha256_file(part)
            if got == f.sha256:
                os.replace(part, final)
                return final
            part.unlink()
            raise FetchError(f"{f.url}: SHA-256 mismatch (manifest {f.sha256}, got {got}); upstream file changed?")
        if have < f.size:
            continue
    raise FetchError(f"{f.url}: download failed (have {have} of {f.size} bytes)")


# --------------------------------------------------------------------------------------------- unpack


def safe_extract_zip(blob: Path, dest: Path, log=print) -> list[str]:
    """Extracts allowed file types from a zip into `dest`. Returns the extracted relative paths, sorted.

    Rejects absolute paths, `..`, drive letters, symlinks and zip bombs; skips (and reports) other file types.
    """
    out: list[str] = []
    with zipfile.ZipFile(blob) as z:
        infos = sorted(z.infolist(), key=lambda i: i.filename)
        if len(infos) > ZIP_MAX_MEMBERS:
            raise FetchError(f"{blob.name}: too many members ({len(infos)})")
        total = sum(i.file_size for i in infos)
        packed = sum(max(i.compress_size, 1) for i in infos)
        if total > ZIP_MAX_TOTAL or total / packed > ZIP_MAX_RATIO:
            raise FetchError(f"{blob.name}: looks like a zip bomb ({total} bytes unpacked)")
        for info in infos:
            name = info.filename
            if info.is_dir():
                continue
            if (info.external_attr >> 16) & 0o170000 == 0o120000:
                log(f"  skip symlink {name}")
                continue
            if not _safe_rel_path(name.rstrip("/")):
                raise FetchError(f"{blob.name}: unsafe member path {name!r}")
            if PurePosixPath(name).suffix.lower() not in SAFE_EXTRACT_EXT:
                log(f"  skip {name} (file type not on the allow list)")
                continue
            target = dest.joinpath(*PurePosixPath(name).parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with z.open(info) as src, open(target, "wb") as dst:
                shutil.copyfileobj(src, dst, CHUNK)
            out.append(name)
    return out


def materialize(asset: Asset, cache: Path, blobs: dict[str, Path], log=print) -> Path:
    """Lays out the asset's files under `<cache>/assets/<id>/` (archives unpacked). Returns that folder."""
    root = cache / "assets" / asset.id
    stamp = root / ".stamp.json"
    want = {"version": CONVERTER_VERSION, "files": sorted(f.sha256 + " " + f.path for f in asset.files)}
    if stamp.is_file() and json.loads(stamp.read_text("utf-8")) == want:
        return root
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    for f in asset.files:
        target = root.joinpath(*PurePosixPath(f.path).parts)
        target.parent.mkdir(parents=True, exist_ok=True)
        if f.map == "archive":
            safe_extract_zip(blobs[f.sha256], target.with_suffix(""), log=log)
        else:
            shutil.copyfile(blobs[f.sha256], target)
    stamp.write_text(json.dumps(want, indent=1, sort_keys=True), "utf-8")
    return root


def role_of(name: str) -> str | None:
    """Guesses a texture map role from a file name, e.g. `Bricks060_2K-JPG_NormalGL.jpg` -> `normal_gl`."""
    p = PurePosixPath(name)
    if p.suffix.lower() not in _IMAGE_EXT:
        return None
    tokens = [t for t in re.split(r"[_\-. ]+", p.stem.lower()) if t]
    for i in range(len(tokens) - 1, -1, -1):
        tok = tokens[i]
        if tok in ("gl", "dx") and i > 0 and tokens[i - 1] in ("nor", "normal"):
            return "normal_gl" if tok == "gl" else "normal_dx"
        if tok in _TOKEN_ROLE:
            return _TOKEN_ROLE[tok]
    return None


def texture_inputs(asset: Asset, root: Path) -> dict[str, Path]:
    """Map role -> source image for a texture asset (explicit files, explicit `members`, or guessed from names)."""
    found: dict[str, Path] = {}
    for f in asset.files:
        if f.map in TEXTURE_MAPS:
            found[f.map] = root.joinpath(*PurePosixPath(f.path).parts)
    members = asset.texture.get("members", {})
    archives = [root.joinpath(*PurePosixPath(f.path).with_suffix("").parts) for f in asset.files if f.map == "archive"]
    for role, rel in sorted(members.items()):
        for base in archives or [root]:
            cand = base.joinpath(*PurePosixPath(rel).parts)
            if cand.is_file():
                found[role] = cand
    if not members:
        for base in archives:
            for p in sorted(base.rglob("*")):
                r = role_of(p.name)
                if r and r not in found:
                    found[r] = p
    if "normal_gl" in found and "normal_dx" in found:
        del found["normal_dx"]
    return found


# --------------------------------------------------------------------------------------------- convert


def _pil():
    try:
        from PIL import Image  # noqa: PLC0415
    except ImportError as e:  # pragma: no cover - environment dependent
        raise FetchError("texture conversion needs Pillow: pip install -r tools/assets/requirements.txt "
                         "(or pass --no-convert)") from e
    return Image


def _to_l(im):
    """Any single-channel or colour image -> 8-bit 'L' (16-bit and float data are scaled, not clipped)."""
    im.load()
    if im.mode in ("I;16", "I;16B", "I;16L"):
        im = im.convert("I")
    if im.mode == "I":
        _lo, hi = im.getextrema()
        scale = 255.0 / 65535.0 if hi > 255 else 1.0
        return im.point(lambda v: v * scale).convert("L")
    if im.mode == "F":
        return im.point(lambda v: v * 255.0).convert("L")
    if im.mode == "LA":
        return im.getchannel("L")
    if im.mode != "L":
        return im.convert("RGB").getchannel("R")
    return im.copy()


def _resize(im, size: int, nearest: bool):
    Image = _pil()
    w, h = im.size
    if w == size and h <= size:
        return im
    nh = max(1, round(size * h / w))
    return im.resize((size, nh), Image.NEAREST if nearest else Image.LANCZOS)


def _save_png(im, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    buf = io.BytesIO()
    im.save(buf, format="PNG", compress_level=6, optimize=False)
    path.write_bytes(buf.getvalue())


def convert_texture(asset: Asset, root: Path, cache: Path, sizes: tuple[int, ...], log=print) -> Path | None:
    """Converts one texture set. Returns the output folder, or None when the asset is not convertible."""
    tex = asset.texture
    if tex.get("convert", "pbr") == "none":
        return None
    out = cache / "converted" / asset.id
    inputs = texture_inputs(asset, root)
    if "albedo" not in inputs:
        raise FetchError(f"{asset.id}: no albedo/colour map found (set [asset.texture].members)")
    stamp_want = {
        "version": CONVERTER_VERSION,
        "inputs": {k: sha256_file(v) for k, v in sorted(inputs.items())},
        "sizes": list(sizes),
        "texture": tex,
        "material": asset.data.get("material"),
    }
    stamp = out / ".stamp.json"
    if stamp.is_file() and json.loads(stamp.read_text("utf-8")) == stamp_want:
        return out
    if out.exists():
        shutil.rmtree(out)
    Image = _pil()
    nearest = bool(tex.get("nearest", False))
    rough_d, metal_d = MATERIAL_DEFAULTS.get(asset.data.get("material", ""), (0.8, 0.0))
    rough_d = float(tex.get("roughness", rough_d))
    metal_d = float(tex.get("metalness", metal_d))

    with Image.open(inputs["albedo"]) as im:
        albedo = im.convert("RGB")
    src_w = albedo.size[0]
    opacity = None
    if "opacity" in inputs:
        with Image.open(inputs["opacity"]) as im:
            opacity = _to_l(im)
    normal = None
    nkey = "normal_gl" if "normal_gl" in inputs else ("normal_dx" if "normal_dx" in inputs else None)
    if nkey:
        with Image.open(inputs[nkey]) as im:
            normal = im.convert("RGB")
        dx = nkey == "normal_dx" or tex.get("normal") == "dx"
        if dx:
            r, g, b = normal.split()
            normal = Image.merge("RGB", (r, g.point(lambda v: 255 - v), b))
    singles = {}
    for key in ("ao", "roughness", "metalness", "height"):
        if key in inputs:
            with Image.open(inputs[key]) as im:
                singles[key] = _to_l(im)
    if tex.get("roughness_from") == "gloss" and "roughness" in singles:
        singles["roughness"] = singles["roughness"].point(lambda v: 255 - v)

    eff = [s for s in sorted(sizes) if s <= src_w] or [src_w]  # never upscale; tiny sources keep native size
    for size in sorted(sizes):
        if size > src_w:
            log(f"  {asset.id}: skip {size} (source is {src_w} px wide)")
    produced = []
    for size in eff:
        d = out / str(size)
        a = _resize(albedo, size, nearest)
        if opacity is not None:
            a = a.convert("RGBA")
            a.putalpha(_resize(opacity, size, nearest).resize(a.size))
        _save_png(a, d / "albedo.png")
        tsize = a.size
        if normal is not None:
            _save_png(_resize(normal, size, nearest).resize(tsize), d / "normal.png")
        chans = []
        for key, default in (("ao", 1.0), ("roughness", rough_d), ("metalness", metal_d)):
            if key in singles:
                chans.append(_resize(singles[key], size, nearest).resize(tsize))
            else:
                chans.append(Image.new("L", tsize, round(default * 255)))
        _save_png(Image.merge("RGB", chans), d / "orm.png")
        if "height" in singles:
            _save_png(_resize(singles["height"], size, nearest).resize(tsize), d / "height.png")
        produced.append(size)
    info = {
        "id": asset.id,
        "material": asset.data.get("material"),
        "look": asset.data.get("look"),
        "role": asset.data.get("role"),
        "sizes": produced,
        "maps": {
            "albedo": "albedo.png (sRGB" + (", alpha = opacity)" if opacity is not None else ")"),
            "normal": "normal.png (linear, OpenGL +Y)" if normal is not None else None,
            "orm": "orm.png (linear: R=AO, G=roughness, B=metalness)",
            "height": "height.png (linear)" if "height" in singles else None,
        },
        "orm_defaults": {"ao": None if "ao" in singles else 1.0,
                         "roughness": None if "roughness" in singles else rough_d,
                         "metalness": None if "metalness" in singles else metal_d},
        "tile_size_m": asset.data.get("tile_size_m"),
        "tint": tex.get("tint", "multiply"),
        "filter": "nearest" if nearest else "linear",
    }
    (out / "material.json").write_text(json.dumps(info, indent=1, sort_keys=True), "utf-8")
    stamp.write_text(json.dumps(stamp_want, indent=1, sort_keys=True), "utf-8")
    return out


# --------------------------------------------------------------------------------------------- licences


def licences_markdown(manifest: Manifest) -> str:
    """Deterministic attribution file for every asset in the manifest."""
    order = {k: i for i, k in enumerate(KINDS)}
    lines = [
        "# Third-party assets",
        "",
        "Generated by `tools/assets/fetch.py` from `assets/manifest/`. Do not edit by hand.",
        "",
        "Every asset below is CC0 1.0 (public domain dedication) unless its row says otherwise. CC0 needs no",
        "attribution; we credit the authors anyway. Licence proof for each source is linked under Sources.",
        "",
        "## Sources",
        "",
        "| Source | Licence proof | Bundling and CDN terms |",
        "|---|---|---|",
    ]
    for key in sorted(manifest.sources):
        s = manifest.sources[key]
        lines.append(f"| [{s['name']}]({s['homepage']}) | {s['licence_proof_url']} | {s['bundling']} |")
    for kind in sorted({a.kind for a in manifest.assets}, key=lambda k: order.get(k, 99)):
        lines += ["", f"## {kind.replace('_', ' ').title()}s", "", "| Asset | Author | Source | Licence |", "|---|---|---|---|"]
        for a in sorted((x for x in manifest.assets if x.kind == kind), key=lambda x: x.id):
            d = a.data
            src = manifest.sources.get(d["source"], {}).get("name", d["source"])
            lines.append(f"| [{d['name']}]({d['page_url']}) (`{a.id}`) | {d['author']} | {src} | {d['licence']} |")
    attrib = [a for a in manifest.assets if ALLOWED_LICENCES.get(a.data.get("licence"), False)]
    if attrib:
        lines += ["", "## Required attributions", ""]
        lines += [f"- {a.data['attribution']}" for a in sorted(attrib, key=lambda x: x.id)]
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------------------------- main


def default_cache() -> Path:
    env = os.environ.get("METALLIC_ASSET_CACHE")
    if env:
        return Path(env)
    base = os.environ.get("LOCALAPPDATA") or os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "Metallic" / "asset-cache"


def run(args: argparse.Namespace, log=print) -> int:
    manifest = load_manifest(Path(args.manifest))
    if args.licenses_out:
        Path(args.licenses_out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.licenses_out).write_text(licences_markdown(manifest), "utf-8", newline="\n")
    if args.check or args.licenses_only:
        log(f"manifest ok: {len(manifest.assets)} assets")
        return 0
    cache = Path(args.cache)
    cache.mkdir(parents=True, exist_ok=True)
    throttle = Throttle({h: float(s.get("min_interval_s", 0)) for s in manifest.sources.values() for h in s["hosts"]})
    chosen = select(manifest, args.only, args.kind)
    sizes = tuple(sorted({int(s) for s in args.sizes.split(",") if s.strip()}))
    failures = 0
    for n, asset in enumerate(chosen, 1):
        log(f"[{n}/{len(chosen)}] {asset.id}")
        try:
            blobs = {f.sha256: download(f, cache, throttle, log=log) for f in asset.files}
            if args.download_only:
                continue
            root = materialize(asset, cache, blobs, log=log)
            if asset.kind == "texture" and not args.no_convert:
                convert_texture(asset, root, cache, sizes, log=log)
        except (FetchError, OSError, zipfile.BadZipFile) as e:
            failures += 1
            log(f"  FAILED: {e}")
    log(f"done: {len(chosen) - failures} ok, {failures} failed; cache {cache}")
    return 1 if failures else 0


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    root = repo_root()
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--manifest", default=str(root / "assets" / "manifest"), help="manifest folder")
    p.add_argument("--cache", default=str(default_cache()),
                   help="cache folder (default %%LOCALAPPDATA%%/Metallic/asset-cache or $METALLIC_ASSET_CACHE)")
    p.add_argument("--only", nargs="*", help="asset id glob(s), e.g. 'tex-brick-*'")
    p.add_argument("--kind", nargs="*", choices=KINDS, help="only these kinds")
    p.add_argument("--sizes", default=",".join(map(str, DEFAULT_SIZES)), help="texture sizes, default 512,1024,2048")
    p.add_argument("--no-convert", action="store_true", help="download and unpack only")
    p.add_argument("--download-only", action="store_true", help="download and verify only")
    p.add_argument("--check", action="store_true", help="validate the manifest and exit (no network)")
    p.add_argument("--licenses-only", action="store_true", help="write the licence file and exit")
    p.add_argument("--licenses-out", default=str(root / "LICENSES" / "ASSETS.md"),
                   help="attribution file to write ('' to skip)")
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    try:
        return run(parse_args(argv))
    except FetchError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
