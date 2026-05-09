#!/usr/bin/env python3
"""Run the full graph-latency sweep matrix.

Iterates corpora × tools × phases × seeds and dispatches each cell to run.py.
Materializes a sweep order under runs/_sweeps/<sweep_id>/order.json so the
sweep is replayable and resumable.

This is a skeleton — the real harness is implemented alongside the first sweep.
"""
from __future__ import annotations

import argparse
import sys


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Run a graph-latency sweep.")
    p.add_argument("--version", default="v1", help="benchmark round (default: v1)")
    p.add_argument(
        "--corpora",
        nargs="*",
        default=None,
        help="subset of corpora to sweep (default: all in METHOD.md)",
    )
    p.add_argument(
        "--tools",
        nargs="*",
        default=None,
        help="subset of orbit.graph.* tools to sweep (default: all 9)",
    )
    p.add_argument(
        "--phases",
        nargs="*",
        default=["build-cold", "build-incremental", "query"],
        help="phases to sweep",
    )
    p.add_argument("--n", type=int, default=5, help="seeds per cell (default: 5)")
    p.add_argument("--sweep-id", default=None, help="sweep id (default: timestamp)")
    p.add_argument("--dry-run", action="store_true", help="print resolved order without executing")
    return p.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    print(
        "sweep.py skeleton — "
        f"version={args.version} corpora={args.corpora or '<all>'} "
        f"tools={args.tools or '<all>'} phases={args.phases} n={args.n} "
        f"dry_run={args.dry_run}"
    )
    print("TODO: materialize order.json under runs/_sweeps/<sweep_id>/ and dispatch run.py per cell.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
