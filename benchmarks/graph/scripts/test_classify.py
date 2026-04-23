"""Unit tests for verdict classification.

Run with: python3 -m unittest benchmarks/graph/scripts/test_classify.py
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import classify
import run as run_module


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
    def test_no_graph_arm_without_graph_calls_is_ok(self):
        result = classify.classify_arm_enforcement(
            arm="no-graph",
            tool_calls_by_name={},
            permission_denials=[],
        )
        self.assertIsNone(result)

    def test_no_graph_arm_with_graph_call_is_error(self):
        result = classify.classify_arm_enforcement(
            arm="no-graph",
            tool_calls_by_name={"orbit.graph.search": 1},
            permission_denials=[],
        )
        self.assertIsNotNone(result)
        verdict, diag = result
        self.assertEqual(verdict, "error")
        self.assertIn("forbids graph navigation", diag)

    def test_graph_arm_with_zero_calls_and_no_denials_is_error(self):
        result = classify.classify_arm_enforcement(
            arm="graph-only",
            tool_calls_by_name={},
            permission_denials=[],
        )
        self.assertIsNotNone(result)
        verdict, diag = result
        self.assertEqual(verdict, "error")
        self.assertIn("zero graph calls", diag)

    def test_graph_arm_with_claude_graph_calls_is_ok(self):
        result = classify.classify_arm_enforcement(
            arm="graph-only",
            tool_calls_by_name={"mcp__orbit-bench__orbit_graph_search": 3},
            permission_denials=[],
        )
        self.assertIsNone(result)

    def test_graph_arm_with_codex_graph_calls_is_ok(self):
        result = classify.classify_arm_enforcement(
            arm="graph-only",
            tool_calls_by_name={"orbit.graph.search": 2},
            permission_denials=[],
        )
        self.assertIsNone(result)

    def test_graph_arm_with_only_denials_is_ok(self):
        # Agent tried a denied tool — that's a fail, not arm-not-enforced.
        result = classify.classify_arm_enforcement(
            arm="graph-only",
            tool_calls_by_name={},
            permission_denials=[{"tool": "Read"}],
        )
        self.assertIsNone(result)


class TestModelEscalation(unittest.TestCase):
    def test_opus_triggers_error(self):
        result = classify.classify_model_escalation(
            "claude",
            {"claude-sonnet-4-6": {}, "claude-opus-4-7": {}}
        )
        self.assertIsNotNone(result)
        verdict, diag = result
        self.assertEqual(verdict, "error")
        self.assertIn("opus", diag.lower())

    def test_sonnet_only_is_ok(self):
        result = classify.classify_model_escalation("claude", {"claude-sonnet-4-6": {}})
        self.assertIsNone(result)

    def test_sonnet_haiku_is_ok(self):
        result = classify.classify_model_escalation(
            "claude",
            {"claude-sonnet-4-6": {}, "claude-haiku-4-5-20251001": {}}
        )
        self.assertIsNone(result)

    def test_codex_model_usage_is_not_treated_as_escalation(self):
        result = classify.classify_model_escalation("codex", {"gpt-5.4": {}})
        self.assertIsNone(result)


class TestEndToEndClassify(unittest.TestCase):
    """Drive classify_run with provider-normalized shapes."""

    def test_claude_pass_classifies_as_pass(self):
        verdict, diag = classify.classify_run(
            provider="claude",
            arm="graph-only",
            run_result={
                "tool_calls": {"mcp__orbit-bench__orbit_graph_search": 2},
                "permission_denials": [],
                "model_usage": {"claude-sonnet-4-6": {}},
            },
            oracle_verdict="pass",
        )
        self.assertEqual(verdict, "pass")
        self.assertIn("oracle accepted", diag)

    def test_codex_no_graph_violation_classifies_as_error(self):
        verdict, diag = classify.classify_run(
            provider="codex",
            arm="no-graph",
            run_result={
                "tool_calls": {"exec_command": 2, "orbit.graph.search": 1},
                "permission_denials": [],
                "model_usage": {"gpt-5.4": {}},
            },
            oracle_verdict="pass",
        )
        self.assertEqual(verdict, "error")
        self.assertIn("forbids graph navigation", diag)


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


if __name__ == "__main__":
    unittest.main()
