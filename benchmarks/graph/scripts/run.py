"""Single-cell benchmark run driver.

Invoked by run.sh. Spawns a child `claude -p` session with deterministic
arm enforcement, a fresh cold-cache nonce, and optional pre-flight
probe. Writes a canonical per-run record under benchmarks/graph/runs/.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
import uuid
from pathlib import Path

import classify
import oracle

BENCH_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = BENCH_ROOT.parent.parent
CLAUDE_BIN = os.environ.get("CLAUDE_BIN", "/Users/daniel/.local/bin/claude")
MCP_CONFIG = str(BENCH_ROOT / "mcp.json")

# Graph-tool names (with mcp.json's "orbit" server prefix).
GRAPH_TOOLS = [
    "mcp__orbit-bench__orbit_graph_search",
    "mcp__orbit-bench__orbit_graph_show",
    "mcp__orbit-bench__orbit_graph_callers",
    "mcp__orbit-bench__orbit_graph_implementors",
    "mcp__orbit-bench__orbit_graph_refs",
    "mcp__orbit-bench__orbit_graph_overview",
    "mcp__orbit-bench__orbit_graph_deps",
    "mcp__orbit-bench__orbit_graph_pack",
]


# Escape-hatch tools: Agent / Task / Skill let the model delegate to a
# subagent or invoke arbitrary registered skills (each of which may
# carry its own tool surface), bypassing the arm's declared surface.
# Always denied.
#
# Notably NOT in this list: ToolSearch — Claude Code's deferred-tools
# resolver, required for the agent to look up MCP tool schemas before
# calling them. Denying ToolSearch silently breaks MCP tool use.
ESCAPE_HATCHES = [
    "Agent", "Task", "Skill",
    "EnterPlanMode", "ExitPlanMode",
    "EnterWorktree", "ExitWorktree",
    "Monitor", "ScheduleWakeup",
    "SendMessage", "AskUserQuestion",
    "CronCreate", "CronDelete", "CronList",
    "TeamCreate", "TeamDelete",
    "TaskOutput", "TaskStop",
    "PushNotification", "RemoteTrigger",
    "NotebookEdit",
]

BASE_DENY = [
    "Bash", "Edit", "Write", "TodoWrite",
    "WebSearch", "WebFetch",
    *ESCAPE_HATCHES,
]


def parse_arm(arm: str) -> tuple[list[str], list[str]]:
    """Return (allowed, disallowed) tool lists."""
    base_fs = ["Read", "Grep", "Glob"]
    if arm == "no-graph":
        return (base_fs, BASE_DENY + GRAPH_TOOLS)
    if arm == "graph-only":
        return (GRAPH_TOOLS, BASE_DENY + base_fs)
    if arm == "hybrid":
        return (base_fs + GRAPH_TOOLS, BASE_DENY)
    raise SystemExit(f"unknown arm: {arm!r}")


ARM_STEER = {
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


def build_system_prompt_suffix(nonce: str, sweep_id: str, arm: str) -> str:
    """Fresh content per run — prevents prompt-cache hits on the system
    prompt suffix. `--append-system-prompt` is appended to the default
    system prompt, so any unique text here breaks cache alignment.

    Per-arm steer is included so each arm knows *which* surface to use —
    without this, graph-only runs hallucinate path answers because the
    agent doesn't spontaneously discover it needs graph tools."""
    steer = ARM_STEER.get(arm, "")
    return (
        f"\n\n<!-- benchmark-nonce: {nonce} sweep: {sweep_id} arm: {arm} -->\n"
        f"You are a benchmark subject. {steer} "
        f"Verify every claim with a tool call; do not answer from memory."
    )


def system_prompt_hash(suffix: str) -> str:
    return hashlib.sha256(suffix.encode()).hexdigest()[:16]


def run_claude(
    *,
    prompt: str,
    allowed: list[str],
    disallowed: list[str],
    system_suffix: str,
    nonce: str,
    sweep_id: str,
    model: str = "sonnet",
    budget_usd: float = 1.0,
    timeout_s: int = 600,
) -> tuple[int, dict | None, str, list[dict]]:
    """Return (exit_code, result_event_or_None, raw_stdout, all_events).

    Cache-busting strategy:
      1. `--exclude-dynamic-system-prompt-sections` moves per-machine
         bits (cwd, git status, memory paths) out of the system prompt
         and into the first user message. With that flag on, the
         system prompt prefix is IDENTICAL across runs → heavily
         cacheable but deterministic.
      2. We then prepend a unique `<run-nonce>` line to the USER
         prompt. Anthropic's cache hits by prefix; a different first
         user-message token forces a fresh cache entry. That's why
         `--append-system-prompt` alone was not enough — the suffix
         sits AFTER the cacheable system-prompt prefix, so a different
         suffix still hits the same cache boundary.
    """
    preamble = f"<run-nonce sweep={sweep_id} nonce={nonce} />\n\n"
    cmd = [
        CLAUDE_BIN,
        "-p",
        preamble + prompt,
        "--output-format", "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--exclude-dynamic-system-prompt-sections",
        "--max-budget-usd", str(budget_usd),
        "--model", model,
        "--mcp-config", MCP_CONFIG,
        "--strict-mcp-config",
        "--append-system-prompt", system_suffix,
        "--allowed-tools", " ".join(allowed),
        "--disallowed-tools", " ".join(disallowed),
    ]
    try:
        proc = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout_s, cwd=REPO_ROOT
        )
    except subprocess.TimeoutExpired as e:
        return (124, None, f"timeout after {timeout_s}s: {e}", [])
    raw = proc.stdout
    events = []
    result = None
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        events.append(ev)
        if ev.get("type") == "result":
            result = ev
    return (proc.returncode, result, raw, events)


