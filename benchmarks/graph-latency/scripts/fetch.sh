#!/usr/bin/env bash
# Fetch corpora for the graph-latency benchmark.
#
# Reads benchmarks/graph-latency/<version>/corpora.yaml and clones each pinned
# <repo>@<sha> into ~/.cache/orbit-bench/<corpus>. Idempotent — if the cache
# directory already sits at the expected SHA, the corpus is skipped.

set -euo pipefail

usage() {
  cat <<EOF
Usage: fetch.sh [--version vN] [--corpus <name>] [--cache-dir <path>] [--force]

Options:
  --version vN       benchmark round to fetch corpora for (default: v1)
  --corpus NAME      fetch a single corpus by name (default: all in corpora.yaml)
  --cache-dir PATH   override cache root (default: ~/.cache/orbit-bench)
  --force            re-fetch even if the SHA already matches
  -h, --help         show this help
EOF
}

VERSION="v1"
CORPUS=""
CACHE_DIR="${HOME}/.cache/orbit-bench"
FORCE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --corpus) CORPUS="$2"; shift 2 ;;
    --cache-dir) CACHE_DIR="$2"; shift 2 ;;
    --force) FORCE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

SCRIPT_DIR="$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
BENCH_ROOT="$( cd -- "${SCRIPT_DIR}/.." &> /dev/null && pwd )"
CORPORA_YAML="${BENCH_ROOT}/${VERSION}/corpora.yaml"

if [[ ! -f "${CORPORA_YAML}" ]]; then
  echo "fatal: corpora.yaml not found at ${CORPORA_YAML}" >&2
  exit 1
fi

mkdir -p "${CACHE_DIR}"

# Parse corpora.yaml with a minimal awk pipeline. We only need name/repo/sha;
# yaml shape is hand-controlled in v1 (no anchors, no nested maps beyond what
# we handle here), so a real YAML parser would be overkill.
parse_corpora() {
  awk '
    /^  - name:/        { if (name) print name "\t" repo "\t" sha; name=$3; repo=""; sha="" }
    /^    repo:/        { repo=$2 }
    /^    sha:/         { sha=$2 }
    END                 { if (name) print name "\t" repo "\t" sha }
  ' "${CORPORA_YAML}"
}

fetch_one() {
  local name="$1" repo="$2" sha="$3"
  local dest="${CACHE_DIR}/${name}"
  local url="https://github.com/${repo}.git"

  if [[ -d "${dest}/.git" ]]; then
    local current
    current="$(git -C "${dest}" rev-parse HEAD 2>/dev/null || echo none)"
    if [[ "${current}" == "${sha}" && "${FORCE}" -eq 0 ]]; then
      echo "[skip] ${name} already at ${sha}"
      return 0
    fi
  fi

  echo "[fetch] ${name} <- ${repo}@${sha}"
  if [[ ! -d "${dest}/.git" ]]; then
    git clone --quiet --filter=blob:none "${url}" "${dest}"
  fi
  git -C "${dest}" fetch --quiet origin "${sha}"
  git -C "${dest}" checkout --quiet --detach "${sha}"
  echo "[done] ${name} at $(git -C "${dest}" rev-parse HEAD)"
}

while IFS=$'\t' read -r name repo sha; do
  if [[ -z "${name}" ]]; then continue; fi
  if [[ -n "${CORPUS}" && "${CORPUS}" != "${name}" ]]; then continue; fi
  fetch_one "${name}" "${repo}" "${sha}"
done < <(parse_corpora)
