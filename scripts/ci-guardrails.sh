#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

measure_ms() {
  local start end
  start="$(date +%s%N)"
  "$@" >/dev/null
  end="$(date +%s%N)"
  echo "$(((end - start + 999999) / 1000000))"
}

p95_json() {
  jq -n --argjson values "$1" '
    ($values | sort) as $sorted
    | ($sorted | length) as $n
    | if $n == 0 then null
      else ($n * 0.95 | ceil | if . < 1 then 1 elif . > $n then $n else . end) as $rank
      | $sorted[$rank - 1]
      end
  '
}

array_json() {
  printf '%s\n' "$@" | jq -Rsc 'split("\n")[:-1] | map(tonumber)'
}

echo "Runner profile"
echo "GITHUB_RUN_ID=${GITHUB_RUN_ID:-}"
echo "GITHUB_RUN_ATTEMPT=${GITHUB_RUN_ATTEMPT:-}"
echo "GITHUB_SHA=${GITHUB_SHA:-}"
uname -a
if [ -f /etc/os-release ]; then
  cat /etc/os-release
fi
nproc
free -m

cargo build -p orbit-cli --bin orbit --release --locked
cargo build -p orbit-knowledge --example graph_build --release --locked

rm -rf .orbit/knowledge .orbit/state/scoreboard/graph_bench.json
mkdir -p .orbit/state/scoreboard
./target/release/examples/graph_build --workspace "$repo_root" | tee /tmp/orbit-graph-build-summary.txt
graph_record="$(jq '.[-1]' .orbit/state/scoreboard/graph_bench.json)"

one_file_timings=()
for i in $(seq 1 5); do
  printf '\n// ci graph baseline mutation %s\n' "$i" >> crates/orbit-knowledge/examples/graph_build.rs
  one_file_timings+=("$(measure_ms ./target/release/orbit graph update --repo "$repo_root" --root "$repo_root/.orbit")")
done
one_file_timings_json="$(array_json "${one_file_timings[@]}")"
one_file_p95_ms="$(p95_json "$one_file_timings_json")"

search_timings=()
for _ in $(seq 1 7); do
  search_timings+=("$(measure_ms ./target/release/orbit tool run orbit.graph.search --root "$repo_root/.orbit" --input '{"query":"GraphCommandContext","limit":5,"model":"codex"}')")
done
search_timings_json="$(array_json "${search_timings[@]}")"
search_p95_ms="$(p95_json "$search_timings_json")"

refs_timings=()
for _ in $(seq 1 7); do
  refs_timings+=("$(measure_ms ./target/release/orbit tool run orbit.graph.refs --root "$repo_root/.orbit" --input '{"selector":"symbol:crates/orbit-knowledge/src/graph_bench.rs#run_benchmark_with_child_process:function","include":"all","limit":20,"model":"codex"}')")
done
refs_timings_json="$(array_json "${refs_timings[@]}")"
refs_p95_ms="$(p95_json "$refs_timings_json")"

impact_timings=()
for _ in $(seq 1 7); do
  impact_timings+=("$(measure_ms ./target/release/orbit tool run orbit.graph.callers --root "$repo_root/.orbit" --input '{"selector":"symbol:crates/orbit-knowledge/src/graph_bench.rs#append_scoreboard:function","depth":3,"model":"codex"}')")
done
impact_timings_json="$(array_json "${impact_timings[@]}")"
impact_p95_ms="$(p95_json "$impact_timings_json")"

db_size_bytes="$(stat -c%s .orbit/knowledge/graph/graph_index.sqlite)"
run_url="${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-danieljhkim/orbit}/actions/runs/${GITHUB_RUN_ID:-unknown}"

jq -n \
  --arg run_url "$run_url" \
  --arg run_id "${GITHUB_RUN_ID:-}" \
  --arg run_attempt "${GITHUB_RUN_ATTEMPT:-}" \
  --arg sha "${GITHUB_SHA:-}" \
  --arg ref "${GITHUB_REF_NAME:-}" \
  --arg runner_os "${RUNNER_OS:-}" \
  --arg runner_arch "${RUNNER_ARCH:-}" \
  --arg runner_name "${RUNNER_NAME:-}" \
  --argjson graph_record "$graph_record" \
  --argjson one_file_timings "$one_file_timings_json" \
  --argjson one_file_p95_ms "$one_file_p95_ms" \
  --argjson search_timings "$search_timings_json" \
  --argjson search_p95_ms "$search_p95_ms" \
  --argjson refs_timings "$refs_timings_json" \
  --argjson refs_p95_ms "$refs_p95_ms" \
  --argjson impact_timings "$impact_timings_json" \
  --argjson impact_p95_ms "$impact_p95_ms" \
  --argjson db_size_bytes "$db_size_bytes" \
  '{
    source: {
      run_url: $run_url,
      run_id: $run_id,
      run_attempt: $run_attempt,
      sha: $sha,
      ref: $ref,
      runner: {
        os: $runner_os,
        arch: $runner_arch,
        name: $runner_name,
        uname: (input_filename? // null)
      }
    },
    graph_bench_record: $graph_record,
    measurements: {
      incremental_one_file_changed_ms_p95: $one_file_p95_ms,
      incremental_one_file_changed_ms_samples: $one_file_timings,
      search_ms_p95: $search_p95_ms,
      search_ms_samples: $search_timings,
      refs_ms_p95: $refs_p95_ms,
      refs_ms_samples: $refs_timings,
      impact_depth_3_ms_p95: $impact_p95_ms,
      impact_depth_3_ms_samples: $impact_timings,
      graph_index_sqlite_size_bytes: $db_size_bytes
    }
  }' > /tmp/orbit-graph-baseline-capture.json

echo "ORBIT_GRAPH_BASELINE_CAPTURE_BEGIN"
cat /tmp/orbit-graph-baseline-capture.json
echo "ORBIT_GRAPH_BASELINE_CAPTURE_END"
