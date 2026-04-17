#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cargo run --quiet -p orbit-cli --bin migrate-task-attribution -- "$@"
