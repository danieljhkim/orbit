"""Sweep aggregator.

Reads benchmark records under `benchmarks/graph/<version>/runs/` and
emits two markdown tables to stdout: the primary `(provider, arm,
task_class)` headline table and the secondary `(provider, model, arm,
task_class)` per-model breakdown.

Defaults target the living version (v3); override with `--runs` /
`--tasks` or `GRAPH_VERSION=vN` to address a frozen snapshot.
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

import yaml

BENCH_ROOT = Path(__file__).resolve().parents[1]
GRAPH_VERSION = os.environ.get("GRAPH_VERSION", "v3")
VERSION_ROOT = BENCH_ROOT / GRAPH_VERSION
ARMS = {"no-graph", "graph-only", "hybrid"}
PROVIDERS = {"claude", "codex"}
CLAUDE_SHELL_OR_FS_TOOLS = {"Bash", "Glob", "Grep", "Read"}
GRAPH_TOOL_PREFIXES = ("mcp__orbit-bench__orbit_graph_", "orbit.graph.")
# For per-tool diagnostics, normalize tool names to a short kind.
GRAPH_TOOL_KINDS = (
    "callers",
    "refs",
    "implementors",
    "deps",
    "pack",
    "search",
    "show",
    "overview",
)
# Firehose threshold: run cost > THRESHOLD × same fixture's no-graph median
# tokens classifies the run as a payload-firehose failure regardless of
# verdict. v4 pre-registered value.
FIREHOSE_THRESHOLD = 5.0


@dataclass(frozen=True)
class ToolUtilization:
    graph_calls: int = 0
    shell_or_fs_calls: int = 0
    other_calls: int = 0


@dataclass
class GraphToolCallStats:
    """Per-tool-kind aggregate stats over a set of runs.

    `response_sizes` records the character length of each successful tool
    call's text response. We use char-length rather than token counts
    because the tool-call result payload doesn't carry token attribution
    (provider-side billing only reports per-turn). Char-length is a
    reasonable proxy for diagnosing payload-volume failures (e.g. the
    v3 `impact-tool-context-struct-literals` 12.43× firehose).
    """

    invocations: int = 0
    successes: int = 0
    failures: int = 0
    response_sizes: list[int] = field(default_factory=list)


@dataclass(frozen=True)
class FailureClassification:
    """Classification of a single non-passing run, or a passing run that
    nonetheless triggered a design-defect or firehose flag.

    Categories (per v4 METHOD.md §"Pre-registered report shape"):
      schema-coercion: at least one graph tool call failed mid-run
      payload-firehose: total tokens > 5x the same fixture's no-graph median
      wrong-tool: graph-only verdict=fail with at least one graph call
                  (graph used but couldn't solve)
      design-defect: hybrid verdict=pass with zero graph calls
                     (fixture wasn't actually graph-shaped)
      oracle-artifact: reserved; populated by manual audit during analysis
      none: classified passing run with no flags raised
    """

    schema_coercion: bool = False
    payload_firehose: bool = False
    wrong_tool: bool = False
    design_defect: bool = False

    def primary_label(self) -> str:
        """Single-label classification for the failure-taxonomy table."""
        # Order matters: the first true label wins. Schema-coercion is
        # most informative because it's an unambiguous schema-shape bug;
        # firehose is a cost diagnostic; wrong-tool is a capability gap;
        # design-defect is a fixture-design concern.
        if self.schema_coercion:
            return "schema-coercion"
        if self.payload_firehose:
            return "payload-firehose"
        if self.wrong_tool:
            return "wrong-tool"
        if self.design_defect:
            return "design-defect"
        return "none"


def _parse_jsonl_stream(raw: str) -> list[dict]:
    events = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict):
            events.append(event)
    return events


def _is_graph_tool_name(name: str) -> bool:
    return any(name.startswith(prefix) for prefix in GRAPH_TOOL_PREFIXES)


def _load_transcript_events(transcript_path: Path) -> list[dict] | None:
    if not transcript_path.exists():
        return None
    return _parse_jsonl_stream(transcript_path.read_text())


def _classify_claude_transcript(events: list[dict]) -> ToolUtilization:
    graph_calls = 0
    shell_or_fs_calls = 0
    other_calls = 0
    for event in events:
        if event.get("type") != "assistant":
            continue
        message = event.get("message", {}) or {}
        for block in message.get("content", []) or []:
            if block.get("type") != "tool_use":
                continue
            name = block.get("name")
            if not isinstance(name, str) or not name:
                other_calls += 1
            elif _is_graph_tool_name(name):
                graph_calls += 1
            elif name in CLAUDE_SHELL_OR_FS_TOOLS:
                shell_or_fs_calls += 1
            else:
                other_calls += 1
    return ToolUtilization(
        graph_calls=graph_calls,
        shell_or_fs_calls=shell_or_fs_calls,
        other_calls=other_calls,
    )


def _classify_codex_transcript(events: list[dict]) -> ToolUtilization:
    graph_calls = 0
    shell_or_fs_calls = 0
    other_calls = 0
    for event in events:
        if event.get("type") != "item.completed":
            continue
        item = event.get("item", {}) or {}
        item_type = item.get("type")
        if item_type == "command_execution":
            shell_or_fs_calls += 1
            continue
        if item_type != "mcp_tool_call":
            continue
        name = item.get("tool") or item.get("name")
        if not isinstance(name, str) or not name:
            other_calls += 1
        elif _is_graph_tool_name(name):
            graph_calls += 1
        else:
            other_calls += 1
    return ToolUtilization(
        graph_calls=graph_calls,
        shell_or_fs_calls=shell_or_fs_calls,
        other_calls=other_calls,
    )


def _classify_transcript(provider: str, transcript_path: Path) -> ToolUtilization | None:
    events = _load_transcript_events(transcript_path)
    if events is None:
        return None
    if provider == "claude":
        return _classify_claude_transcript(events)
    if provider == "codex":
        return _classify_codex_transcript(events)
    return ToolUtilization()


def _graph_tool_kind(name: str) -> str | None:
    """Map a tool name to a graph-tool kind ('callers', 'search', ...).

    Returns None for tool names that aren't graph tools.
    """
    if not isinstance(name, str):
        return None
    for prefix in GRAPH_TOOL_PREFIXES:
        if name.startswith(prefix):
            tail = name[len(prefix):]
            # Names like "orbit.graph.callers" or
            # "mcp__orbit-bench__orbit_graph_callers". Tail is e.g.
            # "callers" or "callers" — extract the last identifier.
            tail = tail.replace("orbit_graph_", "").lstrip("_.")
            tail = tail.split(".")[0].split("_")[0] if "_" not in tail else tail
            for kind in GRAPH_TOOL_KINDS:
                if tail == kind:
                    return kind
            return None
    return None


def _per_tool_stats_claude(events: list[dict]) -> dict[str, GraphToolCallStats]:
    """Per-graph-tool-kind stats from a claude transcript.

    Claude transcripts emit `tool_use` blocks (the call) and `tool_result`
    blocks (the response). We pair them by `id` to get per-call response
    size. Failure is signalled by the `is_error` flag on tool_result.
    """
    stats: dict[str, GraphToolCallStats] = defaultdict(GraphToolCallStats)
    pending: dict[str, str] = {}  # tool_use_id -> kind

    for event in events:
        if event.get("type") != "assistant":
            continue
        message = event.get("message", {}) or {}
        for block in message.get("content", []) or []:
            btype = block.get("type")
            if btype == "tool_use":
                kind = _graph_tool_kind(block.get("name", ""))
                if kind is None:
                    continue
                stats[kind].invocations += 1
                tool_id = block.get("id")
                if tool_id:
                    pending[tool_id] = kind

    # Now scan for tool_result blocks (claude emits these inside `user`
    # event content blocks).
    for event in events:
        if event.get("type") != "user":
            continue
        message = event.get("message", {}) or {}
        for block in message.get("content", []) or []:
            if block.get("type") != "tool_result":
                continue
            tool_id = block.get("tool_use_id")
            kind = pending.pop(tool_id, None)
            if kind is None:
                continue
            content = block.get("content", "")
            if isinstance(content, list):
                content = "".join(c.get("text", "") for c in content if isinstance(c, dict))
            elif not isinstance(content, str):
                content = str(content)
            if block.get("is_error"):
                stats[kind].failures += 1
            else:
                stats[kind].successes += 1
                stats[kind].response_sizes.append(len(content))
    return dict(stats)


def _per_tool_stats_codex(events: list[dict]) -> dict[str, GraphToolCallStats]:
    """Per-graph-tool-kind stats from a codex transcript.

    Codex emits `item.completed` events with `item.type == "mcp_tool_call"`
    where `status` is `"completed"` or `"failed"`. Each item carries the
    full `result` payload, so we can measure response size directly.
    """
    stats: dict[str, GraphToolCallStats] = defaultdict(GraphToolCallStats)
    for event in events:
        if event.get("type") != "item.completed":
            continue
        item = event.get("item", {}) or {}
        if item.get("type") != "mcp_tool_call":
            continue
        kind = _graph_tool_kind(item.get("tool", ""))
        if kind is None:
            continue
        stats[kind].invocations += 1
        if item.get("status") == "completed":
            stats[kind].successes += 1
            result = item.get("result", {}) or {}
            chars = 0
            for c in result.get("content", []) or []:
                t = c.get("text", "") if isinstance(c, dict) else ""
                chars += len(t)
            stats[kind].response_sizes.append(chars)
        else:
            stats[kind].failures += 1
    return dict(stats)


def _per_tool_stats(provider: str, transcript_path: Path) -> dict[str, GraphToolCallStats]:
    events = _load_transcript_events(transcript_path)
    if events is None:
        return {}
    if provider == "claude":
        return _per_tool_stats_claude(events)
    if provider == "codex":
        return _per_tool_stats_codex(events)
    return {}


def _format_graph_call_rate(graph_runs: int, total_runs: int) -> str:
    return f"{graph_runs}/{total_runs} = {graph_runs / total_runs:.1%}"


def _format_tool_utilization(cell_runs: list[dict]) -> tuple[str | int, str, str | int]:
    utilization = [r.get("_tool_utilization") for r in cell_runs]
    if any(stats is None for stats in utilization):
        return ("-", "N/A", "-")

    resolved = [stats for stats in utilization if stats is not None]
    graph_calls = sum(stats.graph_calls for stats in resolved)
    shell_or_fs_calls = sum(stats.shell_or_fs_calls for stats in resolved)
    graph_runs = sum(1 for stats in resolved if stats.graph_calls > 0)
    return (
        graph_calls,
        _format_graph_call_rate(graph_runs, len(resolved)),
        shell_or_fs_calls,
    )


def _total_other_calls(runs: list[dict]) -> int:
    total = 0
    for record in runs:
        stats = record.get("_tool_utilization")
        if stats is None:
            continue
        total += stats.other_calls
    return total


def _fixture_map(tasks_dir: Path) -> dict[str, dict]:
    fixtures = {}
    for p in tasks_dir.glob("*.yaml"):
        if p.stem.startswith("_"):
            continue
        fx = yaml.safe_load(p.read_text())
        fixtures[fx["task_id"]] = fx
    return fixtures


def _iter_arm_dirs(runs_dir: Path):
    for provider_dir in runs_dir.iterdir():
        if not provider_dir.is_dir() or provider_dir.name.startswith("_"):
            continue
        if provider_dir.name not in PROVIDERS:
            continue
        for arm_dir in provider_dir.iterdir():
            if arm_dir.is_dir() and arm_dir.name in ARMS:
                yield (provider_dir.name, arm_dir)


def load_runs(runs_dir: Path, tasks_dir: Path) -> list[dict]:
    fixtures = _fixture_map(tasks_dir)
    out = []
    for provider, arm_dir in _iter_arm_dirs(runs_dir):
        for task_dir in arm_dir.iterdir():
            if not task_dir.is_dir():
                continue
            for run_path in task_dir.glob("*.json"):
                if run_path.name.endswith(".transcript.json"):
                    continue
                try:
                    record = json.loads(run_path.read_text())
                except json.JSONDecodeError:
                    continue
                if not isinstance(record, dict) or "verdict" not in record:
                    continue
                record["provider"] = record.get("provider", provider)
                record["arm"] = record.get("arm", arm_dir.name)
                task_id = record.get("task_id", task_dir.name)
                fx = fixtures.get(task_id, {})
                record["_task_class"] = fx.get("class", "unknown")
                transcript_path = run_path.with_name(f"{run_path.stem}.transcript.json")
                record["_tool_utilization"] = _classify_transcript(
                    record["provider"],
                    transcript_path,
                )
                record["_per_tool_stats"] = _per_tool_stats(
                    record["provider"],
                    transcript_path,
                )
                out.append(record)
    # Compute per-fixture no-graph medians for firehose detection.
    no_graph_medians = _no_graph_medians(out)
    for record in out:
        record["_failure_classification"] = _classify_failure(record, no_graph_medians)
    return out


def _no_graph_medians(runs: list[dict]) -> dict[tuple[str, str], float]:
    """Median (input + output) tokens per (provider, task_id) for no-graph runs.

    Used as the baseline for firehose detection: a run on the same
    (provider, task_id) cell whose total tokens exceeds
    FIREHOSE_THRESHOLD * this median is flagged as a payload-firehose
    failure.
    """
    by_cell: dict[tuple[str, str], list[int]] = defaultdict(list)
    for r in runs:
        if r.get("verdict") == "error":
            continue
        if r.get("arm") != "no-graph":
            continue
        tot = r.get("input_tokens", 0) + r.get("output_tokens", 0)
        by_cell[(r["provider"], r.get("task_id", ""))].append(tot)
    medians: dict[tuple[str, str], float] = {}
    for cell, totals in by_cell.items():
        if totals:
            medians[cell] = statistics.median(totals)
    return medians


def _classify_failure(record: dict, no_graph_medians: dict[tuple[str, str], float]) -> FailureClassification:
    """Classify a run's failure mode (or flag passing runs with anomalies)."""
    if record.get("verdict") == "error":
        return FailureClassification()

    per_tool = record.get("_per_tool_stats") or {}
    failed_calls = sum(s.failures for s in per_tool.values())
    total_calls = sum(s.invocations for s in per_tool.values())

    schema_coercion = failed_calls > 0
    median = no_graph_medians.get((record["provider"], record.get("task_id", "")))
    total = record.get("input_tokens", 0) + record.get("output_tokens", 0)
    payload_firehose = bool(median and total > FIREHOSE_THRESHOLD * median)

    arm = record.get("arm")
    verdict = record.get("verdict")
    wrong_tool = arm == "graph-only" and verdict == "fail" and total_calls > 0
    design_defect = arm == "hybrid" and verdict == "pass" and total_calls == 0

    return FailureClassification(
        schema_coercion=schema_coercion,
        payload_firehose=payload_firehose,
        wrong_tool=wrong_tool,
        design_defect=design_defect,
    )


def primary_table(runs: list[dict]) -> str:
    cells: dict[tuple[str, str, str], list[dict]] = defaultdict(list)
    for r in runs:
        if r["verdict"] == "error":
            continue
        cells[(r["provider"], r["arm"], r["_task_class"])].append(r)

    rows = []
    for (provider, arm, cls), cell_runs in sorted(cells.items()):
        # `input_tokens + output_tokens` is the marginal (uncached) token
        # spend for the run, provider-comparable by convention — see the
        # module docstring in scripts/providers.py.
        totals = [r["input_tokens"] + r["output_tokens"] for r in cell_runs]
        passes = sum(1 for r in cell_runs if r["verdict"] == "pass")
        tps = (sum(totals) / max(1, passes)) if passes else float("inf")
        graph_calls, graph_call_rate, shell_or_fs_calls = _format_tool_utilization(cell_runs)
        rows.append(
            {
                "provider": provider,
                "arm": arm,
                "task_class": cls,
                "runs": len(cell_runs),
                "pass_rate": f"{passes / max(1, len(cell_runs)):.0%}",
                "median_total_tokens": int(statistics.median(totals)) if totals else 0,
                "p90_total_tokens": (
                    int(statistics.quantiles(totals, n=10)[-1])
                    if len(totals) >= 10
                    else (max(totals) if totals else 0)
                ),
                "tokens_per_success": f"{tps:.0f}" if tps != float("inf") else "∞",
                "graph_calls": graph_calls,
                "graph_call_rate": graph_call_rate,
                "shell_or_fs_calls": shell_or_fs_calls,
            }
        )
    return _render("Primary: provider × arm × task_class", rows)


def secondary_table(runs: list[dict]) -> str:
    cells: dict[tuple[str, str, str, str], dict] = defaultdict(
        lambda: {"cache_read_tokens": 0, "output_tokens": 0, "cost_usd": 0.0, "runs": 0}
    )
    for r in runs:
        if r["verdict"] == "error":
            continue
        for model, mu in (r.get("model_usage") or {}).items():
            k = (r["provider"], model, r["arm"], r["_task_class"])
            cells[k]["cache_read_tokens"] += mu.get("cache_read_tokens", 0)
            cells[k]["output_tokens"] += mu.get("output_tokens", 0)
            cells[k]["cost_usd"] += mu.get("cost_usd", 0.0)
            cells[k]["runs"] += 1
    rows = []
    for (provider, model, arm, cls), vals in sorted(cells.items()):
        rows.append(
            {
                "provider": provider,
                "model": model,
                "arm": arm,
                "task_class": cls,
                "runs": vals["runs"],
                "cache_read_tokens": vals["cache_read_tokens"],
                "output_tokens": vals["output_tokens"],
                "cost_usd": f"{vals['cost_usd']:.4f}",
            }
        )
    return _render("Secondary: provider × model × arm × task_class", rows)


def per_tool_table(runs: list[dict]) -> str:
    """Per-graph-tool diagnostic across the round.

    Aggregates `_per_tool_stats` across all runs (any arm) into a single
    row per tool kind: total invocations, success rate, failure count,
    and median response size in characters.
    """
    aggregate: dict[str, GraphToolCallStats] = defaultdict(GraphToolCallStats)
    for r in runs:
        if r.get("verdict") == "error":
            continue
        for kind, stats in (r.get("_per_tool_stats") or {}).items():
            agg = aggregate[kind]
            agg.invocations += stats.invocations
            agg.successes += stats.successes
            agg.failures += stats.failures
            agg.response_sizes.extend(stats.response_sizes)

    if not aggregate:
        return ""

    rows = []
    for kind in GRAPH_TOOL_KINDS:
        s = aggregate.get(kind)
        if s is None or s.invocations == 0:
            continue
        sizes = s.response_sizes
        rows.append(
            {
                "tool": kind,
                "invocations": s.invocations,
                "succeeded": s.successes,
                "failed": s.failures,
                "success_rate": (
                    f"{s.successes / s.invocations:.0%}" if s.invocations else "—"
                ),
                "median_response_chars": int(statistics.median(sizes)) if sizes else 0,
                "p90_response_chars": (
                    int(statistics.quantiles(sizes, n=10)[-1])
                    if len(sizes) >= 10
                    else (max(sizes) if sizes else 0)
                ),
            }
        )
    return _render("Per-tool diagnostic (graph tools, all arms)", rows)


def failure_classification_table(runs: list[dict]) -> str:
    """Failure-taxonomy table.

    For each non-passing run (and passing runs flagged with a design-defect
    or firehose anomaly), report the primary classification label.
    """
    counts: dict[tuple[str, str, str], int] = defaultdict(int)
    sample_paths: dict[tuple[str, str, str], list[str]] = defaultdict(list)
    for r in runs:
        if r.get("verdict") == "error":
            continue
        cls = r.get("_failure_classification")
        if cls is None or cls.primary_label() == "none":
            continue
        key = (r["provider"], r["arm"], cls.primary_label())
        counts[key] += 1
        if len(sample_paths[key]) < 3:
            sample_paths[key].append(f"{r.get('task_id','?')}:{r.get('seed','?')}")

    if not counts:
        return ""

    rows = []
    for (provider, arm, label), n in sorted(counts.items()):
        rows.append(
            {
                "provider": provider,
                "arm": arm,
                "failure_class": label,
                "runs": n,
                "sample": ", ".join(sample_paths[(provider, arm, label)]),
            }
        )
    return _render(
        "Failure taxonomy (non-passing or anomaly-flagged)", rows
    )


def error_table(runs: list[dict]) -> str:
    errs = [r for r in runs if r["verdict"] == "error"]
    if not errs:
        return ""
    rows = [
        {
            "provider": r["provider"],
            "arm": r["arm"],
            "task_id": r["task_id"],
            "seed": r["seed"],
            "diagnostic": (r.get("diagnostic") or "")[:80],
        }
        for r in errs
    ]
    return _render("Errors (excluded from aggregates)", rows)


def _render(title: str, rows: list[dict]) -> str:
    if not rows:
        return f"### {title}\n\n_(no runs)_\n"
    cols = list(rows[0].keys())
    header = "| " + " | ".join(cols) + " |"
    sep = "|" + "|".join("---" for _ in cols) + "|"
    body = "\n".join("| " + " | ".join(str(r[c]) for c in cols) + " |" for r in rows)
    return f"### {title}\n\n{header}\n{sep}\n{body}\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", default=str(VERSION_ROOT / "runs"))
    ap.add_argument("--tasks", default=str(VERSION_ROOT / "tasks"))
    args = ap.parse_args(argv)

    runs = load_runs(Path(args.runs), Path(args.tasks))
    if not runs:
        print("no runs found", file=sys.stderr)
        return 1

    print(primary_table(runs))
    print(secondary_table(runs))
    pt = per_tool_table(runs)
    if pt:
        print(pt)
    ft = failure_classification_table(runs)
    if ft:
        print(ft)
    err = error_table(runs)
    if err:
        print(err)
    other_calls = _total_other_calls(runs)
    if other_calls:
        print(
            (
                "warning: encountered "
                f"{other_calls} other tool-use events outside graph/filesystem/shell buckets"
            ),
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
