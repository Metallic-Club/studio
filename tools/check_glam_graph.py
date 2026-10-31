#!/usr/bin/env python3
"""Check active Cargo trees for one shared glam version, without counting optional metadata."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = "0.33.12"
CONSUMERS = {"metallic_anim", "metallic_asset", "metallic_audio", "metallic_meshgen",
             "metallic_modeling", "metallic_render", "metallic_runtime", "metallic_studio"}
PRESENTATION = {"metallic_render", "wgpu", "winit", "egui", "egui-wgpu", "eframe", "gilrs", "gilrs-core"}


def inspect(output, roots, *, cpu=False):
    packages = set()
    versions = set()
    for line in output.splitlines():
        if not line.strip():
            continue
        match = re.match(r"^([A-Za-z0-9_-]+) v([^\s]+)(?: |$)", line)
        if match is None:
            raise ValueError("unrecognized Cargo tree package line")
        name, version = match.groups()
        packages.add(name)
        if name == "glam":
            versions.add(version)
    if not roots <= packages:
        raise ValueError("incomplete active graph: missing " + ", ".join(sorted(roots - packages)))
    if versions != {EXPECTED}:
        raise ValueError("active glam versions: " + (", ".join(sorted(versions)) or "none")
                         + "; expected only " + EXPECTED)
    if cpu:
        forbidden = sorted(name for name in packages if name in PRESENTATION or name.startswith("wgpu-"))
        if forbidden:
            raise ValueError("presentation dependencies in CPU graph: " + ", ".join(forbidden))
    return versions


def cargo_tree(root, arguments):
    command = ["cargo", "tree", "--locked", *arguments, "--prefix", "none", "--format", "{p}",
               "--color", "never"]
    result = subprocess.run(command, cwd=root, capture_output=True, encoding="utf-8", timeout=120,
                            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    if result.returncode:
        raise ValueError("Cargo tree failed; update the relevant lockfile deliberately before checking")
    return result.stdout


def check(root, *, require_cpu=False, execute=cargo_tree):
    reports = []
    specs = [
        ("workspace-normal-build", ["--workspace", "--edges", "normal,build"], CONSUMERS, False),
        ("workspace-with-dev", ["--workspace", "--edges", "normal,build,dev"], CONSUMERS, False),
        ("standalone-shader-checker", ["--manifest-path", "docs/research/style-shaders/check/Cargo.toml",
                                       "--edges", "normal,build"], {"style_shaders_check"}, False),
    ]
    manifest = tomllib.loads((root / "crates/metallic_runtime/Cargo.toml").read_text(encoding="utf-8"))
    if "presentation" in manifest.get("features", {}):
        specs.append(("cpu-runtime", ["-p", "metallic_runtime", "--no-default-features", "--edges", "normal,build"],
                      {"metallic_runtime", "metallic_core", "metallic_physics", "metallic_script"}, True))
    else:
        reports.append({"graph": "cpu-runtime", "status": "fail" if require_cpu else "unavailable",
                        "reason": "presentation/Simulation split is absent; this is not a CPU-only pass"})
    for name, arguments, roots, cpu in specs:
        try:
            versions = inspect(execute(root, arguments), roots, cpu=cpu)
            reports.append({"graph": name, "status": "pass", "active_glam": sorted(versions)})
        except (OSError, UnicodeError, ValueError, subprocess.TimeoutExpired) as error:
            reports.append({"graph": name, "status": "fail", "reason": str(error)})
    return reports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--require-cpu", action="store_true",
                        help="fail if this source does not yet provide the presentation/Simulation split")
    args = parser.parse_args()
    try:
        reports = check(ROOT, require_cpu=args.require_cpu)
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as error:
        print(json.dumps({"status": "fail", "reason": str(error)}))
        return 1
    print(json.dumps(reports, indent=2))
    return int(any(report["status"] == "fail" for report in reports))


if __name__ == "__main__":
    sys.exit(main())
