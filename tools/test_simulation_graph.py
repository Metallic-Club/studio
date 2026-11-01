import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("graph",Path(__file__).with_name("check_simulation_graph.py"))
graph = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(graph)
BASE = "metallic_runtime v0.1.0 (workspace)\nmetallic_core v0.1.0\nmetallic_physics v0.1.0\nmetallic_script v0.1.0\n"

class GraphPolicy(unittest.TestCase):
    def test_cpu_graph_is_complete_and_presentation_transitives_are_rejected(self):
        self.assertEqual(len(graph.inspect(BASE+"rapier3d v0.36.1\n")),5)
        for package in graph.FORBIDDEN | {"wgpu-hal", "wgpu-core", "wgpu-types"}:
            with self.subTest(package=package), self.assertRaises(ValueError):
                graph.inspect(BASE+package+" v1.0.0\n")

    def test_empty_partial_and_unrecognized_output_fail_closed(self):
        for output in ["", "metallic_runtime v0.1.0\n", BASE+"not a package\n"]:
            with self.subTest(output=output), self.assertRaises(ValueError): graph.inspect(output)

if __name__ == "__main__": unittest.main()