def mcp_init_status(events: list[dict], server_name: str = "orbit-bench") -> tuple[bool, str]:
    """Parse the init event; return (ok, diagnostic). Free: no extra API call."""
    for ev in events:
        if ev.get("type") == "system" and ev.get("subtype") == "init":
            servers = ev.get("mcp_servers", []) or []
            match = next((s for s in servers if s.get("name") == server_name), None)
            if match is None:
                return (False, f"MCP server {server_name!r} not present in init event")
            status = match.get("status", "unknown")
            if status != "connected":
                return (False, f"MCP server {server_name!r} status={status!r} (expected 'connected')")
            return (True, f"MCP server {server_name!r} connected")
    return (False, "no init event in stream")


def count_tool_calls(events: list[dict]) -> dict[str, int]:
    """Count tool_use events by name across all assistant messages."""
    histogram: dict[str, int] = {}
    for ev in events:
        if ev.get("type") != "assistant":
            continue
        msg = ev.get("message", {}) or {}
        for block in msg.get("content", []) or []:
            if block.get("type") == "tool_use":
                name = block.get("name", "<unknown>")
                histogram[name] = histogram.get(name, 0) + 1
    return histogram


def normalize_model_usage(raw_usage: dict) -> dict:
    """Convert claude -p's camelCase modelUsage to snake_case so
    downstream tooling has a consistent shape."""
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


