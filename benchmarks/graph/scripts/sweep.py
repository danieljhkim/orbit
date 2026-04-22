"""Sweep orchestrator.

Enumerates all (arm, task_id, seed) cells, shuffles with a recorded
sweep-seed, writes order.json, and invokes run.sh for each cell in
shuffled order.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import subprocess
import sys
import time
import uuid
from pathlib import Path

BENCH_ROOT = Path(__file__).resolve().parents[1]
RUN_SH = BENCH_ROOT / "scripts" / "run.sh"


def discover_tasks(task_dir: Path) -> list[str]:
    return sorted(
        p.stem
        for p in task_dir.glob("*.yaml")
        if not p.stem.startswith("_")
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--arms", nargs="+", default=["no-graph", "graph-only", "hybrid"])
    ap.add_argument("--tasks", nargs="*", help="task_ids (default: all fixtures)")
    ap.add_argument("--n", type=int, default=5, help="runs per cell")
    ap.add_argument("--sweep-seed", type=int, help="shuffle seed (random if omitted)")
    ap.add_argument("--sweep-id", default=None)
    ap.add_argument("--budget-ceiling", type=float, default=25.0,
                    help="abort if total spend exceeds this (USD)")
    ap.add_argument("--dry-run", action="store_true",
                    help="write order.json and print the plan; do not execute runs")
    args = ap.parse_args()

    task_ids = args.tasks or discover_tasks(BENCH_ROOT / "tasks")
    if not task_ids:
        print("no fixtures under benchmarks/graph/tasks/", file=sys.stderr)
        return 1

    sweep_seed = args.sweep_seed if args.sweep_seed is not None else random.randrange(1 << 30)
    sweep_id = args.sweep_id or time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:6]

    cells = [
        (arm, task_id, seed)
        for arm in args.arms
        for task_id in task_ids
        for seed in range(1, args.n + 1)
    ]
    rng = random.Random(sweep_seed)
    rng.shuffle(cells)

    sweep_dir = BENCH_ROOT / "runs" / "_sweeps" / sweep_id
    sweep_dir.mkdir(parents=True, exist_ok=True)
    order_path = sweep_dir / "order.json"
    order_path.write_text(
        json.dumps(
            {
                "sweep_id": sweep_id,
                "sweep_seed": sweep_seed,
                "arms": args.arms,
                "tasks": task_ids,
                "n": args.n,
                "order": [{"arm": a, "task_id": t, "seed": s} for (a, t, s) in cells],
            },
            indent=2,
        )
        + "\n"
    )
    print(f"sweep_id={sweep_id} seed={sweep_seed} cells={len(cells)} → {order_path}")

    if args.dry_run:
        return 0

    total_spend = 0.0
    results = []
    for i, (arm, task_id, seed) in enumerate(cells):
        nonce = uuid.uuid4().hex
        env = {
            **os.environ,
            "SWEEP_ID": sweep_id,
            "RUN_ORDER_INDEX": str(i),
            "NONCE": nonce,
        }
        print(f"[{i + 1}/{len(cells)}] arm={arm} task={task_id} seed={seed}", flush=True)
        proc = subprocess.run(
            [str(RUN_SH), arm, task_id, str(seed)],
            env=env,
            capture_output=True,
            text=True,
        )
        tail = proc.stdout.strip().split("\n")[-1] if proc.stdout else ""
        try:
            info = json.loads(tail)
            total_spend += info.get("cost", 0.0)
            results.append({"arm": arm, "task_id": task_id, "seed": seed, **info})
            print(f"  verdict={info.get('verdict')} cost={info.get('cost'):.3f} running_total=${total_spend:.2f}")
        except json.JSONDecodeError:
            results.append({"arm": arm, "task_id": task_id, "seed": seed, "error": "unparseable run.sh output"})
            print(f"  stderr: {proc.stderr[:400]}")
        if total_spend > args.budget_ceiling:
            print(f"BUDGET CEILING ${args.budget_ceiling:.2f} EXCEEDED — halting sweep", file=sys.stderr)
            break

    (sweep_dir / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    print(f"\nsweep complete: total=${total_spend:.2f} → {sweep_dir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
