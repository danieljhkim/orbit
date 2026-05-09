#!/usr/bin/env python3
"""Aggregate graph-latency runs into the primary tables for RESULTS.md.

Reads <runs-dir>/<corpus>/<tool>/<phase>/*.json, computes p50/p90/p99 wall_ms
per (corpus × tool × phase), and emits the primary latency table and the
build-phase table per the perf-RESULTS schema in CONVENTIONS.md.

This is a skeleton — the real aggregator is implemented alongside the first sweep.
"""
from __future__ import annotations

import argparse
import sys


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Aggregate graph-latency run records.")
    p.add_argument(
        "--runs",
        required=True,
        help="path to a vN/runs directory",
    )
    p.add_argument(
        "--budgets",
        default=None,
        help="optional path to a YAML mapping (corpus,tool,phase) -> budget_ms",
    )
    p.add_argument(
        "--format",
        default="markdown",
        choices=["markdown", "json", "csv"],
        help="output format (default: markdown)",
    )
    p.add_argument(
        "--baseline",
        default=None,
        help="optional path to a prior version's runs/ to compute Delta vs v(N-1)",
    )
    return p.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    print(
        "aggregate.py skeleton — "
        f"runs={args.runs} budgets={args.budgets or '<none>'} "
        f"format={args.format} baseline={args.baseline or '<none>'}"
    )
    print("TODO: compute p50/p90/p99 per (corpus,tool,phase) and emit primary + build tables.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
