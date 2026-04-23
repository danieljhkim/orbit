"""Provider-specific benchmark execution helpers.

This module owns all child-CLI details so run.py can stay focused on
benchmark orchestration.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import uuid
from dataclasses import dataclass
from pathlib import Path

CLAUDE_BIN = os.environ.get("CLAUDE_BIN", "/Users/daniel/.local/bin/claude")
CODEX_BIN = os.environ.get("CODEX_BIN", "codex")
DEFAULT_MODELS = {
    "claude": "sonnet",
    "codex": "gpt-5.4",
}

CLAUDE_GRAPH_TOOLS = (
    "mcp__orbit-bench__orbit_graph_search",
    "mcp__orbit-bench__orbit_graph_show",
    "mcp__orbit-bench__orbit_graph_callers",
    "mcp__orbit-bench__orbit_graph_implementors",
    "mcp__orbit-bench__orbit_graph_refs",
    "mcp__orbit-bench__orbit_graph_overview",
    "mcp__orbit-bench__orbit_graph_deps",
    "mcp__orbit-bench__orbit_graph_pack",
)

GRAPH_COMMAND_RE = re.compile(r"\borbit tool run (orbit\.graph\.[a-z]+)\b")

ESCAPE_HATCHES = (
    "Agent",
    "Task",
    "Skill",
    "EnterPlanMode",
    "ExitPlanMode",
    "EnterWorktree",
    "ExitWorktree",
    "Monitor",
    "ScheduleWakeup",
    "SendMessage",
    "AskUserQuestion",
    "CronCreate",
    "CronDelete",
    "CronList",
    "TeamCreate",
    "TeamDelete",
    "TaskOutput",
    "TaskStop",
    "PushNotification",
    "RemoteTrigger",
    "NotebookEdit",
)

BASE_DENY = (
    "Bash",
    "Edit",
    "Write",
    "TodoWrite",
    "WebSearch",
    "WebFetch",
    *ESCAPE_HATCHES,
)

CLAUDE_ARM_STEER = {
    "no-graph": (
        "You have filesystem navigation tools (Read, Grep, Glob) but not "
        "the orbit knowledge graph. Answer using the filesystem; verify "
        "by reading source files before stating locations."
    ),
    "graph-only": (
        "You have ONLY orbit knowledge-graph MCP tools "
        "(mcp__orbit-bench__orbit_graph_*). You do NOT have Read, Grep, or "
        "Glob. Answer by querying the graph — start with "
        "mcp__orbit-bench__orbit_graph_search. Do not guess paths; if the "
        "graph cannot answer, say so."
    ),
    "hybrid": (
        "You have both filesystem tools (Read, Grep, Glob) AND orbit "
        "knowledge-graph tools (mcp__orbit-bench__orbit_graph_*). Choose the "
        "tool best fit for each sub-question."
    ),
}

CODEX_ARM_STEER = {
    "no-graph": (
        "You have only a shell command tool for filesystem navigation. Use "
        "`rg`, `git grep`, `find`, `ls`, and focused `sed -n` reads. Do NOT "
        "run `orbit tool run orbit.graph.*` commands in this arm."
    ),
    "graph-only": (
        "You have only a shell command tool, and the intended navigation "
        "surface is Orbit knowledge-graph commands run from the repo root via "
        "`orbit tool run orbit.graph.*`. First inspect the available graph "
        "tools with `orbit tool list | rg 'orbit\\\\.graph'`. Prefer this flow: "
        "`orbit.graph.search` to find the symbol, `orbit.graph.show` on the "
        "returned selector, then `orbit.graph.implementors` with "
        "`{\"trait_selector\":\"symbol:...\"}` when you need implementors. "
        "Do not invent tool names like `orbit.graph.locate_symbol`, and avoid "
        "`orbit tool show orbit.graph.*` in this benchmark. Do NOT use `rg`, "
        "`git grep`, `find`, or broad file reads unless the graph says "
        "knowledge is unavailable."
    ),
    "hybrid": (
        "You have only a shell command tool. Use `orbit tool run "
        "orbit.graph.*` for symbol-oriented lookups and `rg` / `ls` / "
        "focused `sed -n` reads when filesystem inspection is the better fit. "
        "When using the graph, first inspect the available graph tools with "
        "`orbit tool list | rg 'orbit\\\\.graph'`, then prefer "
        "`orbit.graph.search` -> `orbit.graph.show` -> "
        "`orbit.graph.implementors` with a full `trait_selector`. Do not "
        "invent tool names like `orbit.graph.locate_symbol`, and avoid "
        "`orbit tool show orbit.graph.*` in this benchmark."
    ),
}

ARM_STEER_BY_PROVIDER = {
    "claude": CLAUDE_ARM_STEER,
    "codex": CODEX_ARM_STEER,
}


@dataclass(frozen=True)
class ProviderArmConfig:
    allowed: tuple[str, ...]
    disallowed: tuple[str, ...]


@dataclass(frozen=True)
class ProviderExecution:
    exit_code: int
    normalized_result: dict | None
    raw_stdout: str
    events: list[dict]


def build_system_prompt_suffix(
    nonce: str,
    sweep_id: str,
    arm: str,
    provider: str = "claude",
) -> str:
    steer = ARM_STEER_BY_PROVIDER.get(provider, {}).get(arm, "")
    return (
        f"\n\n<!-- benchmark-nonce: {nonce} sweep: {sweep_id} "
        f"provider: {provider} arm: {arm} -->\n"
        f"You are a benchmark subject. {steer} "
        f"Verify every claim with a tool call; do not answer from memory."
    )


def get_provider(
    name: str,
    *,
    repo_root: Path,
    mcp_config: str,
):
    if name == "claude":
        return ClaudeProvider(repo_root=repo_root, mcp_config=mcp_config)
    if name == "codex":
        return CodexProvider(repo_root=repo_root, mcp_config=mcp_config)
    raise SystemExit(f"unknown provider: {name!r}")


def _parse_jsonl_stream(raw: str) -> list[dict]:
    events = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return events


def _run_jsonl_subprocess(
    *,
    cmd: list[str],
    cwd: Path,
    timeout_s: int,
    input_text: str | None = None,
) -> tuple[int, str, list[dict]]:
    try:
        proc = subprocess.run(
            cmd,
            input=input_text,
            capture_output=True,
            text=True,
            timeout=timeout_s,
            cwd=cwd,
        )
    except subprocess.TimeoutExpired as error:
        return (124, f"timeout after {timeout_s}s: {error}", [])

    raw = proc.stdout
    return (proc.returncode, raw, _parse_jsonl_stream(raw))


def _is_graph_tool_name(name: str) -> bool:
    return name.startswith("mcp__orbit-bench__orbit_graph_") or name.startswith(
        "orbit.graph."
    )


def _graph_tool_name_from_command(command: str) -> str | None:
    match = GRAPH_COMMAND_RE.search(command)
    if match is None:
        return None
    return match.group(1)


def _build_codex_prompt(
    *,
    prompt: str,
    arm: str,
    nonce: str,
    sweep_id: str,
    system_suffix: str,
) -> str:
    scaffold = system_suffix.strip()
    return (
        f"<run-nonce sweep={sweep_id} nonce={nonce} provider=codex arm={arm} />\n\n"
        f"{scaffold}\n\n"
        f"Task:\n{prompt}"
    )


def _codex_overrides() -> list[str]:
    return [
        "default_tools_enabled=false",
        'enabled_tools=["exec_command"]',
        'approval_policy="never"',
    ]


def _is_permission_denial_message(message: str) -> bool:
    lowered = message.lower()
    markers = (
        "permission denied",
        "not allowed",
        "approval",
        "user cancelled",
        "read-only",
        "readonly",
        "sandbox",
        "access denied",
    )
    return any(marker in lowered for marker in markers)


def _count_claude_tool_calls(events: list[dict]) -> dict[str, int]:
    histogram: dict[str, int] = {}
    for event in events:
        if event.get("type") != "assistant":
            continue
        message = event.get("message", {}) or {}
        for block in message.get("content", []) or []:
            if block.get("type") == "tool_use":
                name = block.get("name", "<unknown>")
                histogram[name] = histogram.get(name, 0) + 1
    return histogram


def _codex_item_tool_names(item: dict) -> list[str]:
    item_type = item.get("type")
    if item_type == "command_execution":
        names = ["exec_command"]
        graph_name = _graph_tool_name_from_command(item.get("command", ""))
        if graph_name is not None:
            names.append(graph_name)
        return names
    if item_type == "mcp_tool_call":
        tool_name = item.get("tool")
        return [tool_name] if isinstance(tool_name, str) and tool_name else []
    return []


def _count_codex_tool_calls(events: list[dict]) -> dict[str, int]:
    histogram: dict[str, int] = {}
    for event in events:
        if event.get("type") != "item.started":
            continue
        item = event.get("item", {}) or {}
        for name in _codex_item_tool_names(item):
            histogram[name] = histogram.get(name, 0) + 1
    return histogram


def _codex_failures_and_denials(events: list[dict]) -> tuple[list[dict], list[dict]]:
    failures: list[dict] = []
    denials: list[dict] = []
    for event in events:
        if event.get("type") != "item.completed":
            continue
        item = event.get("item", {}) or {}
        item_type = item.get("type")
        if item_type == "command_execution":
            exit_code = item.get("exit_code")
            status = item.get("status")
            if status == "failed" or (exit_code not in (None, 0)):
                output = item.get("aggregated_output", "")
                failures.append(
                    {
                        "type": item_type,
                        "command": item.get("command"),
                        "exit_code": exit_code,
                        "status": status,
                        "output": output,
                    }
                )
                message = output.strip() or status or ""
                if _is_permission_denial_message(message):
                    denials.append(
                        {
                            "tool": "exec_command",
                            "message": message,
                        }
                    )
        elif item_type == "mcp_tool_call" and item.get("status") == "failed":
            error = item.get("error") or {}
            message = error.get("message", "")
            failures.append(
                {
                    "type": item_type,
                    "tool": item.get("tool"),
                    "status": item.get("status"),
                    "message": message,
                }
            )
            denial_message = message or item.get("status") or ""
            if _is_permission_denial_message(denial_message):
                denials.append(
                    {
                        "tool": item.get("tool", "<unknown>"),
                        "message": denial_message,
                    }
                )
    return (failures, denials)


def _codex_final_message(events: list[dict]) -> str:
    final = ""
    for event in events:
        if event.get("type") != "item.completed":
            continue
        item = event.get("item", {}) or {}
        if item.get("type") == "agent_message":
            final = item.get("text") or final
    return final


def _normalize_model_usage(raw_usage: dict) -> dict:
    out = {}
    for model, entry in (raw_usage or {}).items():
        out[model] = {
            "input_tokens": entry.get("inputTokens", 0),
            "cache_read_tokens": entry.get("cacheReadInputTokens", 0),
            "cache_creation_tokens": entry.get("cacheCreationInputTokens", 0),
            "output_tokens": entry.get("outputTokens", 0),
            "cost_usd": entry.get("costUSD", 0.0),
        }
    return out


def _normalize_claude_result(parsed: dict, events: list[dict], requested_model: str) -> dict:
    usage = parsed.get("usage") or {}
    return {
        "provider": "claude",
        "requested_model": requested_model,
        "is_error": bool(parsed.get("is_error")),
        "error_detail": parsed.get("api_error_status"),
        "final_message": parsed.get("result") or "",
        "turns": parsed.get("num_turns", 0),
        "input_tokens": usage.get("input_tokens", 0),
        "cache_read_tokens": usage.get("cache_read_input_tokens", 0),
        "cache_creation_tokens": usage.get("cache_creation_input_tokens", 0),
        "output_tokens": usage.get("output_tokens", 0),
        "total_cost_usd": parsed.get("total_cost_usd", 0.0),
        "model_usage": _normalize_model_usage(parsed.get("modelUsage", {})),
        "permission_denials": parsed.get("permission_denials", []),
        "tool_calls": _count_claude_tool_calls(events),
        "command_failures": [],
    }


def _normalize_codex_result(events: list[dict], requested_model: str, exit_code: int) -> dict:
    turn_completed = next(
        (event for event in reversed(events) if event.get("type") == "turn.completed"),
        None,
    )
    usage = (turn_completed or {}).get("usage", {}) or {}
    failures, denials = _codex_failures_and_denials(events)
    model_usage = {
        requested_model: {
            "input_tokens": usage.get("input_tokens", 0),
            "cache_read_tokens": usage.get("cached_input_tokens", 0),
            "cache_creation_tokens": 0,
            "output_tokens": usage.get("output_tokens", 0),
            "cost_usd": 0.0,
        }
    }
    return {
        "provider": "codex",
        "requested_model": requested_model,
        "is_error": exit_code != 0 or turn_completed is None,
        "error_detail": None if turn_completed is not None else "missing turn.completed event",
        "final_message": _codex_final_message(events),
        "turns": sum(1 for event in events if event.get("type") == "turn.completed"),
        "input_tokens": usage.get("input_tokens", 0),
        "cache_read_tokens": usage.get("cached_input_tokens", 0),
        "cache_creation_tokens": 0,
        "output_tokens": usage.get("output_tokens", 0),
        "total_cost_usd": 0.0,
        "model_usage": model_usage,
        "permission_denials": denials,
        "tool_calls": _count_codex_tool_calls(events),
        "command_failures": failures,
    }


class BenchmarkProvider:
    name: str
    default_model: str

    def __init__(self, *, repo_root: Path, mcp_config: str):
        self.repo_root = repo_root
        self.mcp_config = mcp_config

    def arm_config(self, arm: str) -> ProviderArmConfig:
        raise NotImplementedError

    def run(
        self,
        *,
        prompt: str,
        arm: str,
        arm_config: ProviderArmConfig,
        system_suffix: str,
        nonce: str,
        sweep_id: str,
        model: str,
        budget_usd: float = 1.0,
        timeout_s: int = 600,
    ) -> ProviderExecution:
        raise NotImplementedError

    def preflight_probe(
        self,
        *,
        arm: str,
        nonce: str,
        sweep_id: str,
        model: str,
    ) -> tuple[bool, str]:
        raise NotImplementedError


class ClaudeProvider(BenchmarkProvider):
    name = "claude"
    default_model = DEFAULT_MODELS["claude"]

    def arm_config(self, arm: str) -> ProviderArmConfig:
        base_fs = ("Read", "Grep", "Glob")
        if arm == "no-graph":
            return ProviderArmConfig(
                allowed=base_fs,
                disallowed=BASE_DENY + CLAUDE_GRAPH_TOOLS,
            )
        if arm == "graph-only":
            return ProviderArmConfig(
                allowed=CLAUDE_GRAPH_TOOLS,
                disallowed=BASE_DENY + base_fs,
            )
        if arm == "hybrid":
            return ProviderArmConfig(
                allowed=base_fs + CLAUDE_GRAPH_TOOLS,
                disallowed=BASE_DENY,
            )
        raise SystemExit(f"unknown arm: {arm!r}")

    def run(
        self,
        *,
        prompt: str,
        arm: str,
        arm_config: ProviderArmConfig,
        system_suffix: str,
        nonce: str,
        sweep_id: str,
        model: str,
        budget_usd: float = 1.0,
        timeout_s: int = 600,
    ) -> ProviderExecution:
        preamble = f"<run-nonce sweep={sweep_id} nonce={nonce} provider=claude />\n\n"
        cmd = [
            CLAUDE_BIN,
            "-p",
            preamble + prompt,
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--exclude-dynamic-system-prompt-sections",
            "--max-budget-usd",
            str(budget_usd),
            "--model",
            model,
            "--mcp-config",
            self.mcp_config,
            "--strict-mcp-config",
            "--append-system-prompt",
            system_suffix,
            "--allowed-tools",
            " ".join(arm_config.allowed),
            "--disallowed-tools",
            " ".join(arm_config.disallowed),
        ]
        exit_code, raw, events = _run_jsonl_subprocess(
            cmd=cmd,
            cwd=self.repo_root,
            timeout_s=timeout_s,
        )
        parsed = next((event for event in events if event.get("type") == "result"), None)
        normalized = None
        if parsed is not None:
            normalized = _normalize_claude_result(parsed, events, model)
        return ProviderExecution(
            exit_code=exit_code,
            normalized_result=normalized,
            raw_stdout=raw,
            events=events,
        )

    def preflight_probe(
        self,
        *,
        arm: str,
        nonce: str,
        sweep_id: str,
        model: str,
    ) -> tuple[bool, str]:
        probe_result = self.run(
            prompt=(
                "Call mcp__orbit-bench__orbit_graph_overview once with input "
                f'{{"format":"summary","workspace":"{self.repo_root}"}}. '
                "After the tool returns, reply with exactly: PROBE_OK."
            ),
            arm=arm,
            arm_config=ProviderArmConfig(
                allowed=(
                    "mcp__orbit-bench__orbit_graph_overview",
                    "mcp__orbit-bench__orbit_graph_search",
                ),
                disallowed=("Read", "Grep", "Glob", "Bash", "Edit", "Write"),
            ),
            system_suffix=build_system_prompt_suffix(
                nonce=f"probe-{uuid.uuid4().hex[:8]}",
                sweep_id="probe",
                arm=arm,
                provider=self.name,
            ),
            nonce=f"probe-{uuid.uuid4().hex[:8]}",
            sweep_id="probe",
            model=model,
            budget_usd=0.25,
            timeout_s=90,
        )
        normalized = probe_result.normalized_result
        if probe_result.exit_code != 0 or normalized is None:
            return (
                False,
                f"probe exit={probe_result.exit_code} result_present={normalized is not None}",
            )
        graph_calls = {
            name: count
            for name, count in (normalized.get("tool_calls") or {}).items()
            if _is_graph_tool_name(name)
        }
        final = (normalized.get("final_message") or "").upper()
        if "PROBE_OK" in final and graph_calls:
            return (True, "PROBE_OK")
        if graph_calls:
            return (True, f"graph tool call observed despite missing PROBE_OK: {graph_calls}")
        return (False, f"probe made zero graph-tool calls; final={final[:120]!r}")


class CodexProvider(BenchmarkProvider):
    name = "codex"
    default_model = DEFAULT_MODELS["codex"]

    def arm_config(self, arm: str) -> ProviderArmConfig:
        if arm not in CODEX_ARM_STEER:
            raise SystemExit(f"unknown arm: {arm!r}")
        return ProviderArmConfig(allowed=("exec_command",), disallowed=())

    def run(
        self,
        *,
        prompt: str,
        arm: str,
        arm_config: ProviderArmConfig,
        system_suffix: str,
        nonce: str,
        sweep_id: str,
        model: str,
        budget_usd: float = 1.0,
        timeout_s: int = 600,
    ) -> ProviderExecution:
        del arm_config, budget_usd
        cmd = [
            CODEX_BIN,
            "exec",
            "--json",
            "--ephemeral",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--cd",
            str(self.repo_root),
            "--model",
            model,
        ]
        for override in _codex_overrides():
            cmd.extend(["-c", override])
        cmd.append("-")

        exit_code, raw, events = _run_jsonl_subprocess(
            cmd=cmd,
            cwd=self.repo_root,
            timeout_s=timeout_s,
            input_text=_build_codex_prompt(
                prompt=prompt,
                arm=arm,
                nonce=nonce,
                sweep_id=sweep_id,
                system_suffix=system_suffix,
            ),
        )
        normalized = None
        if events:
            normalized = _normalize_codex_result(events, model, exit_code)
        return ProviderExecution(
            exit_code=exit_code,
            normalized_result=normalized,
            raw_stdout=raw,
            events=events,
        )

    def preflight_probe(
        self,
        *,
        arm: str,
        nonce: str,
        sweep_id: str,
        model: str,
    ) -> tuple[bool, str]:
        probe_result = self.run(
            prompt=(
                "Run `orbit tool run orbit.graph.overview --output text` exactly once "
                "with the shell tool. After the command returns successfully, reply "
                "with exactly: PROBE_OK."
            ),
            arm=arm,
            arm_config=self.arm_config(arm),
            system_suffix=build_system_prompt_suffix(
                nonce=f"probe-{nonce}",
                sweep_id=f"{sweep_id}-probe",
                arm=arm,
                provider=self.name,
            ),
            nonce=f"probe-{nonce}",
            sweep_id=f"{sweep_id}-probe",
            model=model,
            timeout_s=90,
        )
        normalized = probe_result.normalized_result
        if probe_result.exit_code != 0 or normalized is None:
            return (
                False,
                f"probe exit={probe_result.exit_code} result_present={normalized is not None}",
            )
        graph_calls = {
            name: count
            for name, count in (normalized.get("tool_calls") or {}).items()
            if _is_graph_tool_name(name)
        }
        if normalized.get("command_failures"):
            return (False, f"probe command failures: {normalized['command_failures']}")
        final = (normalized.get("final_message") or "").upper()
        if "PROBE_OK" in final and graph_calls:
            return (True, "PROBE_OK")
        if graph_calls:
            return (True, f"graph command observed despite missing PROBE_OK: {graph_calls}")
        return (False, f"probe made zero graph-tool calls; final={final[:120]!r}")
