"""Dependency-policy regressions; no compilation or GPU required."""
from pathlib import Path
import importlib.util
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("check_glam_graph", Path(__file__).with_name("check_glam_graph.py"))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)
CONSUMERS, check, inspect = guard.CONSUMERS, guard.check, guard.inspect


def graph(names, *versions):
    return "\n".join([*(name + " v0.1.1" for name in names),
                      *("glam v" + version for version in versions)])


class ActiveGraphTests(unittest.TestCase):
    def test_one_active_version_accepts_duplicate_tree_references(self):
        self.assertEqual(inspect("metallic_runtime v0.1.1\nglam v0.33.12\nglam v0.33.12 (*)",
                                 {"metallic_runtime"}), {"0.33.12"})

    def test_runtime_old_version_and_dev_only_old_version_are_rejected(self):
        for versions in [("0.30.10", "0.33.12"), ("0.15.2", "0.33.12"), ("0.30.10",)]:
            with self.subTest(versions=versions), self.assertRaisesRegex(ValueError, "active glam versions"):
                inspect(graph(CONSUMERS, *versions), CONSUMERS)

    def test_empty_partial_or_malformed_output_fails_closed(self):
        for output in ["", "glam v0.33.12", "metallic_runtime v0.1.1", "not a Cargo tree"]:
            with self.subTest(output=output), self.assertRaises(ValueError):
                inspect(output, {"metallic_runtime"})

    def test_cpu_graph_cannot_smuggle_presentation_dependencies(self):
        for name in ["metallic_render", "wgpu-core", "egui", "gilrs-core"]:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "presentation"):
                inspect(graph({"metallic_runtime", name}, "0.33.12"), {"metallic_runtime"}, cpu=True)

    def source(self, features):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        path = root / "crates/metallic_runtime/Cargo.toml"
        path.parent.mkdir(parents=True)
        path.write_text("[package]\nname='metallic_runtime'\nversion='0.1.1'\n" + features, encoding="utf-8")
        return root

    @staticmethod
    def execute(root, arguments):
        if "--manifest-path" in arguments:
            return graph({"style_shaders_check"}, "0.33.12")
        if "--no-default-features" in arguments:
            return graph({"metallic_runtime", "metallic_core", "metallic_physics", "metallic_script"}, "0.33.12")
        return graph(CONSUMERS, "0.33.12")

    def test_absent_cpu_split_is_unavailable_and_strict_mode_fails(self):
        root = self.source("")
        for required, status in [(False, "unavailable"), (True, "fail")]:
            reports = check(root, require_cpu=required, execute=self.execute)
            self.assertEqual(next(r for r in reports if r["graph"] == "cpu-runtime")["status"], status)
            self.assertEqual(sum(r["status"] == "pass" for r in reports), 3)

    def test_available_cpu_split_is_executed_not_inferred_from_metadata(self):
        root = self.source("[features]\npresentation=[]\n")
        calls = []

        def execute(root, arguments):
            calls.append(arguments)
            return self.execute(root, arguments)

        reports = check(root, require_cpu=True, execute=execute)
        self.assertEqual([r["status"] for r in reports], ["pass"] * 4)
        self.assertTrue(any("--no-default-features" in args for args in calls))
        self.assertTrue(any("normal,build,dev" in args for args in calls))
        self.assertTrue(any("--manifest-path" in args for args in calls))

    def test_dev_backend_cannot_be_hidden_by_a_clean_runtime_graph(self):
        def execute(root, arguments):
            if "normal,build,dev" in arguments:
                return graph(CONSUMERS, "0.33.12", "0.15.2")
            return self.execute(root, arguments)

        reports = check(self.source(""), execute=execute)
        self.assertEqual(next(r for r in reports if r["graph"] == "workspace-with-dev")["status"], "fail")
        self.assertEqual(next(r for r in reports if r["graph"] == "workspace-normal-build")["status"], "pass")

    def test_cargo_failure_does_not_become_an_empty_success(self):
        def execute(root, arguments):
            raise ValueError("Cargo tree failed")

        reports = check(self.source(""), execute=execute)
        self.assertEqual(sum(r["status"] == "fail" for r in reports), 3)


if __name__ == "__main__":
    unittest.main()
