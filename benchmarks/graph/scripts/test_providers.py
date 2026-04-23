"""Unit tests for provider-specific benchmark helpers."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import providers


class TestCodexDenials(unittest.TestCase):
    def test_permission_marker_detection(self):
        self.assertTrue(
            providers._is_permission_denial_message(
                "error: store error: attempt to write a readonly database"
            )
        )
        self.assertTrue(
            providers._is_permission_denial_message("user cancelled MCP tool call")
        )
        self.assertFalse(
            providers._is_permission_denial_message(
                '{"code":"tool_not_found","error":"tool not found: orbit.graph.locate_symbol"}'
            )
        )
        self.assertFalse(
            providers._is_permission_denial_message(
                '{"code":"invalid_input","error":"missing trait_selector"}'
            )
        )

    def test_generic_command_failures_are_not_denials(self):
        events = [
            {
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "command": "orbit tool run orbit.graph.locate_symbol ...",
                    "status": "failed",
                    "exit_code": 1,
                    "aggregated_output": (
                        '{\n  "code": "tool_not_found",\n'
                        '  "error": "tool not found: orbit.graph.locate_symbol"\n}\n'
                    ),
                },
            },
            {
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "command": "orbit tool show orbit.graph.locate_symbol",
                    "status": "failed",
                    "exit_code": 1,
                    "aggregated_output": (
                        "error: store error: attempt to write a readonly database\n"
                    ),
                },
            },
        ]

        failures, denials = providers._codex_failures_and_denials(events)
        self.assertEqual(len(failures), 2)
        self.assertEqual(len(denials), 1)
        self.assertIn("readonly database", denials[0]["message"])

    def test_mcp_cancellation_counts_as_denial(self):
        events = [
            {
                "type": "item.completed",
                "item": {
                    "type": "mcp_tool_call",
                    "tool": "orbit.graph.search",
                    "status": "failed",
                    "error": {"message": "user cancelled MCP tool call"},
                },
            }
        ]

        failures, denials = providers._codex_failures_and_denials(events)
        self.assertEqual(len(failures), 1)
        self.assertEqual(len(denials), 1)
        self.assertEqual(denials[0]["tool"], "orbit.graph.search")


if __name__ == "__main__":
    unittest.main()
