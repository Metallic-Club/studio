#!/usr/bin/env python3
"""Runner policy/process tests; these do not create a graphics device."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("diagnose_headless", Path(__file__).with_name("diagnose_headless.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class MatrixTests(unittest.TestCase):
    def run_matrix(self, ready=4, unavailable=0, fail_at=None, timed_out=False):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        executable = root / "owned-headless-fixture"
        executable.write_bytes(b"fixed executable hash fixture")
        calls = []

        def launch(command, timeout, environment):
            calls.append(command)
            lines = ["[metallic-gpu] phase=adapter.ready backend=fixture"] * ready
            lines += ["[metallic-gpu] phase=context.unavailable reason=fixture"] * unavailable
            code = 139 if fail_at == len(calls) else 0
            return code, timed_out, b"retained stdout", "\n".join(lines).encode(), 0.001

        output = root / "evidence"
        code = runner.matrix(executable, output, "source-fixture", launch=launch, environment={})
        return code, json.loads((output / "results.json").read_text()), calls, output

    def test_balanced_pairs_require_actual_gpu_contexts(self):
        code, result, calls, _ = self.run_matrix()
        self.assertEqual(code, 0)
        self.assertEqual([c[1] for c in calls], [f"--test-threads={n}" for n in (1, 4, 4, 1, 1, 4)])
        self.assertEqual(result["outcome"], "no_crash_observed")
        self.assertTrue(all(t["gpu_contexts_ready"] == 4 for t in result["trials"]))

    def test_adapter_absence_is_inconclusive(self):
        for ready, unavailable in [(0, 4), (3, 1), (0, 0)]:
            with self.subTest(ready=ready, unavailable=unavailable):
                code, result, _, _ = self.run_matrix(ready, unavailable)
                self.assertEqual(code, 2)
                self.assertEqual(result["outcome"], "inconclusive_adapter_coverage")

    def test_failure_stops_and_preserves_status_and_output(self):
        code, result, calls, output = self.run_matrix(fail_at=2)
        self.assertEqual(code, 1)
        self.assertEqual(len(calls), 2)
        self.assertEqual(result["trials"][-1]["exit"]["raw"], 139)
        self.assertEqual(result["trials"][-1]["exit"]["kind"], "exit")
        self.assertEqual((output / "02-threads-4.stdout.log").read_bytes(), b"retained stdout")

    def test_timeout_stops_without_another_trial(self):
        code, result, calls, _ = self.run_matrix(timed_out=True)
        self.assertEqual(code, 1)
        self.assertEqual(len(calls), 1)
        self.assertEqual(result["outcome"], "timeout")

    def test_exit_status_keeps_signal_and_windows_status_distinct(self):
        self.assertEqual(runner.exit_status(-11, False)["signal"], 11)
        self.assertEqual(runner.exit_status(139, False)["kind"], "exit")
        status = runner.exit_status(0xC0000374, True)
        self.assertEqual(status["raw"], 0xC0000374)
        self.assertEqual(status["signed_32"], -1073740940)
        self.assertEqual(status["hex_32"], "0xc0000374")
        self.assertTrue(status["windows_status_like"])

    def test_changed_executable_stops_before_another_child(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "headless"
            executable.write_bytes(b"original")
            calls = []

            def launch(command, timeout, environment):
                calls.append(command)
                executable.write_bytes(b"changed")
                return 0, False, b"", b"", 0.0

            output = root / "evidence"
            self.assertEqual(runner.matrix(executable, output, "fixture", launch=launch), 1)
            self.assertEqual(len(calls), 1)
            result = json.loads((output / "results.json").read_text())
            self.assertEqual(result["outcome"], "executable_changed")
            with self.assertRaises(FileExistsError):
                runner.matrix(executable, output, "fixture", launch=launch)

    def test_last_successful_trial_mutation_is_not_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "headless"
            executable.write_bytes(b"original")
            calls = []

            def launch(command, timeout, environment):
                calls.append(command)
                if len(calls) == 6:
                    executable.write_bytes(b"changed during final child")
                markers = b"[metallic-gpu] phase=adapter.ready backend=fixture\n" * 4
                return 0, False, b"retained last stdout", markers, 0.0

            output = root / "evidence"
            self.assertEqual(runner.matrix(executable, output, "fixture", launch=launch), 1)
            result = json.loads((output / "results.json").read_text())
            self.assertEqual(len(calls), 6)
            self.assertEqual(result["outcome"], "executable_changed")
            self.assertTrue(result["trials"][-1]["executable_changed"])
            self.assertEqual(result["trials"][-1]["exit"]["raw"], 0)
            self.assertEqual((output / "06-threads-4.stdout.log").read_bytes(), b"retained last stdout")

    def test_changed_image_does_not_replace_a_child_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "headless"
            executable.write_bytes(b"original")

            def launch(command, timeout, environment):
                executable.write_bytes(b"changed")
                return 139, False, b"failure stdout", b"failure stderr", 0.0

            output = root / "evidence"
            self.assertEqual(runner.matrix(executable, output, "fixture", launch=launch), 1)
            result = json.loads((output / "results.json").read_text())
            self.assertEqual(result["outcome"], "child_failure")
            self.assertEqual(len(result["trials"]), 1)
            self.assertEqual(result["trials"][0]["exit"]["raw"], 139)
            self.assertTrue(result["trials"][0]["executable_changed"])
            self.assertEqual((output / "01-threads-1.stderr.log").read_bytes(), b"failure stderr")

    def test_owned_child_is_reaped_on_timeout(self):
        code, timed_out, stdout, stderr, _ = runner.execute(
            [sys.executable, "-c", "import time; print('started', flush=True); time.sleep(10)"],
            0.2, dict(os.environ),
        )
        self.assertTrue(timed_out)
        self.assertIsInstance(code, int)
        self.assertIsInstance(stdout, bytes)
        self.assertEqual(stderr, b"")


if __name__ == "__main__":
    unittest.main()
