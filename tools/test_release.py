#!/usr/bin/env python3
"""Tests for tools/release.py version checks. Run: python -I tools/test_release.py"""
import importlib.util
import contextlib
import io
import os
import re
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("release", Path(__file__).resolve().parent / "release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)

CARGO = '[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n'


class VersionTests(unittest.TestCase):
    def test_package_and_binary_match_the_actual_manifest(self):
        manifest = tomllib.loads((release.ROOT / "crates/metallic_studio/Cargo.toml").read_text(encoding="utf-8"))
        self.assertEqual(release.CARGO_PACKAGE, manifest["package"]["name"])
        self.assertIn(release.CARGO_BIN, [binary["name"] for binary in manifest["bin"]])
        self.assertEqual(release.MAIN_EXE, "Metallic Studio.exe")

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


class PhaseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.built = self.root / "target/release" / f"{release.CARGO_BIN}.exe"
        self.built.parent.mkdir(parents=True)
        self.built.write_bytes(b"MZ: inert test bytes, never executed")
        (self.root / "Cargo.toml").write_text(CARGO, encoding="utf-8")
        for name, value in {
            "ROOT": self.root,
            "OUT_DIR": self.root / "target/release-out",
            "CARGO_TOML": self.root / "Cargo.toml",
            "ICON_PNG": self.root / "branding/placeholder-icon.png",
            "ICON_ICO": self.root / "branding/icon.ico",
        }.items():
            context = patch.object(release, name, value)
            context.start()
            self.addCleanup(context.stop)
        self.quiet = contextlib.redirect_stdout(io.StringIO())
        self.quiet.__enter__()
        self.addCleanup(self.quiet.__exit__, None, None, None)

    def arguments(self, *extra):
        return ["--channel", "dev", "--version", "0.1.0", *map(str, extra)]

    def test_build_only_never_reads_token_or_enters_packaging(self):
        class NoPublishingToken(dict):
            def get(self, key, default=None):
                if key in ("GITHUB_TOKEN", "GH_TOKEN"):
                    raise AssertionError("build-only read publishing credentials")
                return super().get(key, default)

        with (patch.object(release.os, "environ", NoPublishingToken()),
              patch.object(release, "build_studio", return_value=self.built) as build,
              patch.object(release, "check_vpk_version") as version,
              patch.object(release, "stage_executable") as stage,
              patch.object(release, "run") as run):
            release.main(self.arguments("--build-only"))
        build.assert_called_once_with(4, False)
        version.assert_not_called()
        stage.assert_not_called()
        run.assert_not_called()

    def test_cargo_command_is_locked_and_credentials_are_removed(self):
        with (patch.dict(os.environ, {"GITHUB_TOKEN": "test-only-token", "GH_TOKEN": "test-only-gh"}),
              patch.object(release.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as child):
            result = release.build_studio(4, False)
            self.assertEqual(os.environ["GITHUB_TOKEN"], "test-only-token")
        self.assertEqual(result, self.built)
        argv = child.call_args.args[0]
        self.assertEqual(argv[:4], ["cargo", "build", "--release", "--locked"])
        self.assertEqual(argv[argv.index("-p") + 1], release.CARGO_PACKAGE)
        self.assertEqual(argv[argv.index("--bin") + 1], release.CARGO_BIN)
        self.assertEqual(argv[argv.index("--target-dir") + 1], str(self.root / "target"))
        self.assertNotIn("GITHUB_TOKEN", child.call_args.kwargs["env"])
        self.assertNotIn("GH_TOKEN", child.call_args.kwargs["env"])

    def test_publish_only_never_builds_and_preserves_pack_upload_flow(self):
        with (patch.dict(os.environ, {"GITHUB_TOKEN": "test-only-token"}),
              patch.object(release, "build_studio", side_effect=AssertionError("Cargo in publish phase")),
              patch.object(release, "check_vpk_version"),
              patch.object(release, "make_icon", return_value=self.root / "trusted.ico"),
              patch.object(release, "run", return_value=0) as run):
            release.main(self.arguments("--publish-only", "--input-exe", self.built, "--required"))
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual([command[:2] for command in commands],
                         [["vpk", "download"], ["vpk", "pack"], ["vpk", "upload"]])
        self.assertTrue(all(command[0] == "vpk" for command in commands))
        pack, upload = commands[1:]
        self.assertEqual(pack[pack.index("--mainExe") + 1], "Metallic Studio.exe")
        self.assertEqual(pack[pack.index("--packId") + 1], "MetallicStudio")
        self.assertIn("--pre", upload)
        self.assertEqual(upload[upload.index("--tag") + 1], "v0.1.0-dev")
        stage = self.root / "target/release-out/stage/Metallic Studio.exe"
        self.assertEqual(stage.read_bytes(), self.built.read_bytes())
        self.assertIn("required: true", (release.OUT_DIR / "release-notes.md").read_text())

    def test_publish_no_upload_only_packs(self):
        with (patch.object(release, "check_vpk_version"),
              patch.object(release, "make_icon", return_value=self.root / "trusted.ico"),
              patch.object(release, "run", return_value=0) as run):
            release.main(self.arguments("--publish-only", "--input-exe", self.built, "--no-upload"))
        self.assertEqual([call.args[0][:2] for call in run.call_args_list], [["vpk", "pack"]])

    def test_publish_requires_token_before_staging_or_running_tools(self):
        with (patch.dict(os.environ, {}, clear=True),
              patch.object(release, "run") as run,
              patch.object(release, "check_vpk_version") as version,
              self.assertRaises(SystemExit)):
            release.main(self.arguments("--publish-only", "--input-exe", self.built))
        run.assert_not_called()
        version.assert_not_called()
        self.assertFalse(release.OUT_DIR.exists())

    def test_dry_run_phases_execute_no_processes(self):
        with patch.object(release.subprocess, "run", side_effect=AssertionError("dry-run spawned a process")):
            release.main(self.arguments("--build-only", "--dry-run"))
            release.main(self.arguments("--publish-only", "--input-exe", self.built, "--dry-run"))
        self.assertFalse(release.OUT_DIR.exists())

    def test_invalid_phase_arguments_stop_before_build_or_pack(self):
        for extra in [("--publish-only",), ("--input-exe", self.built),
                      ("--build-only", "--publish-only"), ("--build-only", "--jobs", "0")]:
            with (self.subTest(extra=extra), contextlib.redirect_stderr(io.StringIO()),
                  patch.object(release, "build_studio") as build,
                  patch.object(release, "run") as run, self.assertRaises(SystemExit)):
                release.main(self.arguments(*extra))
            build.assert_not_called()
            run.assert_not_called()

    def test_missing_input_does_not_remove_previous_stage(self):
        stage = release.OUT_DIR / "stage"
        stage.mkdir(parents=True)
        marker = stage / "keep"
        marker.write_text("keep")
        with self.assertRaises(SystemExit):
            release.stage_executable(self.root / "missing.exe", False)
        self.assertEqual(marker.read_text(), "keep")

    def test_input_inside_stage_is_not_deleted(self):
        stage = release.OUT_DIR / "stage"
        stage.mkdir(parents=True)
        source = stage / "input.exe"
        source.write_bytes(b"keep")
        with self.assertRaises(SystemExit):
            release.stage_executable(source, False)
        self.assertEqual(source.read_bytes(), b"keep")

    def test_masked_output_hides_token_values(self):
        text = release.masked(["vpk", "upload", "github", "--token", "test-only-token"])
        self.assertIn("<token>", text)
        self.assertNotIn("test-only-token", text)


class WorkflowBoundaryTests(unittest.TestCase):
    """Fixed-workflow contract checks complement the actual CLI phase tests."""

    def test_build_and_publish_have_separate_credentials_and_runners(self):
        text = (Path(__file__).resolve().parent.parent / ".github/workflows/release.yml").read_text()
        prefix, jobs = text.split("jobs:\n", 1)
        build, publish = jobs.split("  publish:\n", 1)
        self.assertIn("permissions: {}", prefix)
        for unprivileged in [prefix, build]:
            for credential in ["secrets.", "GITHUB_TOKEN:", "GH_TOKEN:", "contents: write"]:
                self.assertNotIn(credential, unprivileged)
        self.assertIn("contents: read", build)
        self.assertIn("contents: write", publish)
        self.assertIn("needs: build", publish)
        self.assertEqual(text.count("persist-credentials: false"), 2)
        self.assertEqual(text.count("ref: ${{ github.sha }}"), 2)
        self.assertNotIn("rust-cache", text)
        self.assertIn("--build-only", build)
        self.assertIn("--publish-only", publish)
        self.assertNotIn("--build-only", publish)
        self.assertNotIn("rustup", publish)
        self.assertEqual(text.count("GITHUB_TOKEN:"), 1)

    def test_only_same_run_binary_artifact_is_handed_off_with_strict_digest(self):
        text = (Path(__file__).resolve().parent.parent / ".github/workflows/release.yml").read_text()
        self.assertIn(f"path: target/release/{release.CARGO_BIN}.exe", text)
        self.assertIn("archive: false", text)
        self.assertIn("if-no-files-found: error", text)
        self.assertIn("artifact-ids: ${{ needs.build.outputs.artifact-id }}", text)
        self.assertIn("skip-decompress: true", text)
        self.assertIn("digest-mismatch: error", text)
        self.assertNotIn("github-token:", text)
        self.assertNotIn("run-id:", text)
        uses = re.findall(r"uses: ([^\s#]+)", text)
        self.assertTrue(uses)
        for action in uses:
            self.assertRegex(action, r"^[A-Za-z0-9_/-]+@[0-9a-f]{40}$")


if __name__ == "__main__":
    unittest.main()
