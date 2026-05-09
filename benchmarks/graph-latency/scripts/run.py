#!/usr/bin/env python3
"""Run a single graph-latency cell.

A cell is one (corpus, tool, phase, seed) measurement. Phases:
  build-cold         clear the corpus's graph state, run `orbit graph build`
  build-incremental  apply a controlled mutation, run `orbit graph update`
  query              run `orbit tool run orbit.graph.<name>` with a corpus-aware input

Per-cell record schema is documented in v1/METHOD.md. The aggregator and
sweep harness depend on the field set there; do not rename or drop fields
without bumping the version.
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import resource
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

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

DEFAULT_CACHE_DIR = Path.home() / ".cache" / "orbit-bench"
INCREMENTAL_MARKER = "// orbit-bench-incremental-marker"

# Corpora are checked out in detached-HEAD state by fetch.sh, so we pass an
# explicit graph ref to `orbit graph build/update` to satisfy its detached-HEAD
# guard. The ref name is harness-internal and corpus-local — graph state lives
# under <corpus>/.orbit/knowledge/, so collisions across corpora aren't possible.
GRAPH_REF = "orbit-bench-v1"

# Per-phase wall-clock ceiling; cells that exceed this are recorded with a
# timeout verdict instead of blocking the whole sweep.
PHASE_TIMEOUTS_S = {
    "build-cold": 1800,
    "build-incremental": 600,
    "query": 300,
}


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Run a single graph-latency cell.")
    p.add_argument("--corpus", required=True, help="corpus name from corpora.yaml")
    p.add_argument("--tool", default=None, choices=TOOLS, help="orbit.graph.* tool (query phase only)")
    p.add_argument("--phase", required=True, choices=PHASES)
    p.add_argument("--seed", required=True, type=int, help="1-indexed seed")
    p.add_argument("--version", default="v1")
    p.add_argument("--cache-dir", default=str(DEFAULT_CACHE_DIR))
    p.add_argument("--out-dir", default=None, help="record output dir (default: <bench>/<version>/runs)")
    p.add_argument("--orbit-bin", default="orbit", help="orbit CLI to invoke")
    return p.parse_args(argv)


def load_yaml(path: Path) -> dict[str, Any]:
    """Hand-rolled minimal YAML reader for the two files the harness owns.

    Avoids a PyYAML dependency. Both files are hand-controlled and use a
    fixed shape: top-level `corpora:` (list of maps in corpora.yaml, map of
    maps in queries.yaml). If you extend either file, extend this parser.
    """
    text = path.read_text()
    if path.name == "corpora.yaml":
        return _parse_corpora(text)
    if path.name == "queries.yaml":
        return _parse_queries(text)
    raise ValueError(f"unsupported yaml file: {path}")


def _parse_corpora(text: str) -> dict[str, Any]:
    out: list[dict[str, Any]] = []
    cur: dict[str, Any] | None = None
    for raw in text.splitlines():
        line = raw.rstrip()
        if not line or line.lstrip().startswith("#"):
            continue
        if line.startswith("corpora:"):
            continue
        if line.startswith("  - "):
            if cur is not None:
                out.append(cur)
            cur = {}
            line = line[4:]
        elif line.startswith("    "):
            line = line[4:]
        else:
            continue
        if cur is None:
            cur = {}
        if ":" in line:
            k, _, v = line.partition(":")
            cur[k.strip()] = v.strip().strip('"').strip("'")
    if cur is not None:
        out.append(cur)
    return {"corpora": out}


def _parse_queries(text: str) -> dict[str, Any]:
    """Parse v1/tasks/queries.yaml.

    Shape:
      corpora:
        <corpus_name>:
          search_terms: [str, ...]
          selectors:    [str, ...]
          prefixes:     [str, ...]
    """
    corpora: dict[str, dict[str, list[str]]] = {}
    cur_corpus: str | None = None
    cur_field: str | None = None
    for raw in text.splitlines():
        line = raw.rstrip()
        if not line or line.lstrip().startswith("#"):
            continue
        if line == "corpora:":
            continue
        if line.startswith("  ") and not line.startswith("    "):
            cur_corpus = line.strip().rstrip(":")
            corpora[cur_corpus] = {}
            cur_field = None
            continue
        if line.startswith("    ") and not line.startswith("      "):
            cur_field = line.strip().rstrip(":")
            if cur_corpus is not None:
                corpora[cur_corpus][cur_field] = []
            continue
        if line.startswith("      - "):
            value = line[8:].strip().strip('"').strip("'")
            if cur_corpus is not None and cur_field is not None:
                corpora[cur_corpus][cur_field].append(value)
    return {"corpora": corpora}


def find_bench_root() -> Path:
    return Path(__file__).resolve().parent.parent


def get_corpus(version: str, name: str) -> dict[str, Any]:
    bench_root = find_bench_root()
    path = bench_root / version / "corpora.yaml"
    data = load_yaml(path)
    for c in data["corpora"]:
        if c["name"] == name:
            return c
    raise SystemExit(f"unknown corpus: {name}")


def get_query_primitives(version: str, corpus: str) -> dict[str, list[str]]:
    bench_root = find_bench_root()
    path = bench_root / version / "tasks" / "queries.yaml"
    data = load_yaml(path)
    return data["corpora"].get(corpus, {})


def host_metadata() -> dict[str, Any]:
    cpu = platform.processor() or platform.machine()
    if sys.platform == "darwin":
        try:
            cpu = subprocess.check_output(
                ["sysctl", "-n", "machdep.cpu.brand_string"], text=True
            ).strip()
        except Exception:
            pass
    ram_gb: int | None = None
    try:
        if sys.platform == "darwin":
            mem_bytes = int(subprocess.check_output(["sysctl", "-n", "hw.memsize"]))
            ram_gb = mem_bytes // (1024**3)
        elif sys.platform.startswith("linux"):
            with open("/proc/meminfo") as f:
                for line in f:
                    if line.startswith("MemTotal:"):
                        kb = int(line.split()[1])
                        ram_gb = kb // (1024 * 1024)
                        break
    except Exception:
        ram_gb = None
    return {
        "cpu": cpu,
        "ram_gb": ram_gb,
        "os": platform.platform(),
    }


def orbit_sha() -> str:
    """Return the orbit binary's source SHA — i.e. the SHA of the orbit checkout
    we built the binary from. Best effort; for v1 we read it from the harness
    checkout (this repo) which is the same thing in practice.
    """
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=str(find_bench_root().parent.parent), text=True
        ).strip()
    except Exception:
        return ""


def corpus_sha(repo_path: Path) -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=str(repo_path), text=True
        ).strip()
    except Exception:
        return ""


def maxrss_mb_delta(before: int, after: int) -> int:
    """Convert ru_maxrss delta to MiB. macOS reports bytes; Linux reports KB."""
    delta = max(0, after - before)
    if sys.platform == "darwin":
        return delta // (1024 * 1024)
    return delta // 1024


def time_subprocess(
    cmd: list[str], timeout_s: int, capture_stdout: bool = False, cwd: str | None = None
) -> tuple[float, int, bytes, str]:
    """Run cmd, return (wall_ms, rss_peak_mb, stdout_bytes, error_str).

    On timeout, returns wall_ms = timeout_s * 1000, error_str = 'timeout'.
    On non-zero exit, error_str carries 'exit:<code>' plus the first chunk of stderr.

    `cwd` matters for orbit subcommands: orbit resolves the workspace orbit-root
    by walking up from cwd, NOT from --repo. Pass cwd=<corpus path> for build/update
    so the knowledge graph lands under the corpus's own .orbit/knowledge/, not in
    whatever orbit project the harness happens to live inside.
    """
    rss_before = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    t0 = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            cwd=cwd,
            timeout=timeout_s,
            stdout=subprocess.PIPE if capture_stdout else subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
    except subprocess.TimeoutExpired:
        wall_ms = int((time.perf_counter() - t0) * 1000)
        return wall_ms, 0, b"", "timeout"
    wall_ms = int((time.perf_counter() - t0) * 1000)
    rss_after = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    rss_mb = maxrss_mb_delta(rss_before, rss_after)
    stdout = proc.stdout or b""
    error = ""
    if proc.returncode != 0:
        stderr_snippet = (proc.stderr or b"")[:400].decode("utf-8", errors="replace")
        error = f"exit:{proc.returncode} {stderr_snippet.strip()}"
    return wall_ms, rss_mb, stdout, error


def compose_query_input(tool: str, primitives: dict[str, list[str]], seed: int) -> tuple[dict, str]:
    """Return (input_json, query_shape_id) for a query-phase cell.

    Rotates through the primitive lists by seed so seeds 1..N cover different
    queries without exploding the matrix.
    """
    def pick(field: str) -> str:
        items = primitives.get(field) or []
        if not items:
            raise SystemExit(f"queries.yaml missing {field} for this corpus")
        return items[(seed - 1) % len(items)]

    base = {"ref": GRAPH_REF}  # detached-HEAD corpora need an explicit ref on every call
    if tool == "graph.overview":
        return base, "overview-default"
    if tool == "graph.search":
        term = pick("search_terms")
        return {**base, "query": term}, f"search:{term}"
    if tool == "graph.deps":
        return base, "deps-default"
    if tool == "graph.history":
        sel = pick("selectors")
        return {**base, "selector": sel}, f"history:{sel}"
    if tool == "graph.pack":
        return {**base, "selectors": primitives.get("selectors", [])}, "pack-all-selectors"
    if tool == "graph.implementors":
        sel = pick("selectors")
        return {**base, "trait_selector": sel}, f"impl:{sel}"
    # callers, refs, show
    sel = pick("selectors")
    return {**base, "selector": sel}, f"{tool.split('.')[-1]}:{sel}"


def parse_result_count(stdout: bytes) -> tuple[int | None, int]:
    """Best-effort (result_count, result_size_bytes) from tool stdout."""
    size = len(stdout)
    if not size:
        return None, 0
    try:
        obj = json.loads(stdout)
    except Exception:
        return None, size
    # Common shapes seen: {results:[...], total:N}, {nodes:[...]}, {graph:[...]}
    if isinstance(obj, dict):
        for key in ("total", "count"):
            if isinstance(obj.get(key), int):
                return obj[key], size
        for key in ("results", "nodes", "callers", "refs", "implementors", "code_refs"):
            v = obj.get(key)
            if isinstance(v, list):
                return len(v), size
    if isinstance(obj, list):
        return len(obj), size
    return None, size


def apply_mutation(repo: Path, rel_path: str) -> str:
    """Append a marker line to the mutation file. Returns the original content
    so revert_mutation can restore it. Idempotent against repeated calls only
    via revert."""
    target = repo / rel_path
    if not target.is_file():
        raise SystemExit(f"mutation_path not a file: {target}")
    original = target.read_text()
    target.write_text(original + "\n" + INCREMENTAL_MARKER + "\n")
    return original


def revert_mutation(repo: Path, rel_path: str, original: str) -> None:
    (repo / rel_path).write_text(original)


def run_build_cold(corpus_info: dict, repo: Path, orbit_bin: str) -> dict:
    """Clear the corpus's graph state, then run a full build."""
    knowledge_dir = repo / ".orbit" / "knowledge"
    if knowledge_dir.exists():
        shutil.rmtree(knowledge_dir)
    cmd = [orbit_bin, "graph", "build", "--repo", str(repo), "--ref", GRAPH_REF]
    wall_ms, rss_mb, _stdout, error = time_subprocess(cmd, PHASE_TIMEOUTS_S["build-cold"], cwd=str(repo))
    return {"wall_ms": wall_ms, "rss_peak_mb": rss_mb, "error": error}


