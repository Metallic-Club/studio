#!/usr/bin/env python3
"""Tests for tools/assets/fetch.py. Run: python -I tools/assets/test_fetch.py (Pillow tests skip without Pillow)."""
from __future__ import annotations

import copy
import hashlib
import http.server
import importlib.util
import io
import json
import shutil
import tempfile
import threading
import tomllib
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("metallic_fetch", HERE / "fetch.py")
fetch = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fetch)

try:
    from PIL import Image
except ImportError:  # pragma: no cover
    Image = None

SHA = "a" * 64


def entry(**over) -> dict:
    e = {
        "id": "tex-brick-test", "kind": "texture", "name": "Test", "source": "src", "source_id": "t",
        "page_url": "https://example.org/t", "download_url": "https://dl.example.org/t.jpg", "licence": "CC0-1.0",
        "licence_proof_url": "https://example.org/license", "licence_evidence": "page says CC0",
        "licence_checked": "2026-10-30", "author": "Someone", "attribution": "Test by Someone (CC0)", "format": "jpg",
        "resolution": "2k", "size": 10, "tags": ["x"], "look": "realistic", "material": "Brick", "role": "primary",
        "files": [{"url": "https://dl.example.org/t.jpg", "sha256": SHA, "size": 10, "map": "albedo", "path": "t.jpg"}],
    }
    e.update(over)
    return e


SOURCES = ('[source.src]\nname = "Src"\nhomepage = "https://example.org"\nlicence_proof_url = "https://example.org/l"\n'
           'hosts = ["dl.example.org"]\nbundling = "CC0"\nmin_interval_s = 0.0\n')


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def write(self, *entries: dict) -> Path:
        d = self.tmp / "manifest"
        d.mkdir(exist_ok=True)
        (d / "sources.toml").write_text(SOURCES, "utf-8")
        (d / "textures.toml").write_text(fetch.dump_assets_toml(list(entries), "test"), "utf-8")
        return d

    def errors(self, *entries: dict) -> list[str]:
        try:
            fetch.load_manifest(self.write(*entries))
        except fetch.FetchError as e:
            return str(e).splitlines()[1:]
        return []

    def test_valid_entry_loads(self):
        m = fetch.load_manifest(self.write(entry()))
        self.assertEqual([a.id for a in m.assets], ["tex-brick-test"])
        self.assertEqual(m.assets[0].files[0].map, "albedo")

    def test_rejects_disallowed_licences(self):
        for lic in ("CC-BY-SA-4.0", "CC-BY-NC-4.0", "GPL-3.0", "personal-use", ""):
            errs = self.errors(entry(licence=lic))
            self.assertTrue(any("is not allowed" in e for e in errs), (lic, errs))

    def test_cc_by_needs_attribution(self):
        self.assertTrue(any("attribution" in e for e in self.errors(entry(licence="CC-BY-4.0", attribution=" "))))
        self.assertEqual(self.errors(entry(licence="CC-BY-4.0")), [])

    def test_rejects_unsafe_files(self):
        bad = [
            ({"url": "http://dl.example.org/t.jpg"}, "https"),
            ({"url": "https://evil.example.com/t.jpg"}, "not listed"),
            ({"sha256": "ABC"}, "sha256"),
            ({"path": "../t.jpg"}, "relative path"),
            ({"path": "C:/t.jpg"}, "relative path"),
            ({"map": "script"}, "map"),
        ]
        for over, needle in bad:
            f = dict(entry()["files"][0], **over)
            errs = self.errors(entry(files=[f]))
            self.assertTrue(any(needle in e for e in errs), (over, errs))

    def test_rejects_duplicates_size_mismatch_and_missing_fields(self):
        self.assertTrue(any("duplicate id" in e for e in self.errors(entry(), entry())))
        self.assertTrue(any("sum of file sizes" in e for e in self.errors(entry(size=11))))
        self.assertTrue(any("needs material" in e for e in self.errors(entry(material="Cheese"))))
        e = entry()
        del e["licence_evidence"]
        self.assertTrue(any("missing licence_evidence" in x for x in self.errors(e)))

    def test_toml_writer_round_trips(self):
        e = entry(name='Quote " and \\ backslash', texture={"members": {"albedo": "a b/c.png"}, "nearest": True,
                                                            "roughness": 0.5})
        doc = tomllib.loads(fetch.dump_assets_toml([e], "header\nline two"))
        self.assertEqual(doc["asset"][0], e)

    def test_licences_markdown_is_deterministic_and_credits_cc_by(self):
        d = self.write(entry(id="tex-brick-b"), entry(id="tex-brick-a", licence="CC-BY-4.0", attribution="A by B"))
        m = fetch.load_manifest(d)
        md = fetch.licences_markdown(m)
        self.assertEqual(md, fetch.licences_markdown(fetch.load_manifest(d)))
        self.assertLess(md.index("tex-brick-a"), md.index("tex-brick-b"))
        self.assertIn("- A by B", md)

    def test_select_filters(self):
        m = fetch.load_manifest(self.write(entry(id="tex-brick-a"), entry(id="tex-wood-b", material="Wood")))
        self.assertEqual([a.id for a in fetch.select(m, ["tex-wood-*"], None)], ["tex-wood-b"])
        self.assertEqual(fetch.select(m, None, ["hdri"]), [])


