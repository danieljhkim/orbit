#![allow(missing_docs)]

use crate::graph_bench::{
    GraphBenchRecord, GraphBenchScenarios, SCOREBOARD_CAP, ScenarioMetrics, append_scoreboard,
    format_summary, load_scoreboard,
};

fn metrics(value: u64) -> ScenarioMetrics {
    ScenarioMetrics {
        wall_time_ms: value,
        peak_rss_kib: Some(value * 10),
        file_count: value as usize,
        leaf_count: value as usize + 1,
        dir_count: value as usize + 2,
    }
}

fn record(index: u64) -> GraphBenchRecord {
    GraphBenchRecord {
        timestamp: format!("2026-04-26T00:{index:02}:00Z"),
        git_sha: format!("sha-{index}"),
        hostname: "test-host".to_string(),
        logical_core_count: 8,
        scenarios: GraphBenchScenarios {
            cold_build: metrics(index),
            warm_incremental_noop: metrics(index + 1),
        },
    }
}

#[test]
fn scoreboard_is_capped_and_prunes_oldest_records() {
    let dir = tempfile::tempdir().expect("scoreboard tempdir");
    let path = dir.path().join("graph_bench.json");

    for index in 0..201 {
        append_scoreboard(&path, record(index)).expect("append scoreboard record");
    }

    let records = load_scoreboard(&path).expect("load capped scoreboard");
    assert_eq!(records.len(), SCOREBOARD_CAP);
    assert_eq!(records.first().unwrap().git_sha, "sha-1");
    assert_eq!(records.last().unwrap().git_sha, "sha-200");
}

#[test]
fn summary_prints_baseline_and_prior_deltas() {
    let baseline = format_summary(&record(10), None);
    assert!(baseline.contains("cold_build: 10ms (baseline)"));

    let previous = record(10);
    let current = record(15);
    let delta = format_summary(&current, Some(&previous));
    assert!(delta.contains("cold_build: 15ms (+50% vs last)"));
    assert!(delta.contains("files 15 (+50% vs last)"));
}
