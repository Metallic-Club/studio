"""Regression tests for the performance gate's failure policy and profile parsing."""
import argparse
import contextlib
import copy
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import measure


def report(count=100):
    return {
        "schema": 1, "metric": "instructions",
        "metadata": {key: "same" for key in ["rustc", "platform", "machine", "valgrind", "workload_sha256", "profile"]},
        "workloads": {name: {"samples": [count, count, count]} for name in measure.WORKLOADS},
    }


class ProfileParsing(unittest.TestCase):
    def test_instruction_column_is_named_not_assumed(self):
        self.assertEqual(measure.instructions("events: Dr Ir Dw\nsummary: 99 1234 88\n"), 1234)

    def test_missing_zero_negative_and_ambiguous_profiles_are_errors(self):
        for text in ["", "events: Dr\nsummary: 123\n", "events: Ir\nsummary: 0\n",
                     "events: Ir\nsummary: -1\n", "events: Ir Dr\nsummary: 3\n",
                     "events: Ir\nsummary: 5\nsummary: 6\n"]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                measure.instructions(text)


class GatePolicy(unittest.TestCase):
    def test_threshold_boundary_improvement_and_one_workload_regression(self):
        for name in ["reads", "writes", "clone", "create", "destroy"]:
            for count, allowed in [(90, True), (105, True), (106, False)]:
                head = report()
                head["workloads"][name] = {"samples": [count] * 3}
                with self.subTest(name=name, count=count):
                    passed, summary = measure.compare(report(), head, 5)
                    self.assertEqual(passed, allowed)
                    self.assertIn(name, summary)

    def test_old_three_workload_reports_and_each_missing_mutator_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            for missing in [("create",), ("destroy",), ("create", "destroy")]:
                data = report()
                for name in missing:
                    data["workloads"].pop(name, None)
                path.write_text(json.dumps(data), encoding="utf-8")
                with self.subTest(missing=missing), self.assertRaises(ValueError):
                    measure.read_report(path)

    def test_invalid_threshold_and_different_measurement_context_refuse_comparison(self):
        for threshold in [-1, float("nan"), float("inf"), 101]:
            with self.assertRaises(ValueError):
                measure.compare(report(), report(), threshold)
        for key in report()["metadata"]:
            head = report()
            head["metadata"][key] = "different"
            with self.subTest(key=key), self.assertRaises(ValueError):
                measure.compare(report(), head, 5)

    def test_reports_cannot_hide_a_missing_workload_or_substitute_wall_time(self):
        invalid = []
        walltime = report()
        walltime["metric"] = "walltime"
        invalid.append(walltime)
        missing = report()
        del missing["workloads"]["reads"]
        invalid.append(missing)
        for samples in [[], [100], [100, 100], [0, 100, 100], [True, 100, 100], [1.5, 100, 100]]:
            data = report()
            data["workloads"]["reads"]["samples"] = samples
            invalid.append(data)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            for data in invalid:
                path.write_text(json.dumps(data), encoding="utf-8")
                with self.subTest(data=data), self.assertRaises(ValueError):
                    measure.read_report(path)

    def test_cli_returns_failure_for_regression_and_invalid_baseline(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            base, head, summary = root / "base.json", root / "head.json", root / "summary.md"
            base.write_text(json.dumps(report()), encoding="utf-8")
            head.write_text(json.dumps(report(106)), encoding="utf-8")
            argv = ["measure.py", "compare", "--base", str(base), "--head", str(head), "--summary", str(summary)]
            with patch("sys.argv", argv), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(measure.main(), 1)
            self.assertIn("FAIL", summary.read_text(encoding="utf-8"))
            base.write_text("{}", encoding="utf-8")
            with patch("sys.argv", argv), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(measure.main(), 2)

    def test_reported_median_cannot_override_the_raw_samples(self):
        head = copy.deepcopy(report(106))
        for value in head["workloads"].values():
            value["median"] = 1
        self.assertFalse(measure.compare(report(), head, 5)[0])


class CollectionPolicy(unittest.TestCase):
    def collect_fixture(self, root, failing=None):
        binary, source = root / "perf_ci", root / "perf_ci.rs"
        binary.write_bytes(b"fixture executable; never run")
        source.write_text("fixture workload source", encoding="utf-8")
        args = argparse.Namespace(binary=binary, source=source, output=root / "report.json",
                                  repeats=3, metric="instructions", revision="fixture-revision")
        calls = []

        def fake_command(invocation):
            if invocation == ["valgrind", "--version"]:
                return "valgrind-fixture"
            if invocation == ["rustc", "-Vv"]:
                return "rustc-fixture"
            name = invocation[-1]
            calls.append((name, next(arg for arg in invocation if arg.startswith("--toggle-collect="))))
            profile = next(arg.split("=", 1)[1] for arg in invocation if arg.startswith("--callgrind-out-file="))
            Path(profile).write_text("events: Dr Ir\nsummary: 2 1234\n", encoding="utf-8")
            return json.dumps({"completed": name != failing, "workload": name})

        return args, calls, fake_command

    def test_every_fixed_symbol_is_collected_and_retained_three_times(self):
        expected = {name: f"metallic_perf_{name}" for name in ["reads", "writes", "clone", "create", "destroy"]}
        self.assertEqual(measure.WORKLOADS, expected)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args, calls, command = self.collect_fixture(root)
            with patch("measure.command", command), patch("measure.sys.platform", "linux"), contextlib.redirect_stdout(io.StringIO()):
                measure.collect(args)
            self.assertEqual(set(measure.read_report(args.output)["workloads"]), set(expected))
            self.assertEqual(len(list(root.glob("*.callgrind"))), 15)
            for name, symbol in expected.items():
                self.assertEqual(calls.count((name, f"--toggle-collect={symbol}")), 3)

    def test_failed_mutator_completion_cannot_publish_a_partial_report(self):
        for name in ["create", "destroy"]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                args, _, command = self.collect_fixture(Path(temporary), failing=name)
                with patch("measure.command", command), patch("measure.sys.platform", "linux"), contextlib.redirect_stdout(io.StringIO()):
                    with self.assertRaisesRegex(ValueError, "workload did not complete"):
                        measure.collect(args)
                self.assertFalse(args.output.exists())

    def test_every_lua_symbol_is_collected_and_retained_three_times(self):
        expected = {name: f"metallic_perf_{name}" for name in ["lua_reads", "lua_writes", "lua_method_calls"]}
        self.assertEqual(measure.LUA_WORKLOADS, expected)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args, calls, command = self.collect_fixture(root)
            with patch("measure.command", command), patch("measure.sys.platform", "linux"), contextlib.redirect_stdout(io.StringIO()):
                measure.collect(args, measure.LUA_WORKLOADS)
            self.assertEqual(set(measure.read_report(args.output, measure.LUA_WORKLOADS)["workloads"]), set(expected))
            self.assertEqual(len(list(root.glob("*.callgrind"))), 9)
            for name, symbol in expected.items():
                self.assertEqual(calls.count((name, f"--toggle-collect={symbol}")), 3)

    def test_failed_lua_completion_cannot_publish_a_partial_report(self):
        for name in measure.LUA_WORKLOADS:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                args, _, command = self.collect_fixture(Path(temporary), failing=name)
                with patch("measure.command", command), patch("measure.sys.platform", "linux"), contextlib.redirect_stdout(io.StringIO()):
                    with self.assertRaisesRegex(ValueError, "workload did not complete"):
                        measure.collect(args, measure.LUA_WORKLOADS)
                self.assertFalse(args.output.exists())


def lua_report(count=100):
    data = report(count)
    data["workloads"] = {name: {"samples": [count, count, count]} for name in measure.LUA_WORKLOADS}
    return data


class LuaSuitePolicy(unittest.TestCase):
    def compare(self, head, new=False):
        return measure.compare(lua_report(), head, 5, measure.LUA_WORKLOADS, new)

    def test_same_threshold_for_every_lua_workload_with_base_and_head_numbers(self):
        for name in measure.LUA_WORKLOADS:
            for count, allowed in [(90, True), (105, True), (106, False)]:
                head = lua_report()
                head["workloads"][name] = {"samples": [count] * 3}
                with self.subTest(name=name, count=count):
                    passed, summary = self.compare(head)
                    self.assertEqual(passed, allowed)
                    self.assertIn(f"| {name} | 100 | {count} |", summary)

    def test_suites_do_not_accept_each_others_reports(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            path.write_text(json.dumps(report()), encoding="utf-8")
            with self.assertRaises(ValueError):
                measure.read_report(path, measure.LUA_WORKLOADS)
            path.write_text(json.dumps(lua_report()), encoding="utf-8")
            with self.assertRaises(ValueError):
                measure.read_report(path)

    def test_a_missing_lua_workload_is_rejected(self):
        for name in measure.LUA_WORKLOADS:
            data = lua_report()
            del data["workloads"][name]
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / "report.json"
                path.write_text(json.dumps(data), encoding="utf-8")
                with self.assertRaises(ValueError):
                    measure.read_report(path, measure.LUA_WORKLOADS)

    def test_new_workloads_are_reported_with_their_numbers_and_do_not_fail(self):
        passed, summary = self.compare(lua_report(106), new=True)
        self.assertTrue(passed)
        for name in measure.LUA_WORKLOADS:
            self.assertIn(f"| {name} | 100 | 106 | +6.00% | NEW (not gated) |", summary)
        self.assertNotIn("FAIL", summary)

    def test_new_does_not_excuse_an_invalid_comparison(self):
        for key in lua_report()["metadata"]:
            head = lua_report()
            head["metadata"][key] = "different"
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.compare(head, new=True)

    def test_new_applies_only_when_asked(self):
        self.assertFalse(self.compare(lua_report(106))[0])
        self.assertFalse(measure.compare(report(), report(106), 5)[0])

    def test_cli_selects_the_suite_and_new_only_waives_the_threshold(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            base, head, summary = root / "base.json", root / "head.json", root / "summary.md"
            base.write_text(json.dumps(lua_report()), encoding="utf-8")
            head.write_text(json.dumps(lua_report(106)), encoding="utf-8")
            argv = ["measure.py", "compare", "--suite", "lua", "--base", str(base), "--head", str(head), "--summary", str(summary)]
            with patch("sys.argv", argv), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(measure.main(), 1)
            self.assertIn("FAIL", summary.read_text(encoding="utf-8"))
            with patch("sys.argv", [*argv, "--new"]), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(measure.main(), 0)
            self.assertIn("NEW", summary.read_text(encoding="utf-8"))
            base.write_text("{}", encoding="utf-8")
            with patch("sys.argv", [*argv, "--new"]), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(measure.main(), 2)


if __name__ == "__main__":
    unittest.main()
