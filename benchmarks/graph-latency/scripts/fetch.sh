#!/usr/bin/env bash
# Fetch corpora for the graph-latency benchmark.
#
# Clones pinned <org>/<repo>@<sha> targets into ~/.cache/orbit-bench/<corpus>.
# Idempotent: if the cache directory already exists at the expected SHA, skip.
#
# The corpus list is the source of truth in benchmarks/graph-latency/<version>/METHOD.md.
# This script is a placeholder skeleton — the real fetch logic is implemented
# alongside the first sweep. See task: graph-latency v1 first sweep (follow-up).

set -euo pipefail

usage() {
  cat <<EOF
Usage: fetch.sh [--version vN] [--corpus <name>] [--cache-dir <path>]

Options:
  --version vN       benchmark round to fetch corpora for (default: v1)
  --corpus NAME      fetch a single corpus by name (default: all in METHOD.md)
  --cache-dir PATH   override cache root (default: ~/.cache/orbit-bench)
  -h, --help         show this help

This is a skeleton. The real implementation will:
  1. Parse the corpus list from benchmarks/graph-latency/<version>/METHOD.md.
  2. For each <org>/<repo>@<sha>, clone shallow into <cache-dir>/<corpus>.
  3. Verify the checked-out SHA matches the pin.
  4. Record the resolved corpus_sha for the harness.
EOF
}

VERSION="v1"
CORPUS=""
CACHE_DIR="${HOME}/.cache/orbit-bench"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --corpus) CORPUS="$2"; shift 2 ;;
    --cache-dir) CACHE_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

echo "fetch.sh skeleton — version=${VERSION} corpus=${CORPUS:-<all>} cache=${CACHE_DIR}"
echo "TODO: implement fetch loop against benchmarks/graph-latency/${VERSION}/METHOD.md corpus list."
