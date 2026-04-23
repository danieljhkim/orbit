"""Sweep aggregator.

Reads benchmark records under `benchmarks/graph/runs/` and emits two
markdown tables to stdout: the primary `(provider, arm, task_class)`
headline table and the secondary `(provider, model, arm, task_class)`
per-model breakdown.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

import yaml

BENCH_ROOT = Path(__file__).resolve().parents[1]
ARMS = {"no-graph", "graph-only", "hybrid"}
PROVIDERS = {"claude", "codex"}


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
                out.append(record)
    return out


def primary_table(runs: list[dict]) -> str:
    cells: dict[tuple[str, str, str], list[dict]] = defaultdict(list)
    for r in runs:
        if r["verdict"] == "error":
            continue
        cells[(r["provider"], r["arm"], r["_task_class"])].append(r)

    rows = []
    for (provider, arm, cls), cell_runs in sorted(cells.items()):
        totals = [r["input_tokens"] + r["output_tokens"] for r in cell_runs]
        passes = sum(1 for r in cell_runs if r["verdict"] == "pass")
        tps = (sum(totals) / max(1, passes)) if passes else float("inf")
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


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", default=str(BENCH_ROOT / "runs"))
    ap.add_argument("--tasks", default=str(BENCH_ROOT / "tasks"))
    args = ap.parse_args()

    runs = load_runs(Path(args.runs), Path(args.tasks))
    if not runs:
        print("no runs found", file=sys.stderr)
        return 1

    print(primary_table(runs))
    print(secondary_table(runs))
    err = error_table(runs)
    if err:
        print(err)
    return 0


if __name__ == "__main__":
    sys.exit(main())
