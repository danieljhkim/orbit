#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

baseline_path="bench/baselines.json"
results_path="target/bench/results.json"
threshold_percent="20"

usage() {
  cat <<'EOF'
Usage: bench/check_baseline.sh [--baseline PATH] [--results PATH] [--threshold-percent N]

Compares gated rows in target/bench/results.json against bench/baselines.json.
Rows fail when their median value is more than N percent slower than baseline.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --baseline)
      baseline_path="$2"
      shift 2
      ;;
    --results)
      results_path="$2"
      shift 2
      ;;
    --threshold-percent)
      threshold_percent="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ ! -f "$baseline_path" ]]; then
  echo "baseline file not found: $baseline_path" >&2
  exit 2
fi

if [[ ! -f "$results_path" ]]; then
  echo "results file not found: $results_path" >&2
  exit 2
fi

report="$(
  jq -n \
    --slurpfile baseline "$baseline_path" \
    --slurpfile results "$results_path" \
    --argjson threshold_percent "$threshold_percent" '
      ($baseline[0].rows // []) as $baseline_rows
      | ($results[0].rows // []) as $result_rows
      | (($threshold_percent + 100) / 100) as $multiplier
      | def baseline_row($id):
          first($baseline_rows[] | select(.id == $id));
        [ $result_rows[] | select(.gate != false) ] as $gated
      | {
          checked: [
            $gated[] as $row
            | ($row.baseline_id // $row.id) as $baseline_id
            | (baseline_row($baseline_id)) as $baseline
            | select($baseline != null and (($baseline.baseline.value // null) != null))
            | {
                id: $row.id,
                baseline_id: $baseline_id,
                implementation: ($row.implementation // "unknown"),
                value: $row.value,
                baseline_value: $baseline.baseline.value,
                unit: ($row.unit // ""),
                baseline_unit: ($baseline.baseline.unit // ""),
                limit: ($baseline.baseline.value * $multiplier),
                slower_percent: (
                  if ($baseline.baseline.value | tonumber) > 0 then
                    (($row.value - $baseline.baseline.value) / $baseline.baseline.value * 100)
                  else
                    null
                  end
                )
              }
          ],
          missing: [
            $gated[] as $row
            | ($row.baseline_id // $row.id) as $baseline_id
            | select((baseline_row($baseline_id)) == null)
            | { id: $row.id, baseline_id: $baseline_id }
          ],
          invalid_baselines: [
            $gated[] as $row
            | ($row.baseline_id // $row.id) as $baseline_id
            | (baseline_row($baseline_id)) as $baseline
            | select($baseline != null and (($baseline.baseline.value // null) == null or ($baseline.baseline.value | tonumber) <= 0))
            | { id: $row.id, baseline_id: $baseline_id }
          ]
        }
      | . + {
          unit_mismatches: [
            .checked[]
            | select(.unit != .baseline_unit)
            | { id, unit, baseline_unit }
          ],
          regressions: [
            .checked[]
            | select(.value > .limit)
          ]
        }
    '
)"

echo "$report" | jq -r '
  .checked[]
  | "\(.id) [\(.implementation)]: \(.value)\(.unit) vs baseline \(.baseline_value)\(.baseline_unit) (limit \(.limit | floor)\(.baseline_unit))"
'

if [[ "$(echo "$report" | jq '.checked | length')" -eq 0 ]]; then
  echo "no gated result rows found in $results_path" >&2
  exit 1
fi

if ! echo "$report" | jq -e '
  (.missing | length) == 0
  and (.invalid_baselines | length) == 0
  and (.unit_mismatches | length) == 0
  and (.regressions | length) == 0
' >/dev/null; then
  echo "$report" | jq -r '
    (.missing[]? | "missing baseline for result row \(.id) (baseline_id=\(.baseline_id))"),
    (.invalid_baselines[]? | "invalid baseline for result row \(.id) (baseline_id=\(.baseline_id))"),
    (.unit_mismatches[]? | "unit mismatch for \(.id): result \(.unit), baseline \(.baseline_unit)"),
    (.regressions[]? | "regression: \(.id) [\(.implementation)] \(.value)\(.unit) is \(.slower_percent | floor)% slower than baseline \(.baseline_value)\(.baseline_unit)")
  ' >&2
  exit 1
fi

echo "graph benchmark baseline check passed"