class RealManifestTests(unittest.TestCase):
    MANIFEST = HERE.parents[1] / "assets" / "manifest"

    def setUp(self):
        if not self.MANIFEST.is_dir():
            self.skipTest("no assets/manifest")
        self.m = fetch.load_manifest(self.MANIFEST)

    def test_every_part_material_has_a_primary_and_two_alternates(self):
        for mat in fetch.MATERIAL_DEFAULTS:
            sets = [a for a in self.m.assets if a.kind == "texture" and a.data.get("material") == mat]
            roles = [a.data["role"] for a in sets]
            self.assertEqual(roles.count("primary"), 1, mat)
            self.assertGreaterEqual(roles.count("alternate"), 2, mat)

    def test_counts_and_looks(self):
        kinds = [a.kind for a in self.m.assets]
        self.assertGreaterEqual(kinds.count("hdri"), 10)
        self.assertGreaterEqual(kinds.count("model"), 50)
        looks = {a.data["look"] for a in self.m.assets if a.kind.startswith("texture")}
        self.assertTrue({"stylized", "pixel"} <= looks, looks)

    def test_only_cc0_or_attributed_cc_by(self):
        for a in self.m.assets:
            self.assertIn(a.data["licence"], fetch.ALLOWED_LICENCES, a.id)

    def test_hdri_tags_cover_times_of_day(self):
        tags = {t for a in self.m.assets if a.kind == "hdri" for t in a.data["tags"]}
        self.assertTrue({"day", "sunset", "overcast", "night", "interior"} <= tags, tags)


class _Handler(http.server.BaseHTTPRequestHandler):
    payload = b""
    hits: list = []

    def log_message(self, *a):
        pass

    def do_GET(self):  # noqa: N802
        _Handler.hits.append(self.headers.get("Range"))
        data, status = self.payload, 200
        rng = self.headers.get("Range")
        if rng and rng.startswith("bytes="):
            start = int(rng[6:].split("-")[0])
            data, status = data[start:], 206
        self.send_response(status)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


class DownloadTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.url = f"http://127.0.0.1:{cls.server.server_port}/file.bin"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        self.cache = Path(tempfile.mkdtemp())
        _Handler.payload = bytes(range(256)) * 400
        _Handler.hits = []
        self.sha = hashlib.sha256(_Handler.payload).hexdigest()

    def tearDown(self):
        shutil.rmtree(self.cache)

    def f(self, **over):
        d = dict(url=self.url, sha256=self.sha, size=len(_Handler.payload), map="other", path="file.bin")
        d.update(over)
        return fetch.AssetFile(**d)

    def test_downloads_verifies_and_reuses(self):
        p = fetch.download(self.f(), self.cache, fetch.Throttle({}), log=lambda *_: None)
        self.assertEqual(p.read_bytes(), _Handler.payload)
        fetch.download(self.f(), self.cache, fetch.Throttle({}), log=lambda *_: None)
        self.assertEqual(len(_Handler.hits), 1, "second call must use the verified blob")

    def test_resumes_partial_download_with_range(self):
        part = fetch.blob_path(self.cache, self.sha)
        part.parent.mkdir(parents=True)
        part.with_name(part.name + ".part").write_bytes(_Handler.payload[:1000])
        p = fetch.download(self.f(), self.cache, fetch.Throttle({}), log=lambda *_: None)
        self.assertEqual(_Handler.hits, ["bytes=1000-"])
        self.assertEqual(p.read_bytes(), _Handler.payload)

    def test_hash_mismatch_fails_and_keeps_nothing(self):
        with self.assertRaises(fetch.FetchError):
            fetch.download(self.f(sha256="0" * 64), self.cache, fetch.Throttle({}), log=lambda *_: None)
        self.assertEqual([p for p in self.cache.rglob("*") if p.is_file()], [])

    def test_larger_than_manifest_size_aborts(self):
        with self.assertRaises(fetch.FetchError):
            fetch.download(self.f(size=100), self.cache, fetch.Throttle({}), log=lambda *_: None)


class UnpackTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def zip_with(self, members: dict[str, bytes]) -> Path:
        p = self.tmp / "a.zip"
        with zipfile.ZipFile(p, "w", zipfile.ZIP_DEFLATED) as z:
            for name, data in members.items():
                z.writestr(name, data)
        return p

    def test_extracts_only_allowed_types(self):
        z = self.zip_with({"tex/a_Color.jpg": b"x", "run.exe": b"MZ", "x.py": b"print()", "Demo.blend": b"B",
                           "readme.txt": b"hi"})
        out = fetch.safe_extract_zip(z, self.tmp / "out", log=lambda *_: None)
        self.assertEqual(out, ["readme.txt", "tex/a_Color.jpg"])
        self.assertFalse((self.tmp / "out" / "run.exe").exists())

    def test_rejects_path_traversal(self):
        for bad in ("../evil.png", "/abs.png", "a/../../b.png", "C:/x.png"):
            z = self.zip_with({bad: b"x"})
            with self.assertRaises(fetch.FetchError, msg=bad):
                fetch.safe_extract_zip(z, self.tmp / "out", log=lambda *_: None)

    def test_rejects_zip_bombs(self):
        z = self.zip_with({"big.png": b"\0" * (8 << 20)})
        with self.assertRaises(fetch.FetchError):
            fetch.safe_extract_zip(z, self.tmp / "out", log=lambda *_: None)

    def test_role_detection(self):
        cases = {
            "Bricks060_2K-JPG_Color.jpg": "albedo", "Bricks060_2K-JPG_NormalGL.jpg": "normal_gl",
            "Bricks060_2K-JPG_NormalDX.jpg": "normal_dx", "Bricks060_2K-JPG_Roughness.jpg": "roughness",
            "Bricks060_2K-JPG_AmbientOcclusion.jpg": "ao", "Metal009_2K-JPG_Metalness.jpg": "metalness",
            "Bricks060_2K-JPG_Displacement.jpg": "height", "rusty_metal_02_diff_2k.jpg": "albedo",
            "red_brick_03_nor_gl_2k.jpg": "normal_gl", "red_brick_03_nor_dx_2k.jpg": "normal_dx",
            "rock_01_rough.png": "roughness", "Bricks060.png": None, "Bricks060_2K-JPG.mtlx": None,
            "SurfaceImperfections003_2K-JPG_Opacity.jpg": "opacity",
        }
        for name, role in cases.items():
            self.assertEqual(fetch.role_of(name), role, name)


