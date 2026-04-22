"""Verdict classification for benchmark runs.

Kept separate from run.py so it can be unit-tested without invoking
claude-code. Every function here is pure: `dict in -> (str, str) out`
(verdict, diagnostic).

Verdicts:
    error — harness / arm enforcement / escalation failure; the run
            should not be counted in pass_rate or tokens_per_success.
            The oracle is *not* consulted for error runs.
    pass  — run completed cleanly AND the fixture oracle accepted the
            final assistant message.
    fail  — run completed cleanly AND the oracle rejected the final
            assistant message.
"""

from __future__ import annotations

import re

# Any model whose tokens appear in `modelUsage` must match one of these
# regexes, otherwise the run is tainted by escalation (e.g. advisor
# routing to opus). Sonnet / Haiku 4.x are allowed because Claude Code's
# normal internal routing uses them for subagents and tool
# orchestration regardless of the --model flag.
INFRA_MODEL_PATTERNS = (
    re.compile(r"^claude-sonnet-4-\d+(-\d+)?$"),
    re.compile(r"^claude-haiku-4-\d+(-\d+)?$"),
)


def is_infra_model(name: str) -> bool:
    return any(p.match(name) for p in INFRA_MODEL_PATTERNS)


FS_NAV_TOOLS = ("Read", "Grep", "Glob")


def classify_arm_enforcement(
    arm: str,
    allowed_tools: list[str],
    tool_calls_by_name: dict[str, int],
    permission_denials: list,
) -> tuple[str, str] | None:
    """Return `(error, diagnostic)` if arm enforcement failed, else None.

    Fires ONLY when graph is the exclusive navigation surface (no Read /
    Grep / Glob in the allowlist) AND the agent made zero graph calls
    AND recorded no permission denials. That triad signals MCP-not-wired
    rather than an agent tool choice — the agent had nothing to
    navigate with and didn't even attempt a denied fallback.

    In `hybrid` mode, zero graph calls is a legitimate agent choice
    (the filesystem was available), so this check is skipped there.
    """
    graph_only = any(t.startswith("mcp__orbit-bench__") for t in allowed_tools) and not any(
        t in FS_NAV_TOOLS for t in allowed_tools
    )
    if not graph_only:
        return None
    graph_calls = sum(
        count
        for name, count in tool_calls_by_name.items()
        if name.startswith("mcp__orbit-bench__")
    )
    if graph_calls == 0 and not permission_denials:
        return (
            "error",
            f"arm '{arm}' is graph-exclusive but recorded zero "
            "mcp__orbit-bench__* calls and zero permission_denials — MCP "
            "server likely never connected in the child session",
        )
    return None


def classify_model_escalation(
    model_usage: dict,
) -> tuple[str, str] | None:
    """Return `(error, diagnostic)` if an off-allowlist model was used."""
    off = [name for name in model_usage.keys() if not is_infra_model(name)]
    if off:
        return (
            "error",
            f"model_usage includes non-infra model(s) {off!r} — advisor "
            "or subagent escalation fired; disable advisor and retry",
        )
    return None


def classify_run(
    *,
    arm: str,
    allowed_tools: list[str],
    claude_result: dict,
    oracle_verdict: str | None,
) -> tuple[str, str]:
    """End-to-end classification.

    `claude_result` is the parsed JSON from `claude -p --output-format
    json`. `oracle_verdict` is the pass/fail string from the fixture
    oracle, or None if the oracle was not run (e.g. pre-flight failed).
    Returns `(verdict, diagnostic)`.
    """
    if claude_result.get("is_error"):
        return (
            "error",
            f"claude -p reported is_error=True: "
            f"{claude_result.get('api_error_status')}",
        )

    model_usage = claude_result.get("modelUsage", {}) or {}
    escalation = classify_model_escalation(model_usage)
    if escalation:
        return escalation

    tool_calls = _extract_tool_calls(claude_result)
    denials = claude_result.get("permission_denials", []) or []
    enforcement = classify_arm_enforcement(arm, allowed_tools, tool_calls, denials)
    if enforcement:
        return enforcement

    if oracle_verdict is None:
        return ("error", "oracle did not run (pre-flight probe likely failed)")
    if oracle_verdict == "pass":
        return ("pass", "oracle accepted final message")
    return ("fail", f"oracle rejected final message: {oracle_verdict}")


def _extract_tool_calls(claude_result: dict) -> dict[str, int]:
    """Return {tool_name: count}. Claude -p JSON doesn't emit a flat
    histogram, so callers should prefer stream-json + their own
    histogram. For JSON output we return an empty dict, which means
    arm-enforcement check is a no-op and the run-driver is responsible
    for confirming enforcement through a separate channel (e.g. the
    pre-flight probe)."""
    return claude_result.get("tool_calls", {}) or {}
