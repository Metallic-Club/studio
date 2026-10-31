#!/usr/bin/env python3
"""Fail if the CPU-only runtime's normal/build graph includes presentation crates."""
from pathlib import Path
import re
import subprocess
import sys

FORBIDDEN = {"metallic_render", "wgpu", "winit", "egui", "egui-wgpu", "eframe", "gilrs", "gilrs-core"}

def inspect(output: str) -> set[str]:
    packages = set()
    for line in output.splitlines():
        if not line.strip():
            continue
        match = re.match(r"^([A-Za-z0-9_-]+) v\S+(?: |$)", line)
        if match is None:
            raise ValueError("unrecognized cargo tree package line")
        packages.add(match[1])
    if "metallic_runtime" not in packages or not {"metallic_core", "metallic_physics", "metallic_script"} <= packages:
        raise ValueError("incomplete simulation dependency graph")
    found = sorted(name for name in packages if name in FORBIDDEN or name.startswith("wgpu-"))
    if found:
        raise ValueError("presentation dependencies in CPU graph: " + ", ".join(found))
    return packages

def main() -> int:
    result = subprocess.run(["cargo", "tree", "--locked", "-p", "metallic_runtime", "--no-default-features",
        "--edges", "normal,build", "--prefix", "none", "--format", "{p}", "--color", "never"],
        cwd=Path(__file__).resolve().parents[1], capture_output=True, encoding="utf-8", timeout=120)
    if result.returncode:
        print("simulation graph: cargo tree failed", file=sys.stderr)
        return 1
    try:
        packages = inspect(result.stdout)
    except ValueError as error:
        print(f"simulation graph: {error}", file=sys.stderr)
        return 1
    print(f"simulation graph: {len(packages)} normal/build packages; no presentation dependencies")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