@unittest.skipIf(Image is None, "Pillow not installed")
class ConvertTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.root = self.tmp / "src"
        self.root.mkdir()
        Image.new("RGB", (64, 64), (200, 100, 50)).save(self.root / "a_Color.png")
        Image.new("RGB", (64, 64), (128, 40, 255)).save(self.root / "a_NormalDX.png")
        Image.new("I;16", (64, 64), 65535 // 2).save(self.root / "a_Roughness.png")
        Image.new("L", (64, 64), 77).save(self.root / "a_AmbientOcclusion.png")
        with zipfile.ZipFile(self.tmp / "set.zip", "w") as z:
            for p in sorted(self.root.iterdir()):
                z.write(p, p.name)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def asset(self, **data) -> "fetch.Asset":
        d = dict(entry(material="Metal"), **data)
        f = fetch.AssetFile(url="https://x/set.zip", sha256=SHA, size=1, map="archive", path="set.zip")
        return fetch.Asset(id="tex-metal-test", kind="texture", data=d, files=[f])

    def convert(self, cache: Path, sizes=(16, 32, 128), **data):
        a = self.asset(**data)
        root = fetch.materialize(a, cache, {SHA: self.tmp / "set.zip"}, log=lambda *_: None)
        return fetch.convert_texture(a, root, cache, sizes, log=lambda *_: None)

    def test_outputs_sizes_orm_and_flipped_normal(self):
        out = self.convert(self.tmp / "c1")
        self.assertEqual(sorted(p.name for p in out.iterdir() if p.is_dir()), ["16", "32"])
        orm = Image.open(out / "32" / "orm.png")
        r, g, b = orm.getpixel((5, 5))
        self.assertEqual(r, 77)                      # AO map
        self.assertIn(g, (127, 128))                 # 16-bit roughness scaled, not clipped
        self.assertEqual(b, 255)                     # Metal preset: metalness 1.0 when no map
        self.assertEqual(Image.open(out / "16" / "normal.png").getpixel((3, 3)), (128, 215, 255))  # DX -> GL
        self.assertEqual(Image.open(out / "16" / "albedo.png").size, (16, 16))
        info = json.loads((out / "material.json").read_text("utf-8"))
        self.assertEqual(info["sizes"], [16, 32])
        self.assertEqual(info["orm_defaults"], {"ao": None, "roughness": None, "metalness": 1.0})

    def test_deterministic_and_skips_when_stamp_matches(self):
        a = self.convert(self.tmp / "c1")
        b = self.convert(self.tmp / "c2")
        for p in sorted(a.rglob("*.png")):
            self.assertEqual(p.read_bytes(), (b / p.relative_to(a)).read_bytes(), p)
        before = (a / "16" / "albedo.png").stat().st_mtime_ns
        self.convert(self.tmp / "c1")
        self.assertEqual((a / "16" / "albedo.png").stat().st_mtime_ns, before)

    def test_tiny_pixel_source_keeps_native_size_with_nearest(self):
        out = self.convert(self.tmp / "c3", sizes=(512, 1024), texture={"nearest": True})
        info = json.loads((out / "material.json").read_text("utf-8"))
        self.assertEqual(info["sizes"], [64])
        self.assertEqual(info["filter"], "nearest")

    def test_explicit_members_and_convert_none(self):
        out = self.convert(self.tmp / "c4", texture={"members": {"albedo": "a_Color.png"}})
        self.assertFalse((out / "16" / "normal.png").exists())
        self.assertIsNone(self.convert(self.tmp / "c5", texture={"convert": "none"}))


class CliTests(unittest.TestCase):
    def test_check_and_licences_only(self):
        manifest = HERE.parents[1] / "assets" / "manifest"
        if not manifest.is_dir():
            self.skipTest("no manifest")
        tmp = Path(tempfile.mkdtemp())
        try:
            out = tmp / "ASSETS.md"
            rc = fetch.run(fetch.parse_args(["--check", "--licenses-out", str(out)]), log=lambda *_: None)
            self.assertEqual(rc, 0)
            self.assertIn("# Third-party assets", out.read_text("utf-8"))
            committed = HERE.parents[1] / "LICENSES" / "ASSETS.md"
            if committed.is_file():
                self.assertEqual(out.read_text("utf-8"), committed.read_text("utf-8"),
                                 "LICENSES/ASSETS.md is stale: run fetch.py --licenses-only")
        finally:
            shutil.rmtree(tmp)


if __name__ == "__main__":
    unittest.main(verbosity=2)
