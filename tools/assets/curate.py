#!/usr/bin/env python3
"""Add assets to the Metallic asset manifest with licence proof and SHA-256, or audit existing entries.

    python -I tools/assets/curate.py add polyhaven red_brick_03 --kind texture --material Brick --role primary
    python -I tools/assets/curate.py add ambientcg Bricks060 --kind texture --material Brick --role alternate
    python -I tools/assets/curate.py add polypizza 699sFuLCN2 --kind model --category nature --look stylized
    python -I tools/assets/curate.py add opengameart hand-painted-grass --kind texture --material Grass \\
        --role stylized --file https://opengameart.org/sites/default/files/grass_48.png=albedo
    python -I tools/assets/curate.py audit            # re-check every licence page (no downloads)

`add` fetches the asset's own page or API record, refuses it unless the licence it sees there is CC0 (or CC-BY
with `--allow-by`), streams every file once to compute SHA-256 and size (nothing is saved, nothing is executed),
and writes the entry into `assets/manifest/<kind>s.toml` (replacing an entry with the same id).

Sources and how licences are checked:
- polyhaven:   api.polyhaven.com/info/<slug> (Poly Haven asks for the API instead of scraping pages) plus the
               site licence page, which must still say all assets are CC0.
- ambientcg:   the asset page ambientcg.com/a/<id> must say assets are released under CC0.
- polypizza:   the model page poly.pizza/m/<id> must carry "Licence":"CC0 1.0" (CC-BY only with --allow-by).
- opengameart: the art page's licence block must list CC0 (CC-BY only with --allow-by); files are given with --file.
Requests send a descriptive User-Agent and respect each host's robots.txt crawl-delay (see sources.toml).
"""
from __future__ import annotations

import argparse
import hashlib
import html
import importlib.util
import json
import re
import sys
import time
import tomllib
import urllib.parse
import urllib.request
from datetime import date
from pathlib import Path

_spec = importlib.util.spec_from_file_location("metallic_fetch", Path(__file__).resolve().with_name("fetch.py"))
fetch = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fetch)

UA = fetch.USER_AGENT
TODAY = date.today().isoformat()
MANIFEST_DIR = fetch.repo_root() / "assets" / "manifest"
KIND_FILE = {"texture": "textures.toml", "texture_pack": "textures.toml", "hdri": "hdris.toml",
             "model": "models.toml", "model_pack": "models.toml"}
HEADERS = {
    "textures.toml": "PBR texture sets. One primary + alternates per Part material, plus stylized and pixel sets.\n"
                     "Generated with tools/assets/curate.py; see assets/README.md.",
    "hdris.toml": "HDRI skies and interiors (equirectangular .hdr). 4k for the sky, 1k for previews/irradiance.\n"
                  "Generated with tools/assets/curate.py; see assets/README.md.",
    "models.toml": "3D models for templates (glTF/GLB). Generated with tools/assets/curate.py; see assets/README.md.",
}
_last: dict[str, float] = {}
_DELAY = {"opengameart.org": 10.0, "poly.pizza": 10.0, "ambientcg.com": 1.0}


class CurateError(Exception):
    """The asset cannot be added (licence not proven, file missing, ...)."""


def _wait(url: str) -> None:
    host = urllib.parse.urlparse(url).hostname or ""
    gap = _DELAY.get(host, 0.3)
    if host in _last:
        d = _last[host] + gap - time.monotonic()
        if d > 0:
            time.sleep(d)
    _last[host] = time.monotonic()


def _retry(fn, url: str, tries: int = 3):
    for attempt in range(tries):
        try:
            return fn()
        except (OSError, ValueError) as e:  # URLError, resets, timeouts
            if attempt == tries - 1 or (hasattr(e, "code") and 400 <= e.code < 500):
                raise
            print(f"  retry {url}: {e}", file=sys.stderr)
            time.sleep(5 * (attempt + 1))
    raise AssertionError("unreachable")


def get(url: str) -> bytes:
    def once() -> bytes:
        _wait(url)
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.read()
    return _retry(once, url)


def get_json(url: str):
    return json.loads(get(url))


def page_text(raw: bytes) -> str:
    t = raw.decode("utf-8", "replace")
    t = re.sub(r"<script.*?</script>|<style.*?</style>", " ", t, flags=re.S)
    return re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", " ", t)))


