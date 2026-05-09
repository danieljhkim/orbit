#!/usr/bin/env python3
"""Run the graph-latency sweep matrix.

Iterates corpora × tools × phases × seeds and dispatches each cell to run.py.
Build phases are run once per (corpus, seed) — not per-tool — because they
have no tool dimension. Query phase runs once per (corpus, tool, seed).

Cells already on disk are skipped unless --force is passed, so the sweep is
resumable: if a build cell takes too long and you ctrl-c the sweep, re-running
it picks up from the next missing cell.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

import run as run_mod  # imports the sibling run.py


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Run a graph-latency sweep.")
    p.add_argument("--version", default="v1")
    p.add_argument("--corpora", nargs="*", default=None, help="subset (default: all)")
    p.add_argument("--tools", nargs="*", default=None, help="subset (default: all 9)")
    p.add_argument(
        "--phases",
        nargs="*",
        default=list(run_mod.PHASES),
        help="phases to sweep (default: all three)",
    )
    p.add_argument("--n", type=int, default=5, help="seeds per cell (default: 5)")
    p.add_argument("--sweep-id", default=None, help="sweep id (default: timestamp)")
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("--force", action="store_true", help="re-run cells whose record already exists")
    p.add_argument("--orbit-bin", default="orbit")
    return p.parse_args(argv)


def all_corpus_names(version: str) -> list[str]:
    bench_root = run_mod.find_bench_root()
    data = run_mod.load_yaml(bench_root / version / "corpora.yaml")
    return [c["name"] for c in data["corpora"]]


def cells(args: argparse.Namespace) -> list[tuple[str, str | None, str, int]]:
    """Materialize the sweep order as (corpus, tool, phase, seed) tuples."""
    corpora = args.corpora or all_corpus_names(args.version)
    tools = args.tools or list(run_mod.TOOLS)
    seeds = list(range(1, args.n + 1))
    out: list[tuple[str, str | None, str, int]] = []
    for corpus in corpora:
        for phase in args.phases:
            if phase == "query":
                for tool in tools:
                    for seed in seeds:
                        out.append((corpus, tool, phase, seed))
            else:
                for seed in seeds:
                    out.append((corpus, None, phase, seed))
    return out


def record_path(runs_dir: Path, corpus: str, tool: str | None, phase: str, seed: int) -> Path:
    if phase == "query":
        return runs_dir / corpus / tool / phase / f"{seed}.json"
    return runs_dir / corpus / "_build" / phase / f"{seed}.json"


def dispatch(cell: tuple[str, str | None, str, int], args: argparse.Namespace, runs_dir: Path) -> int:
    corpus, tool, phase, seed = cell
    cmd = [
        sys.executable,
        str(Path(__file__).parent / "run.py"),
        "--corpus", corpus,
        "--phase", phase,
        "--seed", str(seed),
        "--version", args.version,
        "--out-dir", str(runs_dir),
        "--orbit-bin", args.orbit_bin,
    ]
    if tool is not None:
        cmd += ["--tool", tool]
    print(f"[cell] {corpus} {tool or '-'} {phase} seed={seed}")
    proc = subprocess.run(cmd)
    return proc.returncode


def main(argv: list[str]) -> int:
    args = parse_args(argv)

    bench_root = run_mod.find_bench_root()
    runs_dir = bench_root / args.version / "runs"
    sweep_id = args.sweep_id or time.strftime("%Y%m%d-%H%M%S")
    sweep_dir = runs_dir / "_sweeps" / sweep_id

    plan = cells(args)
    print(f"[plan] {len(plan)} cells, sweep_id={sweep_id}")

    if args.dry_run:
        for cell in plan:
            corpus, tool, phase, seed = cell
            print(f"  {corpus} {tool or '-'} {phase} {seed}")
        return 0

    sweep_dir.mkdir(parents=True, exist_ok=True)
    (sweep_dir / "order.json").write_text(
        json.dumps(
            [
                {"corpus": c, "tool": t, "phase": p, "seed": s}
                for (c, t, p, s) in plan
            ],
            indent=2,
        ) + "\n"
    )

    failed: list[tuple] = []
    skipped = 0
    started = time.perf_counter()
    for i, cell in enumerate(plan, 1):
        corpus, tool, phase, seed = cell
        path = record_path(runs_dir, corpus, tool, phase, seed)
        if path.exists() and not args.force:
            skipped += 1
            continue
        rc = dispatch(cell, args, runs_dir)
        elapsed = time.perf_counter() - started
        rate = i / elapsed if elapsed > 0 else 0
        eta_s = (len(plan) - i) / rate if rate > 0 else 0
        print(f"  [{i}/{len(plan)}] rc={rc} elapsed={elapsed:.0f}s eta={eta_s:.0f}s")
        if rc != 0:
            failed.append(cell)

    print(f"[done] sweep_id={sweep_id} skipped={skipped} failed={len(failed)}")
    if failed:
        for c in failed:
            print(f"  fail: {c}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
