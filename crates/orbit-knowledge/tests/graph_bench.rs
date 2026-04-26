use std::fs;
use std::path::Path;
use std::process::Command;

use orbit_knowledge::graph_bench::{
    GraphBenchOptions, ScenarioMetrics, run_benchmark_in_process, run_benchmark_with_child_process,
};
use tempfile::tempdir;

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git command runs");
    assert!(
        status.success(),
        "git {:?} failed in {}",
        args,
        repo.display()
    );
}

fn init_repo(repo: &Path) {
    git(repo, &["init", "-q", "--initial-branch=main"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
}

fn write_file(repo: &Path, rel: &str, content: &str) {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn commit_all(repo: &Path, message: &str) {
    git(repo, &["add", "-A"]);
    let status = Command::new("git")
        .args(["commit", "-q", "-m", message])
        .current_dir(repo)
        .env("GIT_AUTHOR_DATE", "2026-04-26T00:00:00+00:00")
        .env("GIT_COMMITTER_DATE", "2026-04-26T00:00:00+00:00")
        .status()
        .expect("git commit runs");
    assert!(status.success(), "git commit failed in {}", repo.display());
}

#[test]
fn benchmark_tempdir_corpus_writes_parseable_scoreboard_entry() {
    let repo_dir = tempdir().expect("repo tempdir");
    let repo = repo_dir.path();
    init_repo(repo);
    write_file(repo, "src/lib.rs", "pub fn alpha() {}\n");
    write_file(repo, "src/nested.rs", "pub struct Beta;\n");
    write_file(repo, "README.md", "# Bench Corpus\n\nTiny corpus.\n");
    commit_all(repo, "seed graph bench corpus");

    let mut options = GraphBenchOptions::from_workspace(repo);
    options.knowledge_dir = repo.join(".orbit/knowledge");
    options.scoreboard_path = repo.join(".orbit/state/scoreboard/graph_bench.json");

    let outcome = run_benchmark_in_process(&options).expect("benchmark runs");
    assert_eq!(outcome.record.git_sha.len(), 40);
    assert!(outcome.record.logical_core_count > 0);
    assert!(!outcome.record.hostname.is_empty());
    assert!(outcome.record.scenarios.cold_build.wall_time_ms > 0);
    assert!(outcome.record.scenarios.cold_build.file_count >= 3);
    assert!(outcome.record.scenarios.cold_build.dir_count >= 2);
    assert_eq!(
        outcome.record.scenarios.cold_build.file_count,
        outcome.record.scenarios.warm_incremental_noop.file_count
    );

    let raw = fs::read_to_string(&options.scoreboard_path).expect("read scoreboard");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse scoreboard");
    let records = parsed.as_array().expect("scoreboard is array");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert!(
        record
            .get("timestamp")
            .and_then(|value| value.as_str())
            .is_some()
    );
    assert_eq!(
        record
            .pointer("/scenarios/cold_build/file_count")
            .and_then(|value| value.as_u64()),
        Some(outcome.record.scenarios.cold_build.file_count as u64)
    );
}

#[cfg(unix)]
#[test]
fn benchmark_with_child_process_reads_per_scenario_metrics_and_cleans_scratch_dir() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = tempdir().expect("scratch tempdir");
    let workspace = scratch.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace dir");
    let scoreboard_dir = scratch.path().join("scoreboard");
    fs::create_dir_all(&scoreboard_dir).expect("scoreboard dir");
    let scoreboard_path = scoreboard_dir.join("graph_bench.json");

    let fake_child = scratch.path().join("fake_child.sh");
    let script = r#"#!/usr/bin/env bash
set -e
output_path=""
scenario=""
while [ $# -gt 0 ]; do
    case "$1" in
        --child-output) output_path="$2"; shift 2 ;;
        --child-scenario) scenario="$2"; shift 2 ;;
        *) shift ;;
    esac
done

case "$scenario" in
    cold_build) wall=200; files=11; leaves=22; dirs=3 ;;
    warm_incremental_noop) wall=50; files=11; leaves=22; dirs=3 ;;
    *) echo "unknown scenario $scenario" >&2; exit 1 ;;
esac

cat > "$output_path" <<EOF
{
  "wall_time_ms": $wall,
  "peak_rss_kib": null,
  "file_count": $files,
  "leaf_count": $leaves,
  "dir_count": $dirs
}
EOF
"#;
    fs::write(&fake_child, script).expect("write fake child");
    let mut perms = fs::metadata(&fake_child)
        .expect("stat fake child")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake_child, perms).expect("chmod fake child");

    let options = GraphBenchOptions {
        workspace: workspace.clone(),
        knowledge_dir: workspace.join(".orbit/knowledge"),
        scoreboard_path: scoreboard_path.clone(),
    };

    let outcome =
        run_benchmark_with_child_process(&options, &fake_child).expect("subprocess benchmark runs");

    let cold = &outcome.record.scenarios.cold_build;
    let warm = &outcome.record.scenarios.warm_incremental_noop;
    assert_eq!(cold.wall_time_ms, 200);
    assert_eq!(warm.wall_time_ms, 50);
    let expected = ScenarioMetrics {
        wall_time_ms: 200,
        peak_rss_kib: None,
        file_count: 11,
        leaf_count: 22,
        dir_count: 3,
    };
    assert_eq!(cold.file_count, expected.file_count);
    assert_eq!(cold.leaf_count, expected.leaf_count);
    assert_eq!(cold.dir_count, expected.dir_count);
    assert!(cold.peak_rss_kib.is_none());

    let raw = fs::read_to_string(&scoreboard_path).expect("read scoreboard");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse scoreboard");
    assert_eq!(parsed.as_array().expect("array").len(), 1);

    let leftovers: Vec<_> = fs::read_dir(&scoreboard_dir)
        .expect("read scoreboard dir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".graph_bench_tmp_")
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "scratch dir was not cleaned up: {:?}",
        leftovers.iter().map(|e| e.path()).collect::<Vec<_>>()
    );
}