class Hasher:
    """Streams a URL once and returns (sha256, size). Results are memoised in a JSON file (optional)."""

    def __init__(self, memo: Path | None = None):
        self.memo_path = memo
        self.memo = json.loads(memo.read_text("utf-8")) if memo and memo.is_file() else {}

    def __call__(self, url: str) -> tuple[str, int]:
        if url in self.memo:
            sha, size = self.memo[url]
            return sha, size
        def once() -> tuple[str, int]:
            _wait(url)
            h, n = hashlib.sha256(), 0
            req = urllib.request.Request(url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=300) as r:
                while chunk := r.read(1 << 20):
                    h.update(chunk)
                    n += len(chunk)
            return h.hexdigest(), n
        self.memo[url] = _retry(once, url)
        if self.memo_path:
            self.memo_path.write_text(json.dumps(self.memo, indent=0, sort_keys=True), "utf-8")
        return tuple(self.memo[url])


def slugify(s: str) -> str:
    return re.sub(r"-+", "-", re.sub(r"[^a-z0-9]+", "-", s.lower())).strip("-")


def _file(hasher, url: str, map_: str, path: str) -> dict:
    sha, size = hasher(url)
    return {"url": url, "sha256": sha, "size": size, "map": map_, "path": path}


# ----------------------------------------------------------------------------------------------- sources

_PH_LICENCE_OK: list[bool] = []


def _ph_licence_page() -> str:
    if not _PH_LICENCE_OK:
        t = page_text(get("https://polyhaven.com/license"))
        if "Our assets are all licensed as CC0" not in t:
            raise CurateError("polyhaven.com/license no longer says all assets are CC0; check by hand")
        _PH_LICENCE_OK.append(True)
    return "polyhaven.com/license: 'Our assets are all licensed as CC0'"


PH_TEX_MAPS = (("Diffuse", "albedo"), ("nor_gl", "normal_gl"), ("Rough", "roughness"), ("AO", "ao"),
               ("Metal", "metalness"), ("Displacement", "height"))


def resolve_polyhaven(slug: str, kind: str, res: str, hasher) -> dict:
    info = get_json(f"https://api.polyhaven.com/info/{slug}")
    if "name" not in info:
        raise CurateError(f"polyhaven: no asset {slug}")
    want_type = {"hdri": 0, "texture": 1, "model": 2}[kind]
    if info.get("type") != want_type:
        raise CurateError(f"polyhaven: {slug} is type {info.get('type')}, not {kind}")
    evidence = f"api.polyhaven.com/info/{slug} (type {want_type}, '{info['name']}'); " + _ph_licence_page()
    files = get_json(f"https://api.polyhaven.com/files/{slug}")
    out: list[dict] = []
    if kind == "texture":
        for key, role in PH_TEX_MAPS:
            if key in files and res in files[key]:
                url = files[key][res]["jpg"]["url"]
                out.append(_file(hasher, url, role, url.rsplit("/", 1)[1]))
        fmt = "jpg"
    elif kind == "hdri":
        for r in ("1k", res):
            url = files["hdri"][r]["hdr"]["url"]
            out.append(_file(hasher, url, "hdri", url.rsplit("/", 1)[1]))
        fmt = "hdr"
    else:
        g = files["gltf"][res]["gltf"]
        out.append(_file(hasher, g["url"], "model", g["url"].rsplit("/", 1)[1]))
        for path, inc in sorted(g.get("include", {}).items()):
            out.append(_file(hasher, inc["url"], "include", path))
        fmt = "gltf"
    authors = info.get("authors", {})
    author = ", ".join(sorted(authors)) if authors else "Poly Haven"
    entry = {
        "name": info["name"], "source": "polyhaven", "source_id": slug,
        "page_url": f"https://polyhaven.com/a/{slug}", "licence": "CC0-1.0",
        "licence_proof_url": "https://polyhaven.com/license", "licence_evidence": evidence,
        "author": author, "attribution": f"{info['name']} by {author}, Poly Haven (CC0 1.0)",
        "format": fmt, "resolution": res, "source_tags": sorted(set(info.get("tags", []) + info.get("categories", []))),
        "_files": out,
    }
    dims = info.get("dimensions")
    if kind == "texture" and dims and len(dims) >= 2 and dims[0] > 0:
        entry["tile_size_m"] = round(dims[0] / 1000.0, 3)
    if kind == "model" and info.get("polycount"):
        entry["polycount"] = int(info["polycount"])
    return entry


