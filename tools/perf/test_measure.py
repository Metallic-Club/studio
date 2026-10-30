"""Regression tests for the performance gate's failure policy and profile parsing."""
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
        for count, allowed in [(90, True), (105, True), (106, False)]:
            head = report()
            head["workloads"]["writes"]["samples"] = [count] * 3
            passed, summary = measure.compare(report(), head, 5)
            self.assertEqual(passed, allowed)
            self.assertIn("writes", summary)

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


if __name__ == "__main__":
    unittest.main()
