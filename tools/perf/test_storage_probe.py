"""Reject partial, duplicated or arithmetically inconsistent research evidence."""
import copy
import json
import unittest

import storage_probe as probe


def memory_rows():
    return [{"kind": "layout", **{key: 40 for key in ["value_bytes", "id_value_entry_bytes", "named_header_bytes", "id_map_header_bytes", "sorted_header_bytes", "world_stack_bytes", "prototype_schema_requested_bytes"]}}] + [
        {"kind": "memory", "scope": scope, "parts": parts, "overrides": density,
         "live_requested_bytes": 100, "requested_bytes": 120, "freed_bytes": 20,
         "allocation_calls": 2, "outer_stack_bytes": 24}
        for scope in probe.SCOPES for parts in probe.PARTS for density in probe.DENSITIES
    ]


def cpu_rows():
    return [{"kind": "cpu_component", "scope": scope, "parts": parts, "overrides": density,
             "sample": sample, "read_operations": parts * 7 * 20, "write_operations": parts * 20,
             "read_ns": 100, "write_ns": 200}
            for scope in probe.SCOPES[1:] for parts in probe.PARTS for density in probe.DENSITIES for sample in range(6)]


def encoded(rows):
    return "\n".join(json.dumps(row) for row in rows)


class EvidenceIntegrity(unittest.TestCase):
    def test_complete_memory_and_cpu_matrices_are_retained(self):
        layout, rows = probe.parse_memory(encoded(memory_rows()))
        self.assertEqual(len(rows), 24)
        self.assertEqual(layout["value_bytes"], 40)
        self.assertEqual(len(probe.parse_cpu(encoded(cpu_rows()))), 108)

    def test_memory_omission_duplicates_and_false_retained_sizes_are_rejected(self):
        rows = memory_rows()
        invalid = [rows[:-1], rows + [rows[-1]], rows[1:]]
        for field, value in [("live_requested_bytes", 1), ("requested_bytes", True), ("parts", "5000")]:
            changed = copy.deepcopy(rows)
            changed[-1][field] = value
            invalid.append(changed)
        for case in invalid:
            with self.subTest(case=case[-1]), self.assertRaises(ValueError):
                probe.parse_memory(encoded(case))

    def test_cpu_partial_samples_wrong_operations_and_zero_times_are_rejected(self):
        rows = cpu_rows()
        invalid = [rows[:-1], rows + [rows[-1]]]
        for field, value in [("read_operations", 1), ("write_ns", 0), ("sample", True)]:
            changed = copy.deepcopy(rows)
            changed[-1][field] = value
            invalid.append(changed)
        for case in invalid:
            with self.subTest(case=case[-1]), self.assertRaises(ValueError):
                probe.parse_cpu(encoded(case))


if __name__ == "__main__":
    unittest.main()
