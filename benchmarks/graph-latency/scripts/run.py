#!/usr/bin/env python3
"""Run a single graph-latency cell.

A cell is one (corpus × tool × phase × seed) measurement. The harness:
  1. Resolves the corpus path under ~/.cache/orbit-bench/<corpus>.
  2. For build phases: builds the index and times the build.
  3. For query phase: invokes the tool N times via MCP and records each call.
  4. Writes a record JSON conforming to the schema in METHOD.md.

This is a skeleton — the real harness is implemented alongside the first sweep.
"""
from __future__ import annotations

import argparse
import sys


PHASES = ("build-cold", "build-incremental", "query")
TOOLS = (
    "graph.overview",
    "graph.search",
    "graph.callers",
    "graph.deps",
    "graph.refs",
    "graph.show",
    "graph.implementors",
    "graph.history",
    "graph.pack",
)


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Run a single graph-latency cell.")
    p.add_argument("--corpus", required=True, help="corpus name, e.g. python-medium")
    p.add_argument("--tool", required=True, choices=TOOLS, help="orbit.graph.* tool to measure")
    p.add_argument("--phase", required=True, choices=PHASES, help="measurement phase")
    p.add_argument("--seed", required=True, type=int, help="seed for query selection")
    p.add_argument(
        "--query-shape",
        default="default",
        help="query-shape id from v<N>/tasks/<tool>.yaml; ignored for build phases",
    )
    p.add_argument("--version", default="v1", help="benchmark round (default: v1)")
    p.add_argument(
        "--out-dir",
        default=None,
        help="record output directory (default: benchmarks/graph-latency/<version>/runs)",
    )
    return p.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    print(
        "run.py skeleton — "
        f"version={args.version} corpus={args.corpus} tool={args.tool} "
        f"phase={args.phase} seed={args.seed} query_shape={args.query_shape}"
    )
    print("TODO: build/query the corpus via MCP and emit a record JSON.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
