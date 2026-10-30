#!/usr/bin/env python3
"""Contact sheets for the asset library: lit texture spheres, tone-mapped HDRIs and model renders.

    python -I tools/assets/previews.py textures --cache <cache> --out docs/agents/36-asset-library --cleanup
    python -I tools/assets/previews.py hdris    --cache <cache> --out docs/agents/36-asset-library --cleanup
    python -I tools/assets/previews.py models   --cache <cache> --out docs/agents/36-asset-library --cleanup \\
        --blender "C:/Program Files/Blender Foundation/Blender 5.2/blender.exe"

Every preview is rendered here from the CC0 files themselves (source sites' own renders are not reused: Poly Haven's
are not CC0). Each asset is fetched into the cache, previewed, and with --cleanup its raw files are deleted again, so
the whole library never has to fit on disk at once. Needs Pillow and numpy; models need Blender (background mode,
factory settings, auto-run of scripts disabled; only the glTF importer reads the files).
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import math
import shutil
import subprocess
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

_spec = importlib.util.spec_from_file_location("metallic_fetch", Path(__file__).resolve().with_name("fetch.py"))
fetch = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fetch)

TILE = 128
LABEL_H = 26
BG = (30, 31, 34)
FG = (215, 217, 222)
DIM = (140, 143, 150)


# ----------------------------------------------------------------------------------------------- textures


def _srgb_to_lin(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def _lin_to_srgb(c):
    c = np.clip(c, 0, 1)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1 / 2.4) - 0.055)


def _sample(img: np.ndarray, u: np.ndarray, v: np.ndarray) -> np.ndarray:
    h, w = img.shape[:2]
    x = (np.floor(u * w).astype(int)) % w
    y = (np.floor((1 - v) * h).astype(int)) % h
    return img[y, x]


def render_sphere(folder: Path, size: int = TILE, tiles: float = 2.0, base=(0.62, 0.64, 0.68)) -> Image.Image:
    """Renders a lit sphere from converted maps (albedo/normal/orm) with a key light and a sky fill."""
    n = size * 2
    ys, xs = np.mgrid[0:n, 0:n]
    x = (xs + 0.5) / n * 2 - 1
    y = 1 - (ys + 0.5) / n * 2
    r2 = x * x + y * y
    mask = r2 < 1
    z = np.sqrt(np.clip(1 - r2, 0, 1))
    N = np.stack([x, y, z], -1)
    phi = np.arctan2(x, z)
    u = (phi / math.pi * 0.5 + 0.5) * tiles
    v = (np.arcsin(np.clip(y, -1, 1)) / math.pi + 0.5) * tiles * 0.5
    T = np.stack([np.cos(phi), np.zeros_like(phi), -np.sin(phi)], -1)
    B = np.cross(N, T)

    alb_img = Image.open(folder / "albedo.png")
    alb = np.asarray(alb_img.convert("RGBA"), dtype=np.float32) / 255.0
    a = _sample(alb, u, v)
    albedo = _srgb_to_lin(a[..., :3])
    alpha = a[..., 3:4]
    if alb_img.mode == "RGBA":  # overlay sets (glass smudges): composite over a neutral base colour
        albedo = albedo * alpha + np.array(base) ** 2.2 * (1 - alpha)
    if (folder / "normal.png").is_file():
        nm = np.asarray(Image.open(folder / "normal.png").convert("RGB"), dtype=np.float32) / 255.0
        t = _sample(nm, u, v) * 2 - 1
        Np = T * t[..., :1] + B * t[..., 1:2] + N * t[..., 2:3]
        Np /= np.linalg.norm(Np, axis=-1, keepdims=True) + 1e-6
    else:
        Np = N
    orm = np.asarray(Image.open(folder / "orm.png").convert("RGB"), dtype=np.float32) / 255.0
    o = _sample(orm, u, v)
    ao, rough, metal = o[..., 0:1], np.clip(o[..., 1:2], 0.04, 1), o[..., 2:3]

    L = np.array([-0.55, 0.62, 0.56])
    L /= np.linalg.norm(L)
    V = np.array([0, 0, 1.0])
    H = (L + V) / np.linalg.norm(L + V)
    ndl = np.clip((Np * L).sum(-1, keepdims=True), 0, 1)
    ndv = np.clip((Np * V).sum(-1, keepdims=True), 1e-3, 1)
    ndh = np.clip((Np * H).sum(-1, keepdims=True), 0, 1)
    a2 = (rough * rough) ** 2
    D = a2 / (math.pi * (ndh * ndh * (a2 - 1) + 1) ** 2)
    k = (rough + 1) ** 2 / 8
    G = (ndl / (ndl * (1 - k) + k)) * (ndv / (ndv * (1 - k) + k))
    F0 = 0.04 * (1 - metal) + albedo * metal
    F = F0 + (1 - F0) * (1 - np.clip((H * V).sum(), 0, 1)) ** 5
    spec = D * G * F / (4 * ndl * ndv + 1e-4)
    sun = np.array([1.0, 0.96, 0.9]) * 3.0
    sky = 0.35 + 0.25 * np.clip(Np[..., 1:2], -1, 1)
    diffuse = albedo * (1 - metal) / math.pi
    env_spec = F0 * (1 - rough) * 0.6 + F0 * 0.15
    col = (diffuse * sun * ndl + spec * sun * ndl) + (albedo * (1 - metal) * sky * 0.55 + env_spec * sky) * ao
    col = col / (1 + col * 0.35)  # soft shoulder
    rgb = _lin_to_srgb(col)
    bg = np.array(BG) / 255.0
    edge = np.clip((1 - np.sqrt(r2)) * n * 0.5, 0, 1)[..., None]
    out = rgb * edge * mask[..., None] + bg * (1 - edge * mask[..., None])
    img = Image.fromarray((np.clip(out, 0, 1) * 255 + 0.5).astype(np.uint8))
    return img.resize((size, size), Image.LANCZOS)


# ----------------------------------------------------------------------------------------------- HDRIs


def read_hdr(path: Path) -> np.ndarray:
    """Decodes a Radiance .hdr (RGBE, new-style RLE or flat) into float32 RGB."""
    data = path.read_bytes()
    pos = 0
    header = []
    while True:
        end = data.index(b"\n", pos)
        line = data[pos:end]
        pos = end + 1
        if not line.strip():
            break
        header.append(line)
    if not header or not header[0].startswith(b"#?"):
        raise ValueError("not a Radiance HDR file")
    end = data.index(b"\n", pos)
    dims = data[pos:end].split()
    pos = end + 1
    if dims[0] != b"-Y" or dims[2] != b"+X":
        raise ValueError(f"unsupported orientation {dims}")
    h, w = int(dims[1]), int(dims[3])
    if not (0 < w <= 32768 and 0 < h <= 16384):
        raise ValueError("unreasonable size")
    out = np.zeros((h, w, 4), dtype=np.uint8)
    buf = memoryview(data)
    for row in range(h):
        if w >= 8 and buf[pos] == 2 and buf[pos + 1] == 2 and (buf[pos + 2] << 8 | buf[pos + 3]) == w:
            pos += 4
            for ch in range(4):
                col = 0
                line = bytearray(w)
                while col < w:
                    c = buf[pos]
                    pos += 1
                    if c > 128:
                        c -= 128
                        line[col:col + c] = bytes([buf[pos]]) * c
                        pos += 1
                    else:
                        line[col:col + c] = buf[pos:pos + c]
                        pos += c
                    col += c
                out[row, :, ch] = np.frombuffer(bytes(line[:w]), dtype=np.uint8)
        else:
            out[row] = np.frombuffer(bytes(buf[pos:pos + w * 4]), dtype=np.uint8).reshape(w, 4)
            pos += w * 4
    e = out[..., 3].astype(np.int32)
    scale = np.where(e > 0, np.ldexp(1.0, e - 136), 0.0).astype(np.float32)
    return out[..., :3].astype(np.float32) * scale[..., None]


def hdri_thumb(path: Path, width: int = 256, key_value: float = 0.18) -> Image.Image:
    """Tone-maps with a fixed middle grey; pass a low key_value for night skies so they read as night."""
    rgb = read_hdr(path)
    lum = (rgb * np.array([0.2126, 0.7152, 0.0722])).sum(-1)
    key = np.exp(np.mean(np.log(lum + 1e-4)))
    x = rgb * (key_value / key)
    x = np.clip((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0, 1)  # ACES fit
    img = Image.fromarray((_lin_to_srgb(x) * 255 + 0.5).astype(np.uint8))
    return img.resize((width, width // 2), Image.LANCZOS)


# ----------------------------------------------------------------------------------------------- sheets


def _font(size: int):
    for name in ("segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default()


def _fit(d, text, font, width):
    if d.textlength(text, font=font) <= width - 2:
        return text
    while text and d.textlength(text + "…", font=font) > width - 2:
        text = text[:-1]
    return text + "…"


def sheet(tiles: list[tuple[Image.Image, str, str]], cols: int, title: str) -> Image.Image:
    tw, th = tiles[0][0].size
    rows = math.ceil(len(tiles) / cols)
    pad, head = 6, 34
    im = Image.new("RGB", (cols * (tw + pad) + pad, head + rows * (th + LABEL_H + pad) + pad), BG)
    d = ImageDraw.Draw(im)
    d.text((pad + 2, 8), title, fill=FG, font=_font(16))
    f1, f2 = _font(11), _font(10)
    for i, (t, line1, line2) in enumerate(tiles):
        x = pad + (i % cols) * (tw + pad)
        y = head + (i // cols) * (th + LABEL_H + pad)
        im.paste(t, (x, y))
        d.text((x + 1, y + th + 1), _fit(d, line1, f1, tw), fill=FG, font=f1)
        d.text((x + 1, y + th + 13), _fit(d, line2, f2, tw), fill=DIM, font=f2)
    return im


def _cleanup(cache: Path, asset) -> None:
    shutil.rmtree(cache / "assets" / asset.id, ignore_errors=True)
    shutil.rmtree(cache / "converted" / asset.id, ignore_errors=True)
    for f in asset.files:
        fetch.blob_path(cache, f.sha256).unlink(missing_ok=True)


def _throttle(m):
    return fetch.Throttle({h: float(s.get("min_interval_s", 0)) for s in m.sources.values() for h in s["hosts"]})


def cmd_textures(a) -> None:
    m = fetch.load_manifest(Path(a.manifest))
    cache, th = Path(a.cache), _throttle(m)
    order = {"primary": 0, "alternate": 1, "extra": 2, "stylized": 3, "pixel": 4}
    mats = list(fetch.MATERIAL_DEFAULTS)
    assets = sorted((x for x in m.assets if x.kind == "texture"),
                    key=lambda x: (order[x.data["role"]] >= 3, order[x.data["role"]], mats.index(x.data["material"]), x.id))
    tiles = []
    for asset in assets:
        print("texture", asset.id, flush=True)
        blobs = {f.sha256: fetch.download(f, cache, th) for f in asset.files}
        root = fetch.materialize(asset, cache, blobs)
        out = fetch.convert_texture(asset, root, cache, (512,))
        info = json.loads((out / "material.json").read_text("utf-8"))
        tiles.append((render_sphere(out / str(info["sizes"][0])), asset.data["name"],
                      f"{asset.data['material']} · {asset.data['role']}"))
        if a.cleanup:
            _cleanup(cache, asset)
    sheet(tiles, a.cols, f"Metallic asset library: {len(tiles)} texture sets (rendered from the CC0 maps)").save(
        Path(a.out) / "textures.jpg", quality=82, optimize=True)


def cmd_hdris(a) -> None:
    m = fetch.load_manifest(Path(a.manifest))
    cache, th = Path(a.cache), fetch.Throttle({})
    tiles = []
    for asset in (x for x in m.assets if x.kind == "hdri"):
        f = min(asset.files, key=lambda f: f.size)  # the 1k file
        print("hdri", asset.id, flush=True)
        blob = fetch.download(f, cache, th)
        tags = asset.data["tags"]
        kv = 0.035 if "night" in tags else 0.12 if "sunset" in tags else 0.18
        shown = ", ".join(t for t in tags if t in ("day", "sunset", "overcast", "night", "interior", "fog", "studio"))
        tiles.append((hdri_thumb(blob, key_value=kv), asset.data["name"], shown))
        if a.cleanup:
            blob.unlink(missing_ok=True)
    sheet(tiles, 5, f"Metallic asset library: {len(tiles)} HDRIs (1k files, ACES tone-mapped; night and sunset "
                    "exposed darker)").save(Path(a.out) / "hdris.jpg", quality=82, optimize=True)


def cmd_models(a) -> None:
    m = fetch.load_manifest(Path(a.manifest))
    cache, th = Path(a.cache), _throttle(m)
    renders = cache / "renders"
    renders.mkdir(parents=True, exist_ok=True)
    cats = ["platformer", "nature", "prop", "building", "weapon", "horror", "character", "animal"]
    assets = sorted((x for x in m.assets if x.kind == "model"),
                    key=lambda x: (cats.index(x.data["category"]) if x.data["category"] in cats else 99, x.id))
    script = Path(__file__).resolve().with_name("blender_render.py")
    tiles = []
    for asset in assets:
        png = renders / f"{asset.id}.png"
        if not png.is_file() or png.stat().st_size == 0:
            print("model", asset.id, flush=True)
            blobs = {f.sha256: fetch.download(f, cache, th) for f in asset.files}
            root = fetch.materialize(asset, cache, blobs)
            main_file = next(f for f in asset.files if f.map == "model")
            job = Path(tempfile.mkdtemp()) / "job.json"
            job.write_text(json.dumps([{"model": str(root / main_file.path), "out": str(png)}]), "utf-8")
            subprocess.run([a.blender, "-b", "--factory-startup", "-noaudio", "--disable-autoexec",
                            "--python", str(script), "--", str(job)], check=False, timeout=600,
                           stdout=None if a.verbose else subprocess.DEVNULL)
            shutil.rmtree(job.parent, ignore_errors=True)
            if a.cleanup:
                _cleanup(cache, asset)
        try:
            rgba = Image.open(png).convert("RGBA")
            t = Image.alpha_composite(Image.new("RGBA", rgba.size, BG + (255,)), rgba).convert("RGB")
            t = t.resize((TILE, TILE), Image.LANCZOS)
        except (OSError, ValueError):
            print("  no render for", asset.id, flush=True)
            t = Image.new("RGB", (TILE, TILE), (80, 30, 30))
        poly = asset.data.get("polycount")
        tiles.append((t, asset.data["name"], f"{asset.data['category']} · {asset.data['look']}"
                      + (f" · {poly // 1000}k tris" if poly and poly >= 1000 else "")))
    sheet(tiles, a.cols, f"Metallic asset library: {len(tiles)} models (rendered in Blender from the CC0 files)").save(
        Path(a.out) / "models.jpg", quality=80, optimize=True)


def main() -> None:
    root = fetch.repo_root()
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("what", choices=["textures", "hdris", "models"])
    p.add_argument("--manifest", default=str(root / "assets" / "manifest"))
    p.add_argument("--cache", default=str(fetch.default_cache()))
    p.add_argument("--out", default=str(root / "docs" / "agents" / "36-asset-library"))
    p.add_argument("--cols", type=int, default=8)
    p.add_argument("--cleanup", action="store_true", help="delete each asset's raw files after previewing it")
    p.add_argument("--blender", default="blender")
    p.add_argument("--verbose", action="store_true")
    a = p.parse_args()
    Path(a.out).mkdir(parents=True, exist_ok=True)
    {"textures": cmd_textures, "hdris": cmd_hdris, "models": cmd_models}[a.what](a)


if __name__ == "__main__":
    main()
