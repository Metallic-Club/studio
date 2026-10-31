"""Collect bounded core measurements or fail closed on an instruction regression.

Uses only Python's standard library. Wall-time reports are trend artifacts, never PR gates.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import platform
import statistics
import subprocess
import sys
from pathlib import Path

WORKLOADS = {"reads": "metallic_perf_reads", "writes": "metallic_perf_writes", "clone": "metallic_perf_clone"}


def command(args: list[str]) -> str:
    result = subprocess.run(args, capture_output=True, text=True, timeout=180)
    if result.returncode:
        raise ValueError(f"measurement command failed (exit {result.returncode}): {result.stderr.strip()[-4000:]}")
    return result.stdout.strip()


def instructions(text: str) -> int:
    """Find Ir by its declared column, rejecting missing/ambiguous/empty collections."""
    events = [line.split()[1:] for line in text.splitlines() if line.startswith("events:")]
    summaries = [line.split()[1:] for line in text.splitlines() if line.startswith("summary:")]
    if len(events) != 1 or len(summaries) != 1 or events[0].count("Ir") != 1:
        raise ValueError("expected one events/summary section containing Ir")
    if len(events[0]) != len(summaries[0]):
        raise ValueError("event and summary column counts differ")
    count = int(summaries[0][events[0].index("Ir")])
    if count <= 0:
        raise ValueError("no instructions collected; verify the collection symbol")
    return count


def read_report(path: Path) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict) or type(data.get("schema")) is not int or data.get("schema") != 1 or data.get("metric") != "instructions":
        raise ValueError("comparison requires schema 1 instruction reports")
    if not isinstance(data.get("metadata"), dict) or not isinstance(data.get("workloads"), dict):
        raise ValueError("invalid report shape")
    if set(data["workloads"]) != set(WORKLOADS):
        raise ValueError("reports must contain every fixed workload")
    for record in data["workloads"].values():
        samples = record.get("samples") if isinstance(record, dict) else None
        if not isinstance(samples, list) or len(samples) < 3 or len(samples) % 2 != 1:
            raise ValueError("expected at least three, odd-count samples per workload")
        if any(type(value) is not int or value <= 0 for value in samples):
            raise ValueError("instruction samples must be positive integers")
    return data


def compare(base: dict, head: dict, threshold: float) -> tuple[bool, str]:
    if not math.isfinite(threshold) or threshold < 0 or threshold > 100:
        raise ValueError("threshold must be finite and between 0 and 100 percent")
    for key in ["rustc", "platform", "machine", "valgrind", "workload_sha256", "profile"]:
        value = base["metadata"].get(key)
        if not isinstance(value, str) or not value.strip() or value != head["metadata"].get(key):
            raise ValueError(f"incompatible or missing measurement metadata: {key}")
    rows = ["| Workload | Base Ir | Head Ir | Change | Result |", "|---|---:|---:|---:|---|"]
    passed = True
    for name in WORKLOADS:
        before = statistics.median(base["workloads"][name]["samples"])
        after = statistics.median(head["workloads"][name]["samples"])
        change = (after - before) * 100 / before
        allowed = change <= threshold
        passed = passed and allowed
        rows.append(f"| {name} | {before} | {after} | {change:+.2f}% | {'PASS' if allowed else 'FAIL'} |")
    return passed, "\n".join(rows) + "\n"


def collect(args: argparse.Namespace) -> None:
    if args.repeats < 3 or args.repeats > 9 or args.repeats % 2 != 1:
        raise ValueError("repeats must be an odd integer from 3 to 9")
    if args.output.exists():
        raise ValueError("output already exists; use a fresh report path")
    if args.metric == "instructions" and sys.platform != "linux":
        raise ValueError("instruction collection requires Linux and Valgrind; it cannot fall back to wall time")
    binary = args.binary.resolve(strict=True)
    source = args.source.resolve(strict=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    valgrind = command(["valgrind", "--version"]) if args.metric == "instructions" else None
    report = {
        "schema": 1, "metric": args.metric, "revision": args.revision,
        "metadata": {
            "rustc": command(["rustc", "-Vv"]), "platform": platform.system(),
            "machine": platform.machine(), "valgrind": valgrind,
            "workload_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
            "profile": "release", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        },
        "workloads": {},
    }
    for name, symbol in WORKLOADS.items():
        samples = []
        for repeat in range(args.repeats):
            invocation = [str(binary), name]
            profile_path = args.output.parent / f"{name}-{repeat}.callgrind"
            if valgrind:
                invocation = ["valgrind", "--tool=callgrind", "--error-exitcode=99",
                              "--collect-atstart=no", f"--toggle-collect={symbol}",
                              f"--log-file={profile_path.resolve()}.log",
                              f"--callgrind-out-file={profile_path.resolve()}", *invocation]
            completed = json.loads(command(invocation))
            if not isinstance(completed, dict) or completed.get("completed") is not True or completed.get("workload") != name:
                raise ValueError(f"workload did not complete: {name}")
            count = instructions(profile_path.read_text(encoding="utf-8")) if valgrind else completed.get("elapsed_ns")
            if type(count) is not int or count <= 0:
                raise ValueError(f"invalid measurement for {name}")
            samples.append(count)
        report["workloads"][name] = {"samples": samples, "median": statistics.median(samples)}
        print(f"{name}: {statistics.median(samples)} {'Ir' if valgrind else 'ns'}", flush=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    measure = sub.add_parser("collect")
    measure.add_argument("--binary", type=Path, required=True)
    measure.add_argument("--source", type=Path, required=True)
    measure.add_argument("--output", type=Path, required=True)
    measure.add_argument("--revision", required=True)
    measure.add_argument("--metric", choices=["instructions", "walltime"], default="instructions")
    measure.add_argument("--repeats", type=int, default=3)
    gate = sub.add_parser("compare")
    gate.add_argument("--base", type=Path, required=True)
    gate.add_argument("--head", type=Path, required=True)
    gate.add_argument("--threshold", type=float, default=5)
    gate.add_argument("--summary", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.action == "collect":
            collect(args)
            return 0
        passed, summary = compare(read_report(args.base), read_report(args.head), args.threshold)
        args.summary.parent.mkdir(parents=True, exist_ok=True)
        args.summary.write_text(summary, encoding="utf-8")
        print(summary)
        return 0 if passed else 1
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"performance measurement failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
