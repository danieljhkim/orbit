#!/usr/bin/env python3
"""Aggregate graph-latency run records into the perf-RESULTS tables.

Reads <runs-dir>/<corpus>/<tool>/query/*.json   (query phase)
  and <runs-dir>/<corpus>/_build/<phase>/*.json (build-cold + build-incremental)

Emits two markdown tables matching the perf-RESULTS schema in CONVENTIONS.md:
  1. Primary table: corpus × tool × phase=query, p50/p90/p99 wall_ms.
  2. Build-phase table: corpus × phase ∈ {build-cold, build-incremental}.

Errored cells (timeouts, non-zero exits) are excluded from the percentile
computation but counted in a `errors` column so the failure is visible.
"""
from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Aggregate graph-latency run records.")
    p.add_argument("--runs", required=True, help="path to a vN/runs directory")
    p.add_argument(
        "--budgets",
        default=None,
        help="optional path to a YAML/JSON mapping (corpus,tool,phase) -> budget_ms",
    )
    p.add_argument(
        "--baseline",
        default=None,
        help="optional path to a prior version's runs/ for Delta vs v(N-1)",
    )
    p.add_argument("--format", default="markdown", choices=["markdown", "json"])
    return p.parse_args(argv)


def percentile(values: list[int], p: float) -> int:
    if not values:
        return 0
    if len(values) == 1:
        return values[0]
    sv = sorted(values)
    k = (len(sv) - 1) * p
    lo = math.floor(k)
    hi = math.ceil(k)
    if lo == hi:
        return sv[int(k)]
    frac = k - lo
    return int(sv[lo] + (sv[hi] - sv[lo]) * frac)


def load_records(runs_dir: Path) -> list[dict]:
    out: list[dict] = []
    for path in runs_dir.rglob("*.json"):
        if "_sweeps" in path.parts:
            continue
        try:
            out.append(json.loads(path.read_text()))
        except Exception as e:
            print(f"[warn] skipping {path}: {e}", file=sys.stderr)
    return out


def group_by(records: list[dict], *keys: str) -> dict[tuple, list[dict]]:
    out: dict[tuple, list[dict]] = {}
    for r in records:
        k = tuple(r.get(key) for key in keys)
        out.setdefault(k, []).append(r)
    return out


def render_query_table(records: list[dict]) -> str:
    query = [r for r in records if r.get("phase") == "query"]
    groups = group_by(query, "corpus", "tool")
    lines = [
        "| corpus | tool | runs | errors | p50_ms | p90_ms | p99_ms |",
        "|---|---|---:|---:|---:|---:|---:|",
    ]
    for (corpus, tool), rows in sorted(groups.items()):
        ok = [r for r in rows if not r.get("error")]
        errs = len(rows) - len(ok)
        wall = [int(r["wall_ms"]) for r in ok]
        lines.append(
            f"| {corpus} | {tool} | {len(rows)} | {errs} | "
            f"{percentile(wall, 0.50)} | {percentile(wall, 0.90)} | {percentile(wall, 0.99)} |"
        )
    return "\n".join(lines)


def render_build_table(records: list[dict]) -> str:
    build = [r for r in records if r.get("phase") in ("build-cold", "build-incremental")]
    groups = group_by(build, "corpus", "phase")
    lines = [
        "| corpus | phase | runs | errors | p50_ms | p90_ms | p99_ms | rss_p90_mb |",
        "|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for (corpus, phase), rows in sorted(groups.items()):
        ok = [r for r in rows if not r.get("error")]
        errs = len(rows) - len(ok)
        wall = [int(r["wall_ms"]) for r in ok]
        rss = [int(r.get("rss_peak_mb") or 0) for r in ok]
        lines.append(
            f"| {corpus} | {phase} | {len(rows)} | {errs} | "
            f"{percentile(wall, 0.50)} | {percentile(wall, 0.90)} | {percentile(wall, 0.99)} | "
            f"{percentile(rss, 0.90)} |"
        )
    return "\n".join(lines)


def render_errors(records: list[dict]) -> str:
    errs = [r for r in records if r.get("error")]
    if not errs:
        return "_No failed cells._"
    lines = ["| corpus | tool | phase | seed | error |", "|---|---|---|---:|---|"]
    for r in errs:
        lines.append(
            f"| {r.get('corpus')} | {r.get('tool') or '-'} | {r.get('phase')} | "
            f"{r.get('seed')} | `{(r.get('error') or '').replace('|', '/')[:120]}` |"
        )
    return "\n".join(lines)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    runs_dir = Path(args.runs)
    if not runs_dir.is_dir():
        raise SystemExit(f"runs dir not found: {runs_dir}")

    records = load_records(runs_dir)
    if not records:
        print("(no records yet — run scripts/sweep.py first)")
        return 0

    if args.format == "json":
        print(json.dumps(records, indent=2, sort_keys=True))
        return 0

    print("## Primary latency table (query phase)\n")
    print(render_query_table(records))
    print("\n## Build-phase table\n")
    print(render_build_table(records))
    print("\n## Failed cells\n")
    print(render_errors(records))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
