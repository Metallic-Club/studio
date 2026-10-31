#!/usr/bin/env python3
"""Run a fixed, bounded matrix against an owned, prebuilt headless test executable."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ORDER = (1, 4, 4, 1, 1, 4)
EXPECTED_CASES = 4
CREATION_FLAGS = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0


def exit_status(code, windows):
    if code is None:
        return {"raw": None, "kind": "not_exited"}
    unsigned = code & 0xFFFFFFFF
    return {
        "raw": code,
        "signed_32": unsigned if unsigned < 0x80000000 else unsigned - 0x100000000,
        "hex_32": f"0x{unsigned:08x}",
        "kind": "signal" if not windows and code < 0 else "exit",
        "signal": -code if not windows and code < 0 else None,
        # A Windows status-like value alone is not proof of a native crash.
        "windows_status_like": windows and unsigned >= 0x80000000,
    }


def execute(command, timeout, environment):
    started = time.monotonic()
    child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment,
                             creationflags=CREATION_FLAGS)
    timed_out = False
    try:
        stdout, stderr = child.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        child.kill()
        stdout, stderr = child.communicate()  # Reap this owned child before another trial.
    except BaseException:
        child.kill()
        child.communicate()
        raise
    return child.returncode, timed_out, stdout, stderr, time.monotonic() - started


def matrix(executable, output, revision, timeout=120, environment=None, launch=execute):
    if not 0 < timeout <= 120:
        raise ValueError("timeout must be positive and at most 120 seconds")
    executable = Path(executable).resolve(strict=True)
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)  # Never overwrite earlier evidence.
    expected_hash = hashlib.sha256(executable.read_bytes()).hexdigest()
    environment = dict(os.environ if environment is None else environment)
    result = {
        "revision": revision,
        "executable": executable.name,
        "executable_sha256": expected_hash,
        "order": list(ORDER),
        "timeout_seconds": timeout,
        "test_backend_override": environment.get("METALLIC_RENDER_TEST_BACKEND"),
        "windows": os.name == "nt",
        "trials": [],
        "outcome": "incomplete",
    }
    manifest = output / "results.json"

    def save():
        manifest.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")

    save()
    for index, threads in enumerate(ORDER, 1):
        if hashlib.sha256(executable.read_bytes()).hexdigest() != expected_hash:
            result["outcome"] = "executable_changed"
            save()
            return 1
        command = [str(executable), f"--test-threads={threads}", "--nocapture"]
        code, timed_out, stdout, stderr, elapsed = launch(command, timeout, environment)
        prefix = f"{index:02d}-threads-{threads}"
        (output / f"{prefix}.stdout.log").write_bytes(stdout)
        (output / f"{prefix}.stderr.log").write_bytes(stderr)
        try:
            after_hash = hashlib.sha256(executable.read_bytes()).hexdigest()
        except OSError:
            after_hash = None
        changed = after_hash != expected_hash
        diagnostic_lines = stderr.decode("utf-8", errors="replace").splitlines()
        adapters = [line for line in diagnostic_lines if "[metallic-gpu]" in line and "phase=adapter.ready " in line]
        unavailable = sum("[metallic-gpu]" in line and "phase=context.unavailable " in line for line in diagnostic_lines)
        result["trials"].append({
            "threads": threads,
            "exit": exit_status(code, os.name == "nt"),
            "timed_out": timed_out,
            "elapsed_seconds": elapsed,
            "executable_sha256_after": after_hash,
            "executable_changed": changed,
            "gpu_contexts_ready": len(adapters),
            "unavailable_setups": unavailable,
            "adapters": adapters,
            "stdout": f"{prefix}.stdout.log",
            "stderr": f"{prefix}.stderr.log",
        })
        if timed_out or code != 0:
            result["outcome"] = "timeout" if timed_out else "child_failure"
            save()
            return 1  # Never conceal a failure with a later successful rerun.
        if changed:
            result["outcome"] = "executable_changed"
            save()
            return 1
        save()
    complete = all(t["gpu_contexts_ready"] == EXPECTED_CASES and not t["unavailable_setups"] for t in result["trials"])
    result["outcome"] = "no_crash_observed" if complete else "inconclusive_adapter_coverage"
    save()
    return 0 if complete else 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    revision = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True,
                              encoding="utf-8", check=True, timeout=10,
                              creationflags=CREATION_FLAGS).stdout.strip()
    changes = subprocess.run(["git", "status", "--porcelain"], cwd=root, capture_output=True,
                             encoding="utf-8", check=True, timeout=10,
                             creationflags=CREATION_FLAGS).stdout
    if changes.strip():
        parser.error("checkpoint the worktree before recording a source revision")
    return matrix(args.executable, args.output, revision)


if __name__ == "__main__":
    sys.exit(main())
