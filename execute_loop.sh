#!/usr/bin/env bash
set -euo pipefail

if ! command -v orbit >/dev/null 2>&1; then
  echo "[orbit] error: orbit CLI is not installed or not in PATH" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "[orbit] error: jq is required (brew install jq)" >&2
  exit 1
fi

while :; do
  BACKLOG_COUNT="$(orbit task list --status backlog --json | jq 'length')"
  if [[ "$BACKLOG_COUNT" == "0" ]]; then
    echo "[orbit] no backlog tasks remaining at $(date -Is)"
    break
  fi

  echo "[orbit] backlog tasks: ${BACKLOG_COUNT}; running pipeline at $(date -Is)"

  if ! orbit job run job_parallel_task_pipeline; then
    echo "[orbit] job failed at $(date -Is)" >&2
    # Keep trying until backlog is empty.
  fi
done