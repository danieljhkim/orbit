#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

run_count="${GRAPH_BENCH_RUNS:-3}"
if ! [[ "$run_count" =~ ^[0-9]+$ ]] || (( run_count < 1 )); then
  echo "GRAPH_BENCH_RUNS must be a positive integer" >&2
  exit 2
fi

target_dir="${GRAPH_BENCH_TARGET_DIR:-target/bench}"
samples_path="$target_dir/samples.ndjson"
results_path="$target_dir/results.json"
touch_path="crates/orbit-graph-cli/src/__graph_bench_touch.rs"

cleanup() {
  rm -f "$touch_path"
}
trap cleanup EXIT

rm -rf "$target_dir"
mkdir -p "$target_dir"
: > "$samples_path"
cleanup

cargo build -p orbit-knowledge --example graph_build --release
cargo build -p orbit-graph-cli --release

graph_build_bin="$repo_root/target/release/examples/graph_build"
graph_cli_bin="$repo_root/target/release/orbit-graph-cli"

now_ms() {
  perl -MTime::HiRes=time -e 'printf "%.0f\n", time() * 1000'
}

file_size_bytes() {
  local path="$1"
  if stat -c %s "$path" >/dev/null 2>&1; then
    stat -c %s "$path"
  else
    stat -f %z "$path"
  fi
}

memory_mib() {
  if [[ -r /proc/meminfo ]]; then
    awk '/MemTotal/ { print int($2 / 1024) }' /proc/meminfo
  else
    echo 0
  fi
}

measure_ms() {
  local output_path="$1"
  shift
  local start_ms end_ms
  start_ms="$(now_ms)"
  "$@" > "$output_path"
  end_ms="$(now_ms)"
  echo $((end_ms - start_ms))
}

measure_peak_rss_kib() {
  local output_path="$1"
  shift
  local rss_path="${output_path}.rss"
  if /usr/bin/time -f "%M" -o "$rss_path" true >/dev/null 2>&1; then
    /usr/bin/time -f "%M" -o "$rss_path" "$@" > "$output_path"
    cat "$rss_path"
  else
    "$@" > "$output_path"
    echo 0
  fi
}

add_sample() {
  local run="$1"
  local id="$2"
  local baseline_id="$3"
  local implementation="$4"
  local gate="$5"
  local unit="$6"
  local value="$7"

  jq -nc \
    --argjson run "$run" \
    --arg id "$id" \
    --arg baseline_id "$baseline_id" \
    --arg implementation "$implementation" \
    --argjson gate "$gate" \
    --arg unit "$unit" \
    --argjson value "$value" \
    '{
      run: $run,
      id: $id,
      baseline_id: $baseline_id,
      implementation: $implementation,
      gate: $gate,
      unit: $unit,
      value: $value
    }' >> "$samples_path"
}

