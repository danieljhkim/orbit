"""Unit tests for verdict classification.

Run with: python3 -m unittest benchmarks/graph/scripts/test_classify.py
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

import classify
import run as run_module

HERE = Path(__file__).resolve().parent
BENCH_ROOT = HERE.parent


class TestInfraModel(unittest.TestCase):
    def test_sonnet_allowed(self):
        self.assertTrue(classify.is_infra_model("claude-sonnet-4-6"))
        self.assertTrue(classify.is_infra_model("claude-sonnet-4-6-20251001"))

    def test_haiku_allowed(self):
        self.assertTrue(classify.is_infra_model("claude-haiku-4-5-20251001"))

    def test_opus_rejected(self):
        self.assertFalse(classify.is_infra_model("claude-opus-4-7"))
        self.assertFalse(classify.is_infra_model("claude-opus-4-6"))

    def test_unknown_rejected(self):
        self.assertFalse(classify.is_infra_model("gpt-5.4"))
        self.assertFalse(classify.is_infra_model(""))


class TestArmEnforcement(unittest.TestCase):
    def test_no_graph_arm_is_no_op(self):
        # no-graph arm has no mcp__orbit-bench__ in allowlist → enforcement check skipped
        result = classify.classify_arm_enforcement(
            arm="no-graph",
            allowed_tools=["Read", "Grep"],
            tool_calls_by_name={},
            permission_denials=[],
        )
        self.assertIsNone(result)

    def test_graph_arm_with_zero_calls_and_no_denials_is_error(self):
        result = classify.classify_arm_enforcement(
            arm="graph-only",
            allowed_tools=["mcp__orbit-bench__orbit_graph_search"],
            tool_calls_by_name={},
            permission_denials=[],
        )
        self.assertIsNotNone(result)
        verdict, diag = result
        self.assertEqual(verdict, "error")
        self.assertIn("mcp__orbit-bench__", diag)

    def test_graph_arm_with_calls_is_ok(self):
        result = classify.classify_arm_enforcement(
            arm="graph-only",
            allowed_tools=["mcp__orbit-bench__orbit_graph_search"],
            tool_calls_by_name={"mcp__orbit-bench__orbit_graph_search": 3},
            permission_denials=[],
        )
        self.assertIsNone(result)

    def test_graph_arm_with_only_denials_is_ok(self):
        # Agent tried a denied tool — that's a fail, not arm-not-enforced.
        result = classify.classify_arm_enforcement(
            arm="graph-only",
            allowed_tools=["mcp__orbit-bench__orbit_graph_search"],
            tool_calls_by_name={},
            permission_denials=[{"tool": "Read"}],
        )
        self.assertIsNone(result)


class TestModelEscalation(unittest.TestCase):
    def test_opus_triggers_error(self):
        result = classify.classify_model_escalation(
            {"claude-sonnet-4-6": {}, "claude-opus-4-7": {}}
        )
        self.assertIsNotNone(result)
        verdict, diag = result
        self.assertEqual(verdict, "error")
        self.assertIn("opus", diag.lower())

    def test_sonnet_only_is_ok(self):
        result = classify.classify_model_escalation({"claude-sonnet-4-6": {}})
        self.assertIsNone(result)

    def test_sonnet_haiku_is_ok(self):
        result = classify.classify_model_escalation(
            {"claude-sonnet-4-6": {}, "claude-haiku-4-5-20251001": {}}
        )
        self.assertIsNone(result)


class TestEndToEndClassify(unittest.TestCase):
    """Drive classify_run with shapes matching recorded raw.json."""

    def test_recorded_graph_only_run_2_classifies_as_opus_escalation(self):
        # Run 2 of the sanity-check set DID include opus in modelUsage
        # despite being advertised as "graph-only". Under the new harness
        # rules that's an error — exactly the signal we want.
        path = BENCH_ROOT / "runs" / "graph-only" / "locate-agentruntime" / "2.raw.json"
        if not path.exists():
            self.skipTest(f"fixture missing: {path}")
        parsed = json.loads(path.read_text())
        verdict, diag = classify.classify_run(
            arm="graph-only",
            allowed_tools=["mcp__orbit-bench__orbit_graph_search"],
            claude_result=parsed,
            oracle_verdict="pass",
        )
        self.assertEqual(verdict, "error")
        self.assertIn("opus", diag.lower())


class TestNonce(unittest.TestCase):
    """Cold-cache preamble: the suffix injected into --append-system-prompt
    must be unique per run, so two back-to-back runs yield distinct
    system-prompt hashes."""

    def test_distinct_nonces_yield_distinct_hashes(self):
        a = run_module.build_system_prompt_suffix("nonce-a", "sweep-1", "graph-only")
        b = run_module.build_system_prompt_suffix("nonce-b", "sweep-1", "graph-only")
        self.assertNotEqual(
            run_module.system_prompt_hash(a), run_module.system_prompt_hash(b)
        )

    def test_same_nonce_yields_same_hash(self):
        a = run_module.build_system_prompt_suffix("nonce-x", "sweep-1", "graph-only")
        b = run_module.build_system_prompt_suffix("nonce-x", "sweep-1", "graph-only")
        self.assertEqual(
            run_module.system_prompt_hash(a), run_module.system_prompt_hash(b)
        )


class TestArmFragmentPrefixes(unittest.TestCase):
    """Every MCP tool name declared in arms/*.json must start with
    mcp__orbit-bench__ so it matches the server name in mcp.json."""

    def test_arm_fragments_use_orbit_prefix(self):
        arms_dir = BENCH_ROOT / "arms"
        if not arms_dir.exists():
            self.skipTest(f"arms dir missing: {arms_dir}")
        import json as _json
        offenders = []
        for p in arms_dir.glob("*.json"):
            data = _json.loads(p.read_text())
            perms = data.get("permissions", {})
            for entry in perms.get("allow", []) + perms.get("deny", []):
                if entry.startswith("mcp__") and not entry.startswith("mcp__orbit-bench__"):
                    offenders.append((p.name, entry))
        self.assertFalse(
            offenders,
            f"arm fragments reference non-orbit MCP prefix: {offenders}",
        )


if __name__ == "__main__":
    unittest.main()