def run_build_incremental(corpus_info: dict, repo: Path, orbit_bin: str) -> dict:
    """Apply a mutation, run incremental update, revert."""
    rel_path = corpus_info["mutation_path"]
    original = apply_mutation(repo, rel_path)
    try:
        cmd = [orbit_bin, "graph", "update", "--repo", str(repo), "--ref", GRAPH_REF]
        wall_ms, rss_mb, _stdout, error = time_subprocess(cmd, PHASE_TIMEOUTS_S["build-incremental"], cwd=str(repo))
    finally:
        revert_mutation(repo, rel_path, original)
    return {"wall_ms": wall_ms, "rss_peak_mb": rss_mb, "error": error}


def run_query(tool: str, repo: Path, primitives: dict, seed: int, orbit_bin: str) -> dict:
    """Invoke orbit.graph.<tool> with a corpus-aware input, time it."""
    input_obj, query_shape = compose_query_input(tool, primitives, seed)
    cmd = [
        orbit_bin,
        "tool",
        "run",
        f"orbit.{tool}",
        "--input",
        json.dumps(input_obj),
        "--output",
        "json",
    ]
    # The graph tools resolve the workspace from cwd, so run from the repo.
    rss_before = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    t0 = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            cwd=str(repo),
            timeout=PHASE_TIMEOUTS_S["query"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except subprocess.TimeoutExpired:
        return {
            "wall_ms": PHASE_TIMEOUTS_S["query"] * 1000,
            "rss_peak_mb": 0,
            "result_size_bytes": 0,
            "result_count": None,
            "query_shape": query_shape,
            "error": "timeout",
        }
    wall_ms = int((time.perf_counter() - t0) * 1000)
    rss_after = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    rss_mb = maxrss_mb_delta(rss_before, rss_after)
    error = ""
    if proc.returncode != 0:
        # Orbit tools emit error JSON to stdout (e.g. {"code":"...","error":"..."}),
        # not stderr — so include both, prioritizing stdout for the message body.
        stdout = (proc.stdout or b"")[:400].decode("utf-8", errors="replace")
        stderr = (proc.stderr or b"")[:200].decode("utf-8", errors="replace")
        error = f"exit:{proc.returncode} {stdout.strip() or stderr.strip()}"
    count, size = parse_result_count(proc.stdout or b"")
    return {
        "wall_ms": wall_ms,
        "rss_peak_mb": rss_mb,
        "result_size_bytes": size,
        "result_count": count,
        "query_shape": query_shape,
        "error": error,
    }


def write_record(out_dir: Path, record: dict) -> Path:
    if record["phase"] == "query":
        leaf_dir = out_dir / record["corpus"] / record["tool"] / record["phase"]
    else:
        leaf_dir = out_dir / record["corpus"] / "_build" / record["phase"]
    leaf_dir.mkdir(parents=True, exist_ok=True)
    path = leaf_dir / f"{record['seed']}.json"
    path.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
    return path


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.phase == "query" and not args.tool:
        raise SystemExit("--tool is required for phase=query")

    corpus_info = get_corpus(args.version, args.corpus)
    cache_dir = Path(os.path.expanduser(args.cache_dir))
    repo = cache_dir / corpus_info["name"]
    if not repo.is_dir():
        raise SystemExit(f"corpus not fetched yet: {repo} (run fetch.sh first)")

    if args.phase == "build-cold":
        result = run_build_cold(corpus_info, repo, args.orbit_bin)
        result.update({"tool": None, "query_shape": None, "result_size_bytes": None, "result_count": None})
    elif args.phase == "build-incremental":
        result = run_build_incremental(corpus_info, repo, args.orbit_bin)
        result.update({"tool": None, "query_shape": None, "result_size_bytes": None, "result_count": None})
    else:
        primitives = get_query_primitives(args.version, args.corpus)
        result = run_query(args.tool, repo, primitives, args.seed, args.orbit_bin)
        result["tool"] = args.tool

    record = {
        "corpus": args.corpus,
        "phase": args.phase,
        "seed": args.seed,
        "host": host_metadata(),
        "orbit_sha": orbit_sha(),
        "corpus_sha": corpus_sha(repo),
    }
    record.update(result)

    out_dir = Path(args.out_dir) if args.out_dir else (find_bench_root() / args.version / "runs")
    path = write_record(out_dir, record)
    print(f"[record] {path}")
    print(json.dumps(record, indent=2, sort_keys=True))
    return 0 if not record.get("error") else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
