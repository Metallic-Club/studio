#!/usr/bin/env python3
"""Tests for tools/release.py version checks. Run: python -I tools/test_release.py"""
import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("release", Path(__file__).resolve().parent / "release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)

CARGO = '[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n'


class VersionTests(unittest.TestCase):
    def test_reads_workspace_version(self):
        self.assertEqual(release.workspace_version(CARGO), "0.1.0")

    def test_missing_version_is_an_error(self):
        with self.assertRaises(SystemExit):
            release.workspace_version("[workspace]\nmembers = []\n")

    def test_mismatch_fails(self):
        with self.assertRaises(SystemExit):
            release.check_version("0.1.1", CARGO)

    def test_match_passes(self):
        release.check_version("0.1.0", CARGO)

    def test_version_format(self):
        for good in ["0.1.0", "10.20.30"]:
            self.assertTrue(release.valid_version(good), good)
        for bad in ["01.1.0", "1.2", "v1.2.3", "1.2.3-dev", "1.2.3\n", "١.2.3", ""]:
            self.assertFalse(release.valid_version(bad), bad)


class PackCommandTests(unittest.TestCase):
    ARGS = ("vpk", "STAGE", "ICON", "NOTES")

    def test_studio_pack_line_is_unchanged(self):
        vpk, stage, icon, notes = self.ARGS
        out = str(release.OUT_DIR)
        self.assertEqual(
            release.pack_command(vpk, "studio", "0.1.0", "dev", stage, icon, notes),
            ["vpk", "pack", "--packId", "MetallicStudio", "--packVersion", "0.1.0", "--packDir", "STAGE",
             "--mainExe", "Metallic Studio.exe", "--packTitle", "Metallic Studio", "--icon", "ICON",
             "--channel", "dev", "--outputDir", out, "--releaseNotes", "NOTES"])

    def test_launcher_pack_line(self):
        vpk, stage, icon, notes = self.ARGS
        cmd = release.pack_command(vpk, "launcher", "0.1.0", "dev", stage, icon, notes)
        pairs = dict(zip(cmd[2::2], cmd[3::2]))
        self.assertEqual(pairs["--packId"], "MetallicLauncher")
        self.assertEqual(pairs["--mainExe"], "Metallic.exe")
        self.assertEqual(pairs["--framework"], "webview2")
        self.assertEqual(pairs["--runtime"], "win-x64")
        self.assertEqual(pairs["--channel"], "launcher-dev")
        self.assertEqual(pairs["--packTitle"], "Metallic")

    def test_dry_run_prints_the_launcher_commands(self):
        import subprocess
        import sys
        script = Path(release.__file__)
        out = subprocess.run([sys.executable, str(script), "--app", "launcher", "--channel", "dev",
                              "--version", release.workspace_version(release.CARGO_TOML.read_text(encoding="utf-8")),
                              "--dry-run"], capture_output=True, text=True, check=True).stdout
        self.assertIn("-p metallic_launcher -p metallic_player", out)
        self.assertIn("--framework webview2", out)
        self.assertIn("--packId MetallicLauncher", out)
        self.assertIn("--channel launcher-dev", out)
        self.assertIn("--tag launcher-v", out)
        self.assertNotIn("MetallicStudio", out)

    def test_dry_run_studio_is_unchanged(self):
        import subprocess
        import sys
        version = release.workspace_version(release.CARGO_TOML.read_text(encoding="utf-8"))
        out = subprocess.run([sys.executable, str(Path(release.__file__)), "--channel", "dev",
                              "--version", version, "--dry-run"], capture_output=True, text=True, check=True).stdout
        self.assertIn("+ cargo build --release -p metallic_studio -j 4", out)
        self.assertIn("+ stage metallic_studio.exe as Metallic Studio.exe in ", out)
        self.assertIn(f"--tag v{version}-dev --releaseName \"Metallic Studio {version} (dev)\" --publish --token <token> --pre", out)
        self.assertNotIn("--framework", out)
        self.assertNotIn("--runtime", out)
        self.assertNotIn("MetallicLauncher", out)


if __name__ == "__main__":
    unittest.main()
