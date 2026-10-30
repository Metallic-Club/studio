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


if __name__ == "__main__":
    unittest.main()
