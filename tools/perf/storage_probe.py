"""Retain raw storage research evidence and a compact table; never rewrite engine source."""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import statistics
import subprocess
from pathlib import Path

SCOPES = ["world_total", "named_btree", "id_btree", "id_sorted_vec"]
PARTS = [300, 5000]
DENSITIES = [0, 2, 7]


def command(args: list[str], root: Path) -> str:
    result = subprocess.run(args, cwd=root, capture_output=True, text=True, timeout=180)
    if result.returncode:
        raise ValueError(f"probe command failed: {result.stderr[-4000:]}")
    return result.stdout.strip()


def parse_memory(text: str) -> tuple[dict, list[dict]]:
    records = [json.loads(line) for line in text.splitlines() if line.strip()]
    if any(not isinstance(row, dict) for row in records):
        raise ValueError("probe rows must be objects")
    layouts = [row for row in records if row.get("kind") == "layout"]
    rows = [row for row in records if row.get("kind") == "memory"]
    if any(type(row.get("parts")) is not int or type(row.get("overrides")) is not int or not isinstance(row.get("scope"), str) for row in rows):
        raise ValueError("invalid memory matrix coordinates")
    expected = {(scope, parts, density) for scope in SCOPES for parts in PARTS for density in DENSITIES}
    keys = [(row.get("scope"), row.get("parts"), row.get("overrides")) for row in rows]
    if len(layouts) != 1 or len(rows) != len(expected) or set(keys) != expected or len(records) != len(rows) + 1:
        raise ValueError("memory matrix is incomplete or duplicated")
    for row in rows:
        for field in ["live_requested_bytes", "requested_bytes", "freed_bytes", "allocation_calls", "outer_stack_bytes"]:
            if type(row.get(field)) is not int or row[field] < 0:
                raise ValueError(f"invalid memory count: {field}")
        if row["requested_bytes"] - row["freed_bytes"] != row["live_requested_bytes"]:
            raise ValueError("retained bytes do not match allocated-minus-freed")
    for field in ["value_bytes", "id_value_entry_bytes", "named_header_bytes", "id_map_header_bytes", "sorted_header_bytes", "world_stack_bytes", "prototype_schema_requested_bytes"]:
        if type(layouts[0].get(field)) is not int or layouts[0][field] <= 0:
            raise ValueError(f"invalid layout count: {field}")
    return layouts[0], rows


def parse_cpu(text: str) -> list[dict]:
    rows = [json.loads(line) for line in text.splitlines() if line.strip()]
    expected = {(scope, parts, density, sample) for scope in SCOPES[1:] for parts in PARTS for density in DENSITIES for sample in range(6)}
    if any(not isinstance(row, dict) or row.get("kind") != "cpu_component" for row in rows):
        raise ValueError("invalid CPU row")
    if any(any(type(row.get(field)) is not int for field in ["parts", "overrides", "sample"]) or not isinstance(row.get("scope"), str) for row in rows):
        raise ValueError("invalid CPU matrix coordinates")
    keys = [(row.get("scope"), row.get("parts"), row.get("overrides"), row.get("sample")) for row in rows]
    if len(rows) != len(expected) or set(keys) != expected:
        raise ValueError("CPU sample matrix is incomplete or duplicated")
    for row in rows:
        if row.get("read_operations") != row["parts"] * 7 * 20 or row.get("write_operations") != row["parts"] * 20:
            raise ValueError("operation count differs from the fixed workload")
        if any(type(row.get(field)) is not int or row[field] <= 0 for field in ["read_ns", "write_ns"]):
            raise ValueError("invalid CPU duration")
    return rows


def table(layout: dict, memory: list[dict], cpu: list[dict]) -> str:
    lines = [
        "# Storage probe evidence", "",
        "Requested live heap includes retained capacities, not allocator headers or process RSS. World rows include services, Model, slots, children, free list, saturated journal and the ID vector. Component rows include only override containers and owned payloads; they must not be presented as whole-Part sizes.", "",
        "| Parts | Overrides | Scope | Live requested B | B per Part/container |",
        "|---:|---:|---|---:|---:|",
    ]
    for row in memory:
        lines.append(f"| {row['parts']} | {row['overrides']} | {row['scope']} | {row['live_requested_bytes']} | {row['live_requested_bytes']/row['parts']:.3f} |")
    lines += ["", f"On this build, Value is {layout['value_bytes']} B and an ID/Value pair is {layout['id_value_entry_bytes']} B. Seven independent owned Value payloads alone require {7 * layout['value_bytes']} B before keys/containers/world storage. Prototype shared defaults retain {layout['prototype_schema_requested_bytes']} requested B, excluded from each component row.", "",
              "## Component CPU samples", "", "Six rotated-order samples per case. These are component operations with pre-resolved IDs, not World API or frame timings. Concurrent-build noise prevents treating them as a controlled performance baseline.", "",
              "| Parts | Overrides | Scope | Read ns/op median [min,max] | Write ns/op median [min,max] |", "|---:|---:|---|---|---|"]
    for parts in PARTS:
        for density in DENSITIES:
            for scope in SCOPES[1:]:
                selected = [row for row in cpu if (row["parts"], row["overrides"], row["scope"]) == (parts, density, scope)]
                reads = [row["read_ns"] / row["read_operations"] for row in selected]
                writes = [row["write_ns"] / row["write_operations"] for row in selected]
                lines.append(f"| {parts} | {density} | {scope} | {statistics.median(reads):.2f} [{min(reads):.2f},{max(reads):.2f}] | {statistics.median(writes):.2f} [{min(writes):.2f},{max(writes):.2f}] |")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    if args.output.exists():
        raise ValueError("use a fresh evidence directory")
    binary = args.binary.resolve(strict=True)
    memory_text = command([str(binary), "memory"], root)
    layout, memory = parse_memory(memory_text)
    # Repeat deterministic accounting to catch polluted intervals before interpreting bytes.
    again_layout, again_memory = parse_memory(command([str(binary), "memory"], root))
    if layout != again_layout or memory != again_memory:
        raise ValueError("requested-heap accounting did not repeat exactly")
    cpu_text = command([str(binary), "cpu"], root)
    cpu = parse_cpu(cpu_text)
    sources = ["crates/metallic_core/examples/compact_storage.rs", "crates/metallic_core/examples/support/overrides.rs", "crates/metallic_core/examples/support/requested_heap.rs"]
    metadata = {
        "engine_revision": command(["git", "rev-parse", "HEAD"], root),
        "engine_source_diff": command(["git", "diff", "HEAD", "--", "crates/metallic_core/src"], root),
        "rustc": command(["rustc", "-Vv"], root), "platform": platform.platform(), "machine": platform.machine(),
        "build_command": "cargo build --release -p metallic_core --example compact_storage -j 4",
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "source_sha256": {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in sources},
        "conditions": "concurrent engine builds; requested-heap accounting repeated exactly; CPU samples diagnostic only",
    }
    args.output.mkdir(parents=True)
    (args.output / "memory.jsonl").write_text(memory_text + "\n", encoding="utf-8")
    (args.output / "cpu.jsonl").write_text(cpu_text + "\n", encoding="utf-8")
    (args.output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    (args.output / "tables.md").write_text(table(layout, memory, cpu), encoding="utf-8")
    print(args.output / "tables.md")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
