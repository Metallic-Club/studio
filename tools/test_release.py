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


class VpkVersionTests(unittest.TestCase):
    # vpk 1.3.x has no --version flag; its version is only printed in the -h description.
    HELP = ("Description:\n  Velopack CLI 1.3.39-g8d2844f (prerelease), for distributing applications.  \n\n"
            "Usage:\n  vpk [command] [options]\n")

    def test_reads_version_from_help(self):
        self.assertEqual(release.vpk_version_from_help(self.HELP), "1.3.39-g8d2844f")

    def test_help_without_version_is_none(self):
        self.assertIsNone(release.vpk_version_from_help("Usage:\n  vpk [command]\n"))


class StagedBinaryTests(unittest.TestCase):
    """Staged file names must be real [[bin]] names, or the release build fails after compiling."""

    @staticmethod
    def bin_names(package):
        import tomllib
        text = (release.ROOT / "crates" / package / "Cargo.toml").read_text(encoding="utf-8")
        return {b["name"] for b in tomllib.loads(text).get("bin", [])}

    def test_studio_exe_is_its_bin(self):
        self.assertIn(release.STUDIO_BUILT_EXE.removesuffix(".exe"), self.bin_names(release.CARGO_BIN))

    def test_studio_ships_its_templates(self):
        # A fresh install otherwise shows "No templates found" on the start screen.
        self.assertIn("templates", release.STUDIO_FOLDERS)
        self.assertTrue(any((release.ROOT / "templates").glob("*/template.toml")))
        for folder in release.STUDIO_FOLDERS:
            self.assertTrue((release.ROOT / folder).is_dir(), folder)

    def test_launcher_files_are_bins(self):
        bins = set().union(*(self.bin_names(p) for p in release.LAUNCHER_CARGO_PACKAGES))
        for built, _ in release.LAUNCHER_FILES:
            self.assertIn(built.removesuffix(".exe"), bins)


class BuildCommandTests(unittest.TestCase):
    """Shipped builds use [profile.dist] and its output folder, never target/release."""

    def test_build_lines(self):
        self.assertEqual(release.build_command("studio", 4),
                         ["cargo", "build", "--locked", "--profile", "dist", "-p", "metallic_studio", "-j", "4"])
        self.assertEqual(release.build_command("launcher", 2),
                         ["cargo", "build", "--locked", "--profile", "dist", "-p", "metallic_launcher",
                          "-p", "metallic_player", "-j", "2"])

    def test_dist_profile_exists_and_keeps_unwinding(self):
        import tomllib
        profiles = tomllib.loads(release.CARGO_TOML.read_text(encoding="utf-8"))["profile"]
        dist = profiles[release.CARGO_PROFILE]
        self.assertEqual(dist["inherits"], "release")
        # metallic_asset catches glTF parser panics and mlua turns callback panics into Lua errors.
        self.assertEqual(dist["panic"], "unwind")
        self.assertEqual(release.BUILD_DIR, release.ROOT / "target" / "dist")

    def test_symbol_file_names(self):
        self.assertEqual(release.symbol_file("metallic-studio.exe"), "metallic_studio.pdb")
        self.assertEqual(release.symbol_file("Metallic.exe"), "Metallic.pdb")


class PackCommandTests(unittest.TestCase):
    ARGS = ("vpk", "STAGE", "ICON", "NOTES")

    def test_studio_pack_line(self):
        vpk, stage, icon, notes = self.ARGS
        out = str(release.OUT_DIR)
        self.assertEqual(
            release.pack_command(vpk, "studio", "0.1.0", "dev", stage, icon, notes),
            ["vpk", "pack", "--packId", "MetallicStudio", "--packVersion", "0.1.0", "--packDir", "STAGE",
             "--mainExe", "Metallic Studio.exe", "--packTitle", "Metallic Studio", "--icon", "ICON",
             "--runtime", "win-x64", "--channel", "dev", "--outputDir", out, "--releaseNotes", "NOTES"])

    def test_studio_pack_title_is_never_a_bare_metallic(self):
        # A bare "Metallic" title would overwrite another app's Metallic.lnk shortcut.
        self.assertEqual(release.PACK_TITLE, "Metallic Studio")
        self.assertEqual(release.PACK_ID, "MetallicStudio")
        cmd = release.pack_command("vpk", "studio", "0.1.0", "dev", "S", "I", "N")
        self.assertNotEqual(cmd[cmd.index("--packTitle") + 1].strip().lower(), "metallic")

    def test_launcher_pack_line(self):
        vpk, stage, icon, notes = self.ARGS
        cmd = release.pack_command(vpk, "launcher", "0.1.0", "dev", stage, icon, notes)
        pairs = dict(zip(cmd[2::2], cmd[3::2]))
        self.assertEqual(pairs["--packId"], "MetallicLauncher")
        self.assertEqual(pairs["--mainExe"], "Metallic.exe")
        self.assertEqual(pairs["--framework"], "webview2")
        self.assertEqual(pairs["--runtime"], "win-x64")
        self.assertEqual(pairs["--channel"], "launcher-dev")
        self.assertEqual(pairs["--packTitle"], "Metallic Play")

    def test_dry_run_prints_the_launcher_commands(self):
        import subprocess
        import sys
        script = Path(release.__file__)
        out = subprocess.run([sys.executable, str(script), "--app", "launcher", "--channel", "dev",
                              "--version", release.workspace_version(release.CARGO_TOML.read_text(encoding="utf-8")),
                              "--dry-run"], capture_output=True, text=True, check=True).stdout
        self.assertIn("+ cargo build --locked --profile dist -p metallic_launcher -p metallic_player -j 4", out)
        self.assertIn("+ keep metallic_player.pdb in ", out)
        self.assertIn("--framework webview2", out)
        self.assertIn("--packId MetallicLauncher", out)
        self.assertIn("--channel launcher-dev", out)
        self.assertIn("--tag launcher-v", out)
        self.assertNotIn("MetallicStudio", out)

    def test_dry_run_studio(self):
        import subprocess
        import sys
        version = release.workspace_version(release.CARGO_TOML.read_text(encoding="utf-8"))
        out = subprocess.run([sys.executable, str(Path(release.__file__)), "--channel", "dev",
                              "--version", version, "--dry-run"], capture_output=True, text=True, check=True).stdout
        self.assertIn("+ cargo build --locked --profile dist -p metallic_studio -j 4", out)
        self.assertIn("+ keep metallic_studio.pdb in ", out)
        self.assertIn("+ stage metallic-studio.exe as Metallic Studio.exe in ", out)
        self.assertIn(f"--tag v{version}-dev --releaseName \"Metallic Studio {version} (dev)\" --publish --token <token> --pre", out)
        self.assertNotIn("--framework", out)
        self.assertIn("--runtime win-x64", out)
        self.assertNotIn("MetallicLauncher", out)


if __name__ == "__main__":
    unittest.main()
