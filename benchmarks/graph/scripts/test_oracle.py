"""Tests for oracle.py — focused on the v4 structured grader.

The grep/cmd/judge graders are exercised indirectly via the v1/v2/v3 sweep
records and don't have dedicated tests. The structured grader is new for
v4 and has enough edge-case behaviour (JSON extraction, scoring modes,
deny-list) to warrant unit tests.

Run with:
    python3 -m unittest benchmarks.graph.scripts.test_oracle
"""

from __future__ import annotations

import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import oracle  # noqa: E402


def _spec(answer, **kwargs):
    """Build a structured-oracle spec with given ground-truth answer."""
    return {
        "item_kind": kwargs.pop("item_kind", "file_path"),
        "scoring": kwargs.pop("scoring", "exact_set"),
        "case_sensitive": kwargs.pop("case_sensitive", False),
        "ground_truth": {"answer": answer},
        **kwargs,
    }


class StructuredGraderTest(unittest.TestCase):
    def test_exact_match_passes(self):
        spec = _spec(["a/b.rs", "c/d.rs"])
        msg = '{"answer": ["a/b.rs", "c/d.rs"], "excluded": []}'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "pass")

    def test_missing_item_fails_exact_set(self):
        spec = _spec(["a/b.rs", "c/d.rs"])
        msg = '{"answer": ["a/b.rs"], "excluded": []}'
        verdict, rationale = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "fail")
        self.assertIn("missing", rationale)

    def test_extra_item_fails_exact_set(self):
        spec = _spec(["a/b.rs"])
        msg = '{"answer": ["a/b.rs", "extra.rs"], "excluded": []}'
        verdict, rationale = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "fail")
        self.assertIn("unexpected", rationale)

    def test_superset_ok_allows_extras(self):
        spec = _spec(["a/b.rs"], scoring="superset_ok")
        msg = '{"answer": ["a/b.rs", "extra.rs"], "excluded": []}'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "pass")

    def test_superset_ok_still_fails_on_missing(self):
        spec = _spec(["a/b.rs", "c/d.rs"], scoring="superset_ok")
        msg = '{"answer": ["a/b.rs"], "excluded": []}'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "fail")

    def test_case_insensitive_default(self):
        spec = _spec(["A/B.RS"])
        msg = '{"answer": ["a/b.rs"], "excluded": []}'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "pass")

    def test_case_sensitive_when_requested(self):
        spec = _spec(["A/B.RS"], case_sensitive=True)
        msg = '{"answer": ["a/b.rs"], "excluded": []}'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "fail")

    def test_extracts_from_fenced_block(self):
        spec = _spec(["a"])
        msg = 'Here is the answer:\n```json\n{"answer": ["a"], "excluded": []}\n```\nDone.'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "pass")

    def test_extracts_from_unlabeled_fence(self):
        spec = _spec(["a"])
        msg = 'Result:\n```\n{"answer": ["a"], "excluded": []}\n```'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "pass")

    def test_extracts_naked_json_at_end(self):
        spec = _spec(["a"])
        msg = 'Some prose then {"answer": ["a"], "excluded": []}'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "pass")

    def test_no_json_fails(self):
        spec = _spec(["a"])
        msg = "Just some prose, no JSON here."
        verdict, rationale = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "fail")
        self.assertIn("could not extract", rationale)

    def test_empty_answer_list(self):
        spec = _spec([])
        msg = '{"answer": [], "excluded": []}'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "pass")

    def test_deny_list_triggers_fail_even_on_exact_match(self):
        spec = _spec(
            ["crates/orbit-mcp/src/lib.rs"],
            deny_list=[
                {"pattern": "test/", "reason": "test files excluded"},
            ],
        )
        # Answer matches exact_set, but contains a forbidden substring.
        spec["ground_truth"]["answer"] = ["crates/orbit-mcp/src/lib.rs", "crates/foo/test/bar.rs"]
        msg = '{"answer": ["crates/orbit-mcp/src/lib.rs", "crates/foo/test/bar.rs"], "excluded": []}'
        verdict, rationale = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "fail")
        self.assertIn("deny_list", rationale)

    def test_deny_list_empty_allows_pass(self):
        spec = _spec(["a"], deny_list=[])
        msg = '{"answer": ["a"], "excluded": []}'
        verdict, _ = oracle._grade_structured(spec, msg)
        self.assertEqual(verdict, "pass")

    def test_dispatcher_routes_structured(self):
        fixture = {"oracle": {"structured": _spec(["a"])}}
        msg = '{"answer": ["a"], "excluded": []}'
        verdict, _ = oracle.grade(fixture, msg)
        self.assertEqual(verdict, "pass")

    def test_dispatcher_grep_still_works(self):
        # Backward-compat: existing grep oracle still routes.
        fixture = {"oracle": {"grep": {"must_include": ["foo"], "must_not_include": []}}}
        msg = "this contains foo"
        verdict, _ = oracle.grade(fixture, msg)
        self.assertEqual(verdict, "pass")


if __name__ == "__main__":
    unittest.main()
