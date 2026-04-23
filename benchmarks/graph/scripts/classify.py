"""Verdict classification for benchmark runs.

Kept separate from run.py so it can be unit-tested without invoking a
child CLI. Every function here is pure: `dict in -> (str, str) out`
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

GRAPH_TOOL_PREFIXES = (
    "mcp__orbit-bench__orbit_graph_",
    "orbit.graph.",
)

# Any model whose tokens appear in `model_usage` must match one of these
# regexes, otherwise the Claude run is tainted by escalation (e.g.
# advisor routing to opus). Sonnet / Haiku 4.x are allowed because
# Claude Code's normal internal routing uses them for subagents and tool
# orchestration regardless of the --model flag.
INFRA_MODEL_PATTERNS = (
    re.compile(r"^claude-sonnet-4-\d+(-\d+)?$"),
    re.compile(r"^claude-haiku-4-\d+(-\d+)?$"),
)


def is_infra_model(name: str) -> bool:
    return any(p.match(name) for p in INFRA_MODEL_PATTERNS)


def classify_arm_enforcement(
    arm: str,
    tool_calls_by_name: dict[str, int],
    permission_denials: list,
) -> tuple[str, str] | None:
    """Return `(error, diagnostic)` if arm enforcement failed, else None.

    Phase 1 codifies only the enforcement we can observe reliably across
    providers:

    - `no-graph`: any graph call is an arm violation.
    - `graph-only`: zero graph calls and zero permission denials signals
      the graph surface was never used or never wired up.
    - `hybrid`: no enforcement; either surface is legitimate.
    """
    graph_calls = sum(
        count
        for name, count in tool_calls_by_name.items()
        if name.startswith(GRAPH_TOOL_PREFIXES)
    )

    if arm == "no-graph" and graph_calls:
        offenders = sorted(
            name for name in tool_calls_by_name if name.startswith(GRAPH_TOOL_PREFIXES)
        )
        return (
            "error",
            f"arm '{arm}' forbids graph navigation but observed graph tool call(s): "
            f"{offenders!r}",
        )

    if arm != "graph-only":
        return None

    if graph_calls == 0 and not permission_denials:
        return (
            "error",
            f"arm '{arm}' is graph-exclusive but recorded zero "
            "graph calls and zero permission_denials — graph tooling was "
            "likely never used or never connected in the child session",
        )
    return None


def classify_model_escalation(
    provider: str,
    model_usage: dict,
) -> tuple[str, str] | None:
    """Return `(error, diagnostic)` if an off-allowlist model was used."""
    if provider != "claude":
        return None
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
    provider: str,
    arm: str,
    run_result: dict,
    oracle_verdict: str | None,
) -> tuple[str, str]:
    """End-to-end classification for a provider-normalized run record."""
    if run_result.get("is_error"):
        detail = run_result.get("error_detail")
        return (
            "error",
            f"{provider} run reported is_error=True: {detail or 'unknown error'}",
        )

    model_usage = run_result.get("model_usage", {}) or {}
    escalation = classify_model_escalation(provider, model_usage)
    if escalation:
        return escalation

    tool_calls = run_result.get("tool_calls", {}) or {}
    denials = run_result.get("permission_denials", []) or []
    enforcement = classify_arm_enforcement(arm, tool_calls, denials)
    if enforcement:
        return enforcement

    if oracle_verdict is None:
        return ("error", "oracle did not run (pre-flight probe likely failed)")
    if oracle_verdict == "pass":
        return ("pass", "oracle accepted final message")
    return ("fail", f"oracle rejected final message: {oracle_verdict}")