def preflight_probe(system_suffix: str) -> tuple[bool, str]:
    """Live probe: ask sonnet to call orbit_graph_overview and confirm
    it got a tool result. Costs ~$0.05–0.15/probe but catches the exact
    MCP-not-wired failure mode that kills real runs. Sonnet is chosen
    over haiku because haiku sometimes narrates instead of calling the
    tool (empirically observed 2026-04-22).

    The init-event `mcp_servers[].status` field cannot be used as a
    cheap substitute: `--mcp-config` servers show `status: disabled`
    until a tool from that server is invoked, so the init line is
    uninformative by itself."""
    probe_prompt = (
        "Call mcp__orbit-bench__orbit_graph_overview once with input "
        f'{{"format":"summary","workspace":"{REPO_ROOT}"}}. '
        "After the tool returns, reply with exactly: PROBE_OK."
    )
    exit_code, result, _raw, events = run_claude(
        prompt=probe_prompt,
        allowed=[
            "mcp__orbit-bench__orbit_graph_overview",
            "mcp__orbit-bench__orbit_graph_search",
        ],
        disallowed=["Read", "Grep", "Glob", "Bash", "Edit", "Write"],
        system_suffix=system_suffix,
        nonce=f"probe-{uuid.uuid4().hex[:8]}",
        sweep_id="probe",
        model="sonnet",
        budget_usd=0.25,
        timeout_s=90,
    )
    if exit_code != 0 or result is None:
        return (False, f"probe exit={exit_code} result_present={result is not None}")
    final = (result.get("result") or "").upper()
    if "PROBE_OK" not in final:
        # Not strictly required — the probe is satisfied by a successful
        # tool call, even if the model paraphrased. Check the histogram.
        calls = count_tool_calls(events)
        if any(n.startswith("mcp__orbit-bench__") for n in calls):
            return (True, f"tool call observed despite missing PROBE_OK sentinel: {calls}")
        return (False, f"probe made zero mcp__orbit-bench__* calls; final={final[:120]!r}")
    return (True, "PROBE_OK")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("arm", choices=["no-graph", "graph-only", "hybrid"])
    ap.add_argument("task_id")
    ap.add_argument("seed", type=int)
    ap.add_argument("--fixture", help="path to fixture YAML (default: tasks/<task_id>.yaml)")
    ap.add_argument("--no-probe", action="store_true", help="skip pre-flight probe")
    ap.add_argument("--budget", type=float, default=1.0, help="--max-budget-usd for main run")
    args = ap.parse_args()

    sweep_id = os.environ.get("SWEEP_ID", "adhoc")
    run_order_index = int(os.environ.get("RUN_ORDER_INDEX", "0"))
    nonce = os.environ.get("NONCE", str(uuid.uuid4()))

    fixture_path = args.fixture or str(BENCH_ROOT / "tasks" / f"{args.task_id}.yaml")
    fixture = oracle.load_fixture(fixture_path)

    allowed, disallowed = parse_arm(args.arm)
    system_suffix = build_system_prompt_suffix(nonce, sweep_id, args.arm)

    out_dir = BENCH_ROOT / "runs" / args.arm / args.task_id
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"{args.seed}.json"
    transcript_path = out_dir / f"{args.seed}.transcript.json"

    record: dict = {
        "arm": args.arm,
        "task_id": args.task_id,
        "seed": args.seed,
        "sweep_id": sweep_id,
        "run_order_index": run_order_index,
        "nonce": nonce,
        "system_prompt_hash": system_prompt_hash(system_suffix),
        "fixture_path": str(Path(fixture_path).relative_to(REPO_ROOT)),
        "allowed_tools": allowed,
        "disallowed_tools_head": disallowed[:5],
        "verdict": "error",
        "diagnostic": "not set",
        "wall_seconds": 0,
        "turns": 0,
        "input_tokens": 0,
        "cache_read_tokens": 0,
        "cache_creation_tokens": 0,
        "output_tokens": 0,
        "total_cost_usd": 0.0,
        "tool_calls": {},
        "model_usage": {},
        "permission_denials": [],
        "transcript_path": str(transcript_path.relative_to(REPO_ROOT)),
        "final_diff_path": None,
    }

    # ---- pre-flight probe (graph-enabled arms only) --------------------
    if not args.no_probe and any(t.startswith("mcp__orbit-bench__") for t in allowed):
        ok, diag = preflight_probe(system_suffix)
        if not ok:
            record["verdict"] = "error"
            record["diagnostic"] = f"pre-flight probe failed: {diag}"
            _write(record, out_path)
            print(json.dumps({"out": str(out_path), "verdict": "error", "diag": diag}))
            return 2

    # ---- main run -----------------------------------------------------
    prompt = fixture["prompt"]
    t0 = time.monotonic()
    exit_code, parsed, raw, events = run_claude(
        prompt=prompt,
        allowed=allowed,
        disallowed=disallowed,
        system_suffix=system_suffix,
        nonce=nonce,
        sweep_id=sweep_id,
        budget_usd=args.budget,
    )
    record["wall_seconds"] = round(time.monotonic() - t0, 2)

    transcript_path.write_text(raw)

    if parsed is None:
        record["verdict"] = "error"
        record["diagnostic"] = f"claude -p produced no result event (exit={exit_code})"
        _write(record, out_path)
        return 3

    usage = parsed.get("usage") or {}
    record["input_tokens"] = usage.get("input_tokens", 0)
    record["cache_read_tokens"] = usage.get("cache_read_input_tokens", 0)
    record["cache_creation_tokens"] = usage.get("cache_creation_input_tokens", 0)
    record["output_tokens"] = usage.get("output_tokens", 0)
    record["total_cost_usd"] = parsed.get("total_cost_usd", 0.0)
    record["turns"] = parsed.get("num_turns", 0)
    record["model_usage"] = normalize_model_usage(parsed.get("modelUsage", {}))
    record["permission_denials"] = parsed.get("permission_denials", [])
    record["tool_calls"] = count_tool_calls(events)

    # Stuff the tool-call histogram into the Claude result before
    # classify_run reads it (classify._extract_tool_calls looks for
    # `tool_calls` on the result dict).
    parsed["tool_calls"] = record["tool_calls"]

    final_message = parsed.get("result") or ""

    # Oracle evaluation only if the run itself looked clean.
    if parsed.get("is_error") or exit_code != 0:
        oracle_verdict = None
    else:
        verd, _rat = oracle.grade(fixture, final_message, sandbox=str(REPO_ROOT))
        oracle_verdict = verd

    verdict, diag = classify.classify_run(
        arm=args.arm,
        allowed_tools=allowed,
        claude_result=parsed,
        oracle_verdict=oracle_verdict,
    )
    record["verdict"] = verdict
    record["diagnostic"] = diag

    _write(record, out_path)
    print(
        json.dumps(
            {
                "out": str(out_path),
                "verdict": verdict,
                "cost": record["total_cost_usd"],
                "wall": record["wall_seconds"],
            }
        )
    )
    return 0 if verdict != "error" else 4


def _write(record: dict, path: Path) -> None:
    path.write_text(json.dumps(record, indent=2) + "\n")


if __name__ == "__main__":
    sys.exit(main())
