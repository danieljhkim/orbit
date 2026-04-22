#!/usr/bin/env bash
# Single-cell benchmark driver. Thin wrapper around run.py so the AC
# "`bash benchmarks/graph/scripts/run.sh`" is honored while the real
# logic lives in Python.
#
# Usage:
#   benchmarks/graph/scripts/run.sh <arm> <task_id> <seed> [--no-probe] [--budget N]
#
# Environment (optional; normally set by sweep.py):
#   SWEEP_ID           — groups runs into a sweep
#   RUN_ORDER_INDEX    — 0-based index within a shuffled sweep order
#   NONCE              — cold-cache nonce (uuid4 if unset)
#   CLAUDE_BIN         — override claude binary path

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$SCRIPT_DIR/run.py" "$@"