def resolve_ambientcg(aid: str, kind: str, res: str, hasher) -> dict:
    d = get_json(f"https://ambientcg.com/api/v3/assets?id={aid}&include=title,tags,downloads,technique")
    if not d.get("assets"):
        raise CurateError(f"ambientcg: no asset {aid}")
    a = d["assets"][0]
    page = f"https://ambientcg.com/a/{a['id']}"
    t = page_text(get(page))
    m = re.search(r"All assets are released under the Creative Commons CC0 license", t)
    if not m:
        raise CurateError(f"ambientcg: {page} does not state CC0")
    dl = next((x for x in a.get("downloads", []) if x.get("attributes") == res), None)
    if dl is None:
        raise CurateError(f"ambientcg: {aid} has no {res} download")
    url = dl["url"]
    fname = f"{a['id']}_{res}.zip"
    entry = {
        "name": a.get("title", aid), "source": "ambientcg", "source_id": a["id"], "page_url": page,
        "licence": "CC0-1.0", "licence_proof_url": "https://docs.ambientcg.com/license/",
        "licence_evidence": f"{page}: 'All assets are released under the Creative Commons CC0 license'",
        "author": "Lennart Demes (ambientCG)",
        "attribution": f"{a.get('title', aid)} from ambientCG.com, licensed under CC0 1.0",
        "format": "zip (jpg)", "resolution": res, "source_tags": sorted(a.get("tags", [])),
        "_files": [_file(hasher, url, "archive", fname)],
    }
    return entry


def resolve_polypizza(pid: str, kind: str, allow_by: bool, hasher) -> dict:
    page = f"https://poly.pizza/m/{pid}"
    raw = get(page).decode("utf-8", "replace")
    m = re.search(r'"model":(\{"Title".*?"Animated":(?:true|false)\})', raw)
    if not m:
        raise CurateError(f"polypizza: no model data on {page}")
    model = json.loads(m.group(1))
    lic = model.get("Licence", "")
    if lic == "CC0 1.0":
        licence, proof = "CC0-1.0", "https://creativecommons.org/publicdomain/zero/1.0/"
    elif lic == "CC-BY 3.0" and allow_by:
        licence, proof = "CC-BY-3.0", "https://creativecommons.org/licenses/by/3.0/"
    else:
        raise CurateError(f"polypizza: {page} licence is {lic!r}; not accepted")
    url = f"https://static.poly.pizza/{model['ResourceID']}.glb"
    if url not in raw:
        raise CurateError(f"polypizza: download link {url} not on the page")
    author = model.get("Creator", {}).get("Username", "unknown")
    title = model["Title"]
    entry = {
        "name": title, "source": "polypizza", "source_id": pid, "page_url": page, "licence": licence,
        "licence_proof_url": proof, "licence_evidence": f"{page}: model data \"Licence\":\"{lic}\"",
        "author": author,
        "attribution": f"{title} by {author} on Poly Pizza ({lic})",
        "format": "glb", "resolution": "n/a", "source_tags": sorted(set(model.get("Tags", []) + [model.get("Category", "")]) - {""}),
        "polycount": int(model.get("Tris", 0) or 0), "animated": bool(model.get("Animated")),
        "_files": [_file(hasher, url, "model", slugify(title) + ".glb")],
    }
    return entry


