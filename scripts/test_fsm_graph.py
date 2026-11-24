"""Integration checks; run in the Nix shell: python3 scripts/test_fsm_graph.py."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from fsm_graph import load_graph, overview_dot

ROOT = Path(__file__).resolve().parents[1]


class ExportTests(unittest.TestCase):
    def export(self, *extra, expected_code=0):
        temporary = tempfile.TemporaryDirectory(prefix="kvellox-export-test-")
        self.addCleanup(temporary.cleanup)
        prefix = Path(temporary.name) / "protocol"
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts/export-model.py"), "--requests", "1",
             "--render", "none", "--out", str(prefix), *extra],
            cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(result.returncode, expected_code, result.stdout + result.stderr)
        graph = json.loads(prefix.with_suffix(".json").read_text())
        return graph, prefix

    def test_exact_graph_and_cold_write_continuation(self):
        graph, prefix = self.export()
        self.assertTrue(graph["summary"]["complete"])
        # Same small instance checked by Veil in SlabIO.lean.
        self.assertEqual(graph["summary"]["states"], 95)
        states = [json.dumps(n["state"], sort_keys=True) for n in graph["nodes"]]
        self.assertEqual(len(states), len(set(states)))
        chains = [e for e in graph["edges"] if "chainReadToWrite" in graph["events"][e["event"]]["value"]]
        self.assertTrue(chains)
        for edge in chains:
            before = graph["nodes"][edge["source"]]["state"]
            after = graph["nodes"][edge["target"]]["state"]
            self.assertEqual(before["io"]["slots"][0]["kind"], "read")
            self.assertEqual(after["io"]["slots"][0]["kind"], "write")
            self.assertEqual(after["io"]["slots"][0]["stage"], "prepared")
            self.assertEqual(after["worker"]["requests"][0]["replies"], 0)
        self.assertIn("Projection:", Path(str(prefix) + ".overview.dot").read_text())
        self.assertIn("KvellModel/Protocol.lean", graph["source"]["sha256"])

    def test_state_limit_has_explicit_boundary_edges(self):
        graph, prefix = self.export("--max-states", "1")
        self.assertFalse(graph["summary"]["complete"])
        self.assertEqual(len(graph["nodes"]), 1)
        omitted = sum(e["target"] is None for e in graph["edges"])
        self.assertGreater(omitted, 0)
        self.assertEqual(graph["summary"]["omittedEdges"], omitted)
        self.assertIn("Omitted states", prefix.with_suffix(".dot").read_text())
        self.assertNotIn("accept", "\n".join(overview_dot(graph)))

    def test_depth_limit_marks_unexpanded_nodes(self):
        graph, _ = self.export("--max-depth", "1")
        self.assertFalse(graph["summary"]["complete"])
        self.assertTrue(graph["summary"]["depthLimited"])
        self.assertTrue(any(not n["expanded"] for n in graph["nodes"]))
        self.assertFalse(any(n["deadlock"] for n in graph["nodes"] if not n["expanded"]))

    def test_resource_starvation_is_reported_as_deadlock(self):
        graph, _ = self.export("--buffers", "0", expected_code=1)
        self.assertTrue(graph["summary"]["complete"])
        self.assertGreater(graph["summary"]["deadlockStates"], 0)
        self.assertTrue(any(n["deadlock"] for n in graph["nodes"]))

    def test_interrupted_stream_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "partial.jsonl"
            path.write_text('{"type":"header","schemaVersion":1}\n')
            with self.assertRaisesRegex(ValueError, "unfinished export"):
                load_graph(path)


if __name__ == "__main__":
    unittest.main()
