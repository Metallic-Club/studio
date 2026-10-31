#!/usr/bin/env python3
"""Isolated cache-safety and process-lock tests: python -I tools/test_cargo_lane.py."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("cargo_lane", Path(__file__).with_name("cargo_lane.py"))
lane = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lane)

# A real child executable records argv and environment but never removes files.
STUB = '''import json, os, pathlib, sys, time
root = pathlib.Path.cwd()
target = pathlib.Path(os.environ["CARGO_TARGET_DIR"])
with (root / "calls.jsonl").open("a") as log:
    log.write(json.dumps({"args": sys.argv[1:], "target": str(target),
        "build": os.environ.get("CARGO_BUILD_BUILD_DIR")}) + "\\n")
command = sys.argv[1]
if command == "metadata":
    print(json.dumps({"workspace_root": str(root), "target_directory": str(target),
        "workspace_members": ["local-a", "local-b"], "packages": [
            {"id": "local-a", "name": "metallic_new_member", "source": None},
            {"id": "local-b", "name": "another_member", "source": None},
            {"id": "external", "name": "third_party", "source": "registry"}]}))
else:
    if command == "test" and (root / "wait").exists():
        (root / "started").touch()
        deadline = time.monotonic() + 15
        while not (root / "release").exists() and time.monotonic() < deadline:
            time.sleep(.02)
    failure = root / (command + "-status")
    if failure.exists():
        sys.exit(int(failure.read_text()))
'''


class LaneTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.workspace = self.make_workspace("first worktree")
        self.other = self.make_workspace("second worktree")
        self.target = self.root / "artifact lane"
        self.target.mkdir()
        (self.target / "CACHEDIR.TAG").write_text(
            "Signature: 8a477f597d28d172789f06886806bc55\n")
        self.stub = self.root / "stub cargo.py"
        self.stub.write_text(STUB, encoding="utf-8")
        self.cargo = (sys.executable, str(self.stub))

    def make_workspace(self, name):
        workspace = self.root / name
        workspace.mkdir()
        (workspace / "Cargo.toml").write_text("[workspace]\nmembers=[]\n", encoding="utf-8")
        return workspace

    def run_lane(self, workspace=None, args=None, **kwargs):
        return lane.run_lane(workspace or self.workspace, self.target,
                             args or ["test", "--workspace", "-j", "4"],
                             cargo=self.cargo, **kwargs)

    def calls(self, workspace=None):
        path = (workspace or self.workspace) / "calls.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def test_unknown_owner_cleans_only_metadata_workspace_members(self):
        self.assertEqual(self.run_lane(), 0)
        calls = self.calls()
        self.assertEqual([call["args"][0] for call in calls], ["metadata", "clean", "test"])
        self.assertEqual(calls[1]["args"], ["clean", "--target-dir", str(self.target),
                         "--profile", "dev",
                         "--package", "another_member", "--package", "metallic_new_member"])
        self.assertTrue(all(call["target"] == str(self.target) == call["build"] for call in calls))
        self.assertEqual(json.loads((self.target / lane.OWNER_NAME).read_text())["workspace"],
                         str(self.workspace))

    def test_same_owner_skips_clean_but_switch_and_recovery_clean(self):
        self.run_lane()
        self.run_lane()
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls()), 1)
        self.run_lane(self.other)
        self.assertEqual([call["args"][0] for call in self.calls(self.other)],
                         ["metadata", "clean", "test"])
        self.run_lane(self.other, reset=True)
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls(self.other)), 2)
        self.run_lane()
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls()), 2)

    def test_corrupt_owner_is_unknown(self):
        self.run_lane()
        (self.target / lane.OWNER_NAME).write_text("broken", encoding="utf-8")
        self.run_lane()
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls()), 2)

    def test_release_custom_profiles_and_targets_clean_matching_scope(self):
        self.run_lane()
        self.run_lane(args=["build", "--release", "--target", "example-target"])
        self.assertEqual(self.calls()[-2]["args"], ["clean", "--target-dir", str(self.target),
                         "--profile", "release", "--target", "example-target",
                         "--package", "another_member", "--package", "metallic_new_member"])
        # Equivalent explicit selection keeps ownership even with a different command.
        self.run_lane(args=["test", "--profile=bench", "--target=example-target"])
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls()), 2)
        self.run_lane(args=["build", "--profile=custom"])
        self.assertIn("custom", self.calls()[-2]["args"])
        self.run_lane(args=["bench"])
        self.assertEqual(self.calls()[-2]["args"][3:5], ["--profile", "release"])

    def test_inherited_build_target_change_requires_clean(self):
        self.run_lane()
        with patch.dict(os.environ, {"CARGO_BUILD_TARGET": "different-target"}):
            self.run_lane()
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls()), 2)
        self.assertEqual(json.loads((self.target / lane.OWNER_NAME).read_text())
                         ["selection"]["configured_target"], "different-target")

    def test_cargo_configuration_edits_require_clean_without_recording_contents(self):
        self.run_lane()
        configuration = self.workspace / ".cargo" / "config.toml"
        configuration.parent.mkdir()
        configuration.write_text('[build]\ntarget="example-target"\n# private-value\n')
        self.run_lane()
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls()), 2)
        owner = (self.target / lane.OWNER_NAME).read_text()
        self.assertNotIn("private-value", owner)
        self.run_lane()
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls()), 2)

    def test_included_config_edits_and_optional_file_appearance_invalidate_owner(self):
        configuration = self.workspace / ".cargo" / "config.toml"
        configuration.parent.mkdir()
        configuration.write_text('include=[{path="target.toml",optional=true}]\n')
        self.run_lane()
        included = configuration.with_name("target.toml")
        included.write_text('[build]\ntarget="first-target"\n')
        self.run_lane()
        included.write_text('[build]\ntarget="second-target"\n')
        self.run_lane()
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls()), 3)

    def test_config_include_cycle_fails_before_clean_or_owner_publish(self):
        configuration = self.workspace / ".cargo" / "config.toml"
        configuration.parent.mkdir()
        configuration.write_text('include=["config.toml"]\n')
        with self.assertRaises(lane.LaneError):
            self.run_lane()
        self.assertEqual([call["args"][0] for call in self.calls()], ["metadata"])
        self.assertFalse((self.target / lane.OWNER_NAME).exists())

    def test_clean_failure_preserves_previous_owner_and_never_builds(self):
        self.run_lane()
        previous = (self.target / lane.OWNER_NAME).read_bytes()
        (self.other / "clean-status").write_text("23")
        self.assertEqual(self.run_lane(self.other), 23)
        self.assertEqual((self.target / lane.OWNER_NAME).read_bytes(), previous)
        self.assertEqual([call["args"][0] for call in self.calls(self.other)], ["metadata", "clean"])

    def test_failed_test_status_is_preserved_and_owner_is_provenance(self):
        (self.workspace / "test-status").write_text("17")
        self.assertEqual(self.run_lane(), 17)
        self.assertTrue((self.target / lane.OWNER_NAME).is_file())
        self.assertEqual(self.run_lane(), 17)
        self.assertEqual(sum(call["args"][0] == "clean" for call in self.calls()), 1)

    def test_metadata_failure_never_cleans_or_claims(self):
        with patch.object(lane, "execute", return_value=subprocess.CompletedProcess(
                [], 19, stdout="", stderr="metadata failed\n")):
            self.assertEqual(self.run_lane(), 19)
        self.assertFalse((self.target / lane.OWNER_NAME).exists())

    def test_root_and_workspace_ancestor_rejected_before_cargo(self):
        for target in (Path(self.root.anchor), self.workspace, self.root, self.workspace / "target"):
            with self.subTest(target=target), self.assertRaises(lane.LaneError):
                lane.run_lane(self.workspace, target, ["test"], cargo=self.cargo)
        self.assertEqual(self.calls(), [])

    def test_symlink_component_and_state_rejected(self):
        actual = self.root / "actual"
        actual.mkdir()
        link = self.root / "link"
        try:
            link.symlink_to(actual, target_is_directory=True)
        except OSError:
            self.skipTest("OS does not permit creating symlinks")
        for target in (link, link / "nested"):
            with self.subTest(target=target), self.assertRaises(lane.LaneError):
                lane.run_lane(self.workspace, target, ["test"], cargo=self.cargo)
        self.target.mkdir(exist_ok=True)
        (self.target / lane.OWNER_NAME).symlink_to(self.root / "owner-elsewhere")
        with self.assertRaises(lane.LaneError):
            self.run_lane()
        self.assertEqual(self.calls(), [])

    @unittest.skipUnless(os.name == "nt", "Windows reparse path test")
    def test_windows_junction_is_rejected(self):
        actual = self.root / "actual"
        actual.mkdir()
        junction = self.root / "junction"
        # mklink /J is a directory junction and needs no symlink privilege.
        result = subprocess.run(["cmd", "/c", "mklink", "/J", str(junction), str(actual)],
                                capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        try:
            with self.assertRaises(lane.LaneError):
                lane.run_lane(self.workspace, junction / "nested", ["test"], cargo=self.cargo)
        finally:
            junction.rmdir()
        self.target.mkdir(exist_ok=True)
        for name in (lane.LOCK_NAME, lane.OWNER_NAME):
            state_link = self.target / name
            result = subprocess.run(["cmd", "/c", "mklink", "/J", str(state_link), str(actual)],
                                    capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            try:
                with self.assertRaises(lane.LaneError):
                    self.run_lane()
            finally:
                state_link.rmdir()
        self.assertEqual(self.calls(), [])

    def test_overrides_and_unrestricted_subcommands_rejected_before_cargo(self):
        invalid = [["clean"], ["rustc"], ["install"], ["+nightly", "test"],
                   ["test", "--target-dir=elsewhere"], ["test", "--target-dir", "elsewhere"],
                   ["test", "--config=build.target-dir='elsewhere'"], ["test", "--config", "x"],
                   ["test", "--manifest-path", "other/Cargo.toml"], ["test", "-mother"],
                   ["test", "-Cother"], ["test", "-Zunstable-options"],
                   ["test", "--build-dir=elsewhere"], ["build", "--artifact-dir=elsewhere"],
                   ["test", "-qr"], ["test", "--profile"], ["test", "--target="],
                   ["test", "--profile=../elsewhere"]]
        for arguments in invalid:
            with self.subTest(arguments=arguments), self.assertRaises(lane.LaneError):
                self.run_lane(args=arguments)
        self.assertEqual([path.name for path in self.target.iterdir()], ["CACHEDIR.TAG"])
        self.assertEqual(self.calls(), [])

    def test_clippy_rustc_output_flags_rejected_before_cleanup_or_execution(self):
        outside = self.root / "outside"
        for tail in (["-C", f"incremental={outside}"], [f"-Cincremental={outside}"],
                     ["--out-dir", str(outside)], [f"--emit=link={outside}"],
                     ["-Z", "unstable-options"]):
            with self.subTest(tail=tail), self.assertRaises(lane.LaneError):
                self.run_lane(args=["clippy", "--", *tail])
        self.assertFalse(outside.exists())
        self.assertEqual(self.calls(), [])
        self.assertEqual([path.name for path in self.target.iterdir()], ["CACHEDIR.TAG"])
        self.assertEqual(self.run_lane(args=["clippy", "--", "-D", "warnings",
                         "-Aclippy::too_many_arguments", "--cap-lints=warn"]), 0)

    def test_nonempty_unmarked_target_refused_without_tagging_or_cargo(self):
        unmarked = self.root / "not a cache"
        unmarked.mkdir()
        sentinel = unmarked / "valuable.txt"
        sentinel.write_text("retain me")
        with self.assertRaises(lane.LaneError):
            lane.run_lane(self.workspace, unmarked, ["test"], cargo=self.cargo)
        self.assertEqual(sentinel.read_text(), "retain me")
        self.assertFalse((unmarked / "CACHEDIR.TAG").exists())
        self.assertFalse((unmarked / lane.OWNER_NAME).exists())
        self.assertEqual(self.calls(), [])

    @unittest.skipUnless(shutil.which("cargo"), "real Cargo is not installed")
    def test_real_cargo_bootstraps_fresh_lane_then_reuses_owner(self):
        workspace = self.root / "real 日本 workspace"
        (workspace / "src").mkdir(parents=True)
        (workspace / "Cargo.toml").write_text(
            '[package]\nname="lane_bootstrap_probe"\nversion="0.1.0"\nedition="2021"\n')
        (workspace / "src" / "lib.rs").write_text("pub fn probe() -> i32 { 1 }\n")
        # The helper intentionally requires a lockfile for its locked metadata.
        subprocess.run(["cargo", "generate-lockfile"], cwd=workspace, check=True,
                       capture_output=True)
        fresh = self.root / "fresh real lane"
        # Python 3.11 on Windows commonly uses a legacy locale codec. Cargo
        # emits UTF-8 JSON regardless, so disable Python's UTF-8 mode explicitly.
        driver = ("import importlib.util,sys; "
                  "s=importlib.util.spec_from_file_location('lane',sys.argv[1]); "
                  "m=importlib.util.module_from_spec(s); s.loader.exec_module(m); "
                  "sys.exit(m.run_lane(sys.argv[2],sys.argv[3],['check','-j','1']))")
        result = subprocess.run([sys.executable, "-I", "-X", "utf8=0", "-c", driver,
                                 str(SPEC.origin), str(workspace), str(fresh)],
                                capture_output=True, text=True, encoding="utf-8")
        self.assertEqual(result.returncode, 0, result.stderr)
        owner = (fresh / lane.OWNER_NAME).read_bytes()
        self.assertTrue((fresh / "CACHEDIR.TAG").read_bytes().startswith(lane.CACHE_SIGNATURE))
        self.assertEqual(lane.run_lane(workspace, fresh, ["check", "-j", "1"]), 0)
        self.assertEqual((fresh / lane.OWNER_NAME).read_bytes(), owner)
        self.assertEqual(lane.run_lane(workspace, fresh, ["check", "--release", "-j", "1"]), 0)
        self.assertEqual(json.loads((fresh / lane.OWNER_NAME).read_text())["selection"]["profile"],
                         "release")

    def test_application_arguments_are_forwarded_as_literal_array(self):
        arguments = ["run", "-p", "another_member", "--", "--target-dir", "a b; $(no-shell)"]
        self.assertEqual(self.run_lane(args=arguments), 0)
        self.assertEqual(self.calls()[-1]["args"],
                         ["run", "--target-dir", str(self.target), *arguments[1:]])

    def test_metadata_workspace_or_target_mismatch_and_empty_members_rejected(self):
        metadata = {"workspace_root": str(self.workspace), "target_directory": str(self.target),
                    "workspace_members": [], "packages": []}
        for changed in ({}, {"workspace_root": str(self.other)},
                        {"target_directory": str(self.root / "elsewhere")}):
            with self.subTest(changed=changed), self.assertRaises(lane.LaneError):
                lane.workspace_packages(metadata | changed, self.workspace, self.target)

    def test_cli_returns_cargo_failure_and_lane_error(self):
        with patch.object(lane, "run_lane", return_value=31):
            self.assertEqual(lane.main(["--target-dir", str(self.target), "--", "test"]), 31)
        with patch.object(lane, "run_lane", side_effect=lane.LaneError("unsafe lane")):
            self.assertEqual(lane.main(["--target-dir", str(self.target), "--", "test"]), 2)

    def test_keyboard_interrupt_waits_for_child_before_returning(self):
        process = unittest.mock.MagicMock()
        process.__enter__.return_value = process
        process.communicate.side_effect = [KeyboardInterrupt(), (None, None)]
        process.returncode = 29
        with patch.object(lane.subprocess, "Popen", return_value=process):
            self.assertEqual(lane.execute(["stub"]).returncode, 29)
        self.assertEqual(process.communicate.call_count, 2)

    def start_guard(self, workspace):
        driver = ("import importlib.util,sys; "
                  "s=importlib.util.spec_from_file_location('lane',sys.argv[1]); "
                  "m=importlib.util.module_from_spec(s); s.loader.exec_module(m); "
                  "sys.exit(m.run_lane(sys.argv[2],sys.argv[3],['test'],"
                  "cargo=(sys.executable,sys.argv[4])))")
        process = subprocess.Popen([sys.executable, "-I", "-c", driver, str(SPEC.origin),
                                    str(workspace), str(self.target), str(self.stub)],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(self.finish_process, process)
        return process

    def finish_process(self, process):
        (self.workspace / "release").touch()
        (self.other / "release").touch()
        if process.poll() is None:
            process.wait(timeout=20)
        process.communicate()

    def wait_file(self, path):
        deadline = time.monotonic() + 10
        while not path.exists() and time.monotonic() < deadline:
            time.sleep(.02)
        self.assertTrue(path.exists(), str(path))

    def test_process_lock_covers_running_test_and_owner_transition(self):
        (self.workspace / "wait").touch()
        first = self.start_guard(self.workspace)
        self.wait_file(self.workspace / "started")
        second = self.start_guard(self.other)
        # Wait until second has had time to contend; no metadata or clean may run.
        time.sleep(.4)
        self.assertIsNone(second.poll())
        self.assertEqual(self.calls(self.other), [])
        self.assertEqual(json.loads((self.target / lane.OWNER_NAME).read_text())["workspace"],
                         str(self.workspace))
        (self.workspace / "release").touch()
        self.assertEqual(first.wait(timeout=15), 0)
        self.assertEqual(second.wait(timeout=15), 0)
        self.assertEqual([call["args"][0] for call in self.calls(self.other)],
                         ["metadata", "clean", "test"])
        self.assertEqual(json.loads((self.target / lane.OWNER_NAME).read_text())["workspace"],
                         str(self.other))


if __name__ == "__main__":
    unittest.main()