def resolve_opengameart(slug: str, kind: str, files: list[str], allow_by: bool, hasher) -> dict:
    page = f"https://opengameart.org/content/{slug}"
    raw = get(page).decode("utf-8", "replace")
    lics = re.findall(r"class=['\"]license-name['\"]>([^<]*)", raw)
    if "CC0" in lics:
        licence, proof = "CC0-1.0", "https://creativecommons.org/publicdomain/zero/1.0/"
    elif allow_by and any(x in ("CC-BY 4.0", "CC-BY 3.0") for x in lics):
        licence = "CC-BY-4.0" if "CC-BY 4.0" in lics else "CC-BY-3.0"
        proof = "https://creativecommons.org/licenses/by/4.0/" if licence.endswith("4.0") else \
            "https://creativecommons.org/licenses/by/3.0/"
    else:
        raise CurateError(f"opengameart: {page} licences {lics}; not accepted")
    title = html.unescape(re.search(r"<title>(.*?)\s*\|", raw, re.S).group(1)).strip()
    sub = re.search(r"field-name-author-submitter.*?<a[^>]*>([^<]*)</a>", raw, re.S)
    author = html.unescape(sub.group(1)).strip() if sub else "unknown"
    out = []
    for spec in files:
        url, _, map_ = spec.rpartition("=")
        if not url:
            raise CurateError(f"--file needs URL=map, got {spec}")
        if url not in raw and urllib.parse.unquote(url) not in raw and url.replace(" ", "%20") not in raw:
            raise CurateError(f"opengameart: {url} is not linked from {page}")
        name = urllib.parse.unquote(url.rsplit("/", 1)[1]).replace(" ", "_")
        out.append(_file(hasher, url, map_, name))
    fmt = sorted({f["path"].rsplit(".", 1)[-1].lower() for f in out})
    return {
        "name": title, "source": "opengameart", "source_id": slug, "page_url": page, "licence": licence,
        "licence_proof_url": proof, "licence_evidence": f"{page}: licence block lists {', '.join(lics)}",
        "author": author, "attribution": f"{title} by {author}, OpenGameArt.org ({', '.join(lics)})",
        "format": "/".join(fmt), "resolution": "native", "source_tags": [], "_files": out,
    }


# ----------------------------------------------------------------------------------------------- entries


def build_entry(source: str, source_id: str, *, kind: str, hasher, res: str | None = None, id_: str | None = None,
                material: str | None = None, role: str | None = None, look: str = "realistic",
                category: str | None = None, tags: list[str] | None = None, files: list[str] | None = None,
                texture: dict | None = None, notes: str | None = None, name: str | None = None,
                allow_by: bool = False) -> dict:
    """Resolves one asset into a manifest entry (dict in manifest field order)."""
    if source == "polyhaven":
        r = resolve_polyhaven(source_id, kind, res or {"texture": "2k", "hdri": "4k", "model": "1k"}[kind], hasher)
    elif source == "ambientcg":
        r = resolve_ambientcg(source_id, kind, res or "2K-JPG", hasher)
    elif source == "polypizza":
        r = resolve_polypizza(source_id, kind, allow_by, hasher)
    elif source == "opengameart":
        r = resolve_opengameart(source_id, kind, files or [], allow_by, hasher)
    else:
        raise CurateError(f"unknown source {source}")
    fl = r.pop("_files")
    if name:
        r["name"] = name
    if not id_:
        prefix = {"texture": f"tex-{slugify(material or 'misc')}", "texture_pack": "texpack", "hdri": "hdri",
                  "model": f"mdl-{category or 'misc'}", "model_pack": f"mdlpack-{category or 'misc'}"}[kind]
        stem = r["source_id"] if source in ("polyhaven", "ambientcg") else r["name"]
        id_ = f"{prefix}-{slugify(stem)}"
    e: dict = {"id": id_, "kind": kind, "name": r["name"], "source": r["source"], "source_id": r["source_id"],
               "page_url": r["page_url"], "download_url": fl[0]["url"], "licence": r["licence"],
               "licence_proof_url": r["licence_proof_url"], "licence_evidence": r["licence_evidence"],
               "licence_checked": TODAY, "author": r["author"], "attribution": r["attribution"],
               "format": r["format"], "resolution": r["resolution"], "size": sum(f["size"] for f in fl)}
    if material:
        e["material"] = material
    if role:
        e["role"] = role
    if category:
        e["category"] = category
    e["look"] = look
    for k in ("tile_size_m", "polycount", "animated"):
        if k in r:
            e[k] = r[k]
    src_tags = [slugify(t) for t in r["source_tags"]]
    e["tags"] = sorted(set((tags or []) + [t for t in src_tags if t and not t.isdigit()]))
    if notes:
        e["notes"] = notes
    if texture:
        e["texture"] = texture
    e["files"] = fl
    return e


