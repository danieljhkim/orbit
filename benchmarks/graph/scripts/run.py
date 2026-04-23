"""Single-cell benchmark run driver.

Invoked by run.sh. Spawns a child CLI session with deterministic arm
steering, a fresh cold-cache nonce, and optional pre-flight probe.
Writes a canonical per-run record under benchmarks/graph/runs/.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
import uuid
from pathlib import Path

import classify
import oracle
from providers import DEFAULT_MODELS, build_system_prompt_suffix, get_provider

BENCH_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = BENCH_ROOT.parent.parent
MCP_CONFIG = str(BENCH_ROOT / "mcp.json")


def system_prompt_hash(suffix: str) -> str:
    return hashlib.sha256(suffix.encode()).hexdigest()[:16]


def build_record(
    *,
    provider: str,
    requested_model: str,
    arm: str,
    task_id: str,
    seed: int,
    sweep_id: str,
    run_order_index: int,
    nonce: str,
    fixture_path: str,
    allowed_tools: list[str],
    disallowed_tools: list[str],
    transcript_path: Path,
    system_suffix: str,
) -> dict:
    return {
        "provider": provider,
        "requested_model": requested_model,
        "arm": arm,
        "task_id": task_id,
        "seed": seed,
        "sweep_id": sweep_id,
        "run_order_index": run_order_index,
        "nonce": nonce,
        "system_prompt_hash": system_prompt_hash(system_suffix),
        "fixture_path": fixture_path,
        "allowed_tools": allowed_tools,
        "disallowed_tools_head": disallowed_tools[:5],
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
        "command_failures": [],
        "transcript_path": str(transcript_path.relative_to(REPO_ROOT)),
        "final_diff_path": None,
    }


def apply_provider_result(record: dict, normalized_result: dict) -> None:
    record["input_tokens"] = normalized_result.get("input_tokens", 0)
    record["cache_read_tokens"] = normalized_result.get("cache_read_tokens", 0)
    record["cache_creation_tokens"] = normalized_result.get(
        "cache_creation_tokens",
        0,
    )
    record["output_tokens"] = normalized_result.get("output_tokens", 0)
    record["total_cost_usd"] = normalized_result.get("total_cost_usd", 0.0)
    record["turns"] = normalized_result.get("turns", 0)
    record["model_usage"] = normalized_result.get("model_usage", {})
    record["permission_denials"] = normalized_result.get("permission_denials", [])
    record["command_failures"] = normalized_result.get("command_failures", [])
    record["tool_calls"] = normalized_result.get("tool_calls", {})


def write_record(record: dict, path: Path) -> None:
    path.write_text(json.dumps(record, indent=2) + "\n")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("arm", choices=["no-graph", "graph-only", "hybrid"])
    ap.add_argument("task_id")
    ap.add_argument("seed", type=int)
    ap.add_argument(
        "--provider",
        choices=sorted(DEFAULT_MODELS.keys()),
        default="claude",
        help="child CLI provider to benchmark",
    )
    ap.add_argument("--fixture", help="path to fixture YAML (default: tasks/<task_id>.yaml)")
    ap.add_argument("--no-probe", action="store_true", help="skip pre-flight probe")
    ap.add_argument("--budget", type=float, default=1.0, help="budget hint for Claude runs")
    args = ap.parse_args()

    provider_runner = get_provider(
        args.provider,
        repo_root=REPO_ROOT,
        mcp_config=MCP_CONFIG,
    )
    requested_model = provider_runner.default_model
    sweep_id = os.environ.get("SWEEP_ID", "adhoc")
    run_order_index = int(os.environ.get("RUN_ORDER_INDEX", "0"))
    nonce = os.environ.get("NONCE", str(uuid.uuid4()))

    fixture_path = Path(args.fixture or BENCH_ROOT / "tasks" / f"{args.task_id}.yaml")
    fixture = oracle.load_fixture(fixture_path)
    arm_config = provider_runner.arm_config(args.arm)
    system_suffix = build_system_prompt_suffix(
        nonce,
        sweep_id,
        args.arm,
        provider=args.provider,
    )

    out_dir = BENCH_ROOT / "runs" / args.provider / args.arm / args.task_id
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"{args.seed}.json"
    transcript_path = out_dir / f"{args.seed}.transcript.json"

    record = build_record(
        provider=args.provider,
        requested_model=requested_model,
        arm=args.arm,
        task_id=args.task_id,
        seed=args.seed,
        sweep_id=sweep_id,
        run_order_index=run_order_index,
        nonce=nonce,
        fixture_path=str(fixture_path.relative_to(REPO_ROOT)),
        allowed_tools=list(arm_config.allowed),
        disallowed_tools=list(arm_config.disallowed),
        transcript_path=transcript_path,
        system_suffix=system_suffix,
    )

    if not args.no_probe and args.arm != "no-graph":
        ok, diag = provider_runner.preflight_probe(
            arm=args.arm,
            nonce=nonce,
            sweep_id=sweep_id,
            model=requested_model,
        )
        if not ok:
            record["verdict"] = "error"
            record["diagnostic"] = f"pre-flight probe failed: {diag}"
            write_record(record, out_path)
            print(
                json.dumps(
                    {
                        "provider": args.provider,
                        "out": str(out_path),
                        "verdict": "error",
                        "diag": diag,
                    }
                )
            )
            return 2

    t0 = time.monotonic()
    execution = provider_runner.run(
        prompt=fixture["prompt"],
        arm=args.arm,
        arm_config=arm_config,
        system_suffix=system_suffix,
        nonce=nonce,
        sweep_id=sweep_id,
        model=requested_model,
        budget_usd=args.budget,
    )
    record["wall_seconds"] = round(time.monotonic() - t0, 2)
    transcript_path.write_text(execution.raw_stdout)

    normalized_result = execution.normalized_result
    if normalized_result is None:
        record["verdict"] = "error"
        record["diagnostic"] = (
            f"{args.provider} produced no parseable result (exit={execution.exit_code})"
        )
        write_record(record, out_path)
        return 3

    apply_provider_result(record, normalized_result)
    final_message = normalized_result.get("final_message") or ""

    if normalized_result.get("is_error") or execution.exit_code != 0:
        oracle_verdict = None
    else:
        oracle_verdict, _oracle_rationale = oracle.grade(
            fixture,
            final_message,
            sandbox=str(REPO_ROOT),
        )

    verdict, diag = classify.classify_run(
        provider=args.provider,
        arm=args.arm,
        run_result=normalized_result,
        oracle_verdict=oracle_verdict,
    )
    record["verdict"] = verdict
    record["diagnostic"] = diag

    write_record(record, out_path)
    print(
        json.dumps(
            {
                "provider": args.provider,
                "out": str(out_path),
                "verdict": verdict,
                "cost": record["total_cost_usd"],
                "wall": record["wall_seconds"],
            }
        )
    )
    return 0 if verdict != "error" else 4


if __name__ == "__main__":
    sys.exit(main())