for run_index in $(seq 1 "$run_count"); do
  run_dir="$target_dir/run-$run_index"
  mkdir -p "$run_dir"

  rm -rf .orbit/graph
  cleanup

  "$graph_build_bin" \
    --workspace "$repo_root" \
    --knowledge-dir "$run_dir/v1-knowledge" \
    --scoreboard "$run_dir/v1-scoreboard.json" \
    > "$run_dir/v1-summary.txt"

  jq -c --argjson run "$run_index" '
    .[-1] as $record
    | [
        {
          id: "v1_cold_build",
          baseline_id: "cold_full_build",
          implementation: "v1",
          gate: false,
          unit: "ms",
          value: $record.scenarios.cold_build.wall_time_ms
        },
        {
          id: "v1_incremental_no_changes",
          baseline_id: "incremental_no_changes",
          implementation: "v1",
          gate: false,
          unit: "ms",
          value: $record.scenarios.warm_incremental_noop.wall_time_ms
        },
        {
          id: "v1_resident_memory",
          baseline_id: "resident_memory",
          implementation: "v1",
          gate: false,
          unit: "KiB",
          value: ($record.scenarios.warm_incremental_noop.peak_rss_kib // 0)
        }
      ]
    | .[] + { run: $run }
  ' "$run_dir/v1-scoreboard.json" >> "$samples_path"

  v2_full_sync_json="$run_dir/v2-full-sync.json"
  v2_rss_kib="$(measure_peak_rss_kib "$v2_full_sync_json" "$graph_cli_bin" sync --full)"
  add_sample "$run_index" "cold_full_build" "cold_full_build" "v2" "true" "ms" "$(jq -r '.duration_ms' "$v2_full_sync_json")"
  add_sample "$run_index" "resident_memory" "resident_memory" "v2" "true" "KiB" "$v2_rss_kib"

  v2_noop_sync_json="$run_dir/v2-noop-sync.json"
  "$graph_cli_bin" sync > "$v2_noop_sync_json"
  add_sample "$run_index" "incremental_no_changes" "incremental_no_changes" "v2" "true" "ms" "$(jq -r '.duration_ms' "$v2_noop_sync_json")"

  cat > "$touch_path" <<'RS'
pub fn graph_bench_touch() -> u8 {
    1
}
RS
  v2_one_file_json="$run_dir/v2-one-file-sync.json"
  "$graph_cli_bin" sync > "$v2_one_file_json"
  add_sample "$run_index" "incremental_one_file_changed" "incremental_one_file_changed" "v2" "true" "ms" "$(jq -r '.duration_ms' "$v2_one_file_json")"

  search_ms="$(measure_ms "$run_dir/v2-search.json" "$graph_cli_bin" search GraphBenchOptions --kind symbol --limit 20)"
  add_sample "$run_index" "search" "search" "v2" "true" "ms" "$search_ms"

  refs_selector="symbol:crates/orbit-knowledge/src/graph_bench.rs#run_benchmark_with_child_process:function"
  refs_ms="$(measure_ms "$run_dir/v2-refs.json" "$graph_cli_bin" refs "$refs_selector" --confidence same_module)"
  add_sample "$run_index" "refs" "refs" "v2" "true" "ms" "$refs_ms"

  impact_selector="symbol:crates/orbit-knowledge/src/graph_bench.rs#append_scoreboard:function"
  impact_ms="$(measure_ms "$run_dir/v2-impact.json" "$graph_cli_bin" impact "$impact_selector" --depth 3)"
  add_sample "$run_index" "impact_depth_3" "impact_depth_3" "v2" "true" "ms" "$impact_ms"

  v2_db_path_json="$run_dir/v2-db-path.json"
  "$graph_cli_bin" db-path > "$v2_db_path_json"
  db_path="$(jq -r '.path' "$v2_db_path_json")"
  add_sample "$run_index" "db_size" "db_size" "v2" "true" "bytes" "$(file_size_bytes "$db_path")"
done

generated_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
git_sha="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
logical_core_count="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 0)"
runner_image="${GRAPH_BENCH_RUNNER_IMAGE:-ubuntu-24.04}"
runner_os_release="$(if [[ -r /etc/os-release ]]; then . /etc/os-release && echo "${PRETTY_NAME:-unknown}"; else echo unknown; fi)"

jq -s \
  --arg generated_at "$generated_at" \
  --arg git_sha "$git_sha" \
  --argjson run_count "$run_count" \
  --arg runner_image "$runner_image" \
  --arg runner_os_release "$runner_os_release" \
  --argjson logical_core_count "$logical_core_count" \
  --argjson memory_mib "$(memory_mib)" '
    def median:
      sort
      | .[((length - 1) / 2 | floor)];

    sort_by(.id)
    | group_by(.id)
    | map(
        sort_by(.run) as $items
        | $items[0] as $first
        | ($items | map(.value) | median) as $median
        | {
            id: $first.id,
            baseline_id: $first.baseline_id,
            implementation: $first.implementation,
            gate: $first.gate,
            unit: $first.unit,
            statistic: ("median_of_" + ($items | length | tostring)),
            value: $median,
            samples: ($items | map({ run, value }))
          }
      )
    | sort_by((.gate == false), .id)
    | {
        schema_version: 1,
        generated_at: $generated_at,
        git_sha: $git_sha,
        run_count: $run_count,
        gate: {
          implementation: "v2",
          regression_threshold_percent: 20,
          statistic: "median"
        },
        runner: {
          image: $runner_image,
          os_release: $runner_os_release,
          logical_core_count: $logical_core_count,
          memory_mib: $memory_mib
        },
        rows: .
      }
  ' "$samples_path" > "$results_path"

echo "Wrote $results_path"