def write_entries(path: Path, entries: list[dict], header: str) -> None:
    path.write_text(fetch.dump_assets_toml(entries, header), "utf-8", newline="\n")


def read_entries(path: Path) -> list[dict]:
    if not path.is_file():
        return []
    with open(path, "rb") as f:
        return tomllib.load(f).get("asset", [])


def cmd_add(a: argparse.Namespace) -> int:
    hasher = Hasher()
    texture = {}
    for kv in a.texture or []:
        k, _, v = kv.partition("=")
        texture[k] = v if not re.fullmatch(r"[0-9.]+", v) else float(v)
    e = build_entry(a.source, a.source_id, kind=a.kind, hasher=hasher, res=a.res, id_=a.id, material=a.material,
                    role=a.role, look=a.look, category=a.category, tags=a.tags, files=a.file, texture=texture or None,
                    notes=a.notes, allow_by=a.allow_by)
    fname = KIND_FILE[a.kind]
    path = Path(a.manifest) / fname
    entries = [x for x in read_entries(path) if x.get("id") != e["id"]] + [e]
    write_entries(path, entries, HEADERS[fname])
    fetch.load_manifest(Path(a.manifest))  # validates; raises on a bad entry
    print(f"added {e['id']} ({e['size']} bytes, {len(e['files'])} files) to {path}")
    return 0


def cmd_audit(a: argparse.Namespace) -> int:
    """Re-checks every licence statement (pages/APIs) without downloading asset files."""
    m = fetch.load_manifest(Path(a.manifest))
    bad = 0
    for asset in fetch.select(m, a.only, None):
        d = asset.data
        try:
            if d["source"] == "polyhaven":
                info = get_json(f"https://api.polyhaven.com/info/{d['source_id']}")
                ok = "name" in info and bool(_ph_licence_page())
            elif d["source"] == "ambientcg":
                ok = "released under the Creative Commons CC0 license" in page_text(get(d["page_url"]))
            elif d["source"] == "polypizza":
                ok = '"Licence":"CC0 1.0"' in get(d["page_url"]).decode("utf-8", "replace") or d["licence"] != "CC0-1.0"
            elif d["source"] == "opengameart":
                ok = "CC0" in re.findall(r"class=['\"]license-name['\"]>([^<]*)", get(d["page_url"]).decode("utf-8", "replace"))
            else:
                ok = False
        except (OSError, CurateError, ValueError) as e:
            ok = False
            print(f"{asset.id}: {e}")
        print(f"{'ok  ' if ok else 'FAIL'} {asset.id}")
        bad += not ok
    return 1 if bad else 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--manifest", default=str(MANIFEST_DIR))
    sub = p.add_subparsers(dest="cmd", required=True)
    ad = sub.add_parser("add", help="resolve one asset and add it to the manifest")
    ad.add_argument("source", choices=["polyhaven", "ambientcg", "polypizza", "opengameart"])
    ad.add_argument("source_id")
    ad.add_argument("--kind", required=True, choices=fetch.KINDS)
    ad.add_argument("--id", help="manifest id (default derived from kind/material/category and name)")
    ad.add_argument("--res", help="polyhaven 2k/4k/1k, ambientcg 2K-JPG/1K-JPG")
    ad.add_argument("--material", help="Part material for textures (Plastic, Metal, ...)")
    ad.add_argument("--role", choices=fetch.ROLES)
    ad.add_argument("--look", default="realistic", help="look preset: realistic, stylized, toon, pixel, lowpoly")
    ad.add_argument("--category", help="models: nature, prop, building, character, animal, weapon, horror, platformer")
    ad.add_argument("--tags", nargs="*")
    ad.add_argument("--file", action="append", help="opengameart: URL=map (map: albedo, normal_gl, archive, ...)")
    ad.add_argument("--texture", action="append", help="[asset.texture] key=value, e.g. convert=none")
    ad.add_argument("--notes")
    ad.add_argument("--allow-by", action="store_true", help="accept CC-BY (records attribution)")
    au = sub.add_parser("audit", help="re-check licence statements of manifest entries")
    au.add_argument("--only", nargs="*")
    a = p.parse_args(argv)
    try:
        return cmd_add(a) if a.cmd == "add" else cmd_audit(a)
    except (CurateError, fetch.FetchError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
