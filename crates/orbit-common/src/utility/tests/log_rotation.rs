//! [ORB-00415] Rotation + retention tests for the global JSONL tracing feed.

use std::io::Write;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use crate::utility::fs::append_private_file;
use crate::utility::log_rotation::{
    LogRotationConfig, PruneReason, maybe_roll, plan_prune, rotate_and_report,
};

/// The archive file names the retention classifier would select, paired with
/// the reason, for readable assertions.
fn planned(
    active: &std::path::Path,
    retention: Duration,
    max_total_bytes: u64,
) -> Vec<(String, PruneReason)> {
    plan_prune(active, retention, max_total_bytes, SystemTime::now())
        .expect("plan")
        .candidates
        .into_iter()
        .map(|candidate| {
            (
                candidate
                    .path
                    .file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .into_owned(),
                candidate.reason,
            )
        })
        .collect()
}

fn archive_names(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("read_dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("orbit.jsonl."))
        .collect()
}

fn backdate(path: &std::path::Path, ago: Duration) {
    let when = SystemTime::now().checked_sub(ago).expect("valid time");
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open for mtime")
        .set_modified(when)
        .expect("set mtime");
}

#[test]
fn oversized_active_file_rolls_to_dated_archive_preserving_old_content() {
    let dir = TempDir::new().expect("tempdir");
    let active = dir.path().join("orbit.jsonl");
    let old_content = "OLD-LINE\n".repeat(20); // ~180 bytes
    std::fs::write(&active, &old_content).expect("write active");

    let config = LogRotationConfig {
        retention_days: 7,
        max_total_bytes: 10_000_000,
        max_file_bytes: 50,
    };
    maybe_roll(&active, &config).expect("roll");

    assert!(!active.exists(), "active file should have been rolled away");
    let archives = archive_names(dir.path());
    assert_eq!(
        archives.len(),
        1,
        "expected one dated archive, got {archives:?}"
    );
    let archived = std::fs::read_to_string(dir.path().join(&archives[0])).expect("read archive");
    assert_eq!(
        archived, old_content,
        "old content must be preserved in the archive"
    );

    // The subscriber reopening the fixed active path creates a fresh file at
    // the boundary.
    std::fs::write(&active, "NEW-LINE\n").expect("reopen active");
    assert!(active.exists());
    assert_eq!(
        std::fs::read_to_string(&active).expect("read active"),
        "NEW-LINE\n"
    );
}

#[test]
fn within_budget_active_file_is_not_rolled() {
    let dir = TempDir::new().expect("tempdir");
    let active = dir.path().join("orbit.jsonl");
    std::fs::write(&active, "small\n").expect("write active");

    let config = LogRotationConfig {
        retention_days: 7,
        max_total_bytes: 10_000_000,
        max_file_bytes: 1_000,
    };
    maybe_roll(&active, &config).expect("roll");

    assert!(active.exists(), "within-budget file must not be rolled");
    assert!(
        archive_names(dir.path()).is_empty(),
        "no archive should be created"
    );
}

#[test]
fn classifier_selects_archives_older_than_age_budget() {
    let dir = TempDir::new().expect("tempdir");
    let active = dir.path().join("orbit.jsonl");
    let old = dir.path().join("orbit.jsonl.OLD");
    let recent = dir.path().join("orbit.jsonl.RECENT");
    std::fs::write(&old, "old").expect("write old");
    std::fs::write(&recent, "recent").expect("write recent");
    backdate(&old, Duration::from_secs(10 * 86_400)); // 10 days old

    let selected = planned(&active, Duration::from_secs(7 * 86_400), 10_000_000);
    assert_eq!(
        selected,
        vec![("orbit.jsonl.OLD".to_string(), PruneReason::Age)],
        "only the over-age archive is selected, by age"
    );
    // Classification is pure: nothing is deleted.
    assert!(old.exists(), "the classifier must not delete anything");
    assert!(recent.exists(), "recent archive is untouched");
}

#[test]
fn classifier_selects_oldest_archives_beyond_total_size_budget() {
    let dir = TempDir::new().expect("tempdir");
    let active = dir.path().join("orbit.jsonl");
    let a = dir.path().join("orbit.jsonl.A");
    let b = dir.path().join("orbit.jsonl.B");
    let c = dir.path().join("orbit.jsonl.C");
    for (path, secs_ago) in [(&a, 30u64), (&b, 20), (&c, 10)] {
        std::fs::write(path, "x".repeat(100)).expect("write archive");
        backdate(path, Duration::from_secs(secs_ago));
    }

    // 300 bytes total, budget 150: select oldest (A) -> 200, then B -> 100.
    let selected = planned(&active, Duration::from_secs(3_650 * 86_400), 150);
    assert_eq!(
        selected,
        vec![
            ("orbit.jsonl.A".to_string(), PruneReason::Size),
            ("orbit.jsonl.B".to_string(), PruneReason::Size),
        ],
        "the two oldest archives are size-selected, oldest first"
    );
    // Pure classification: every archive survives the plan.
    assert!(a.exists() && b.exists() && c.exists());
}

#[test]
fn rotate_and_report_is_a_noop_when_nothing_to_do() {
    let dir = TempDir::new().expect("tempdir");
    let active = dir.path().join("orbit.jsonl");
    std::fs::write(&active, "line\n").expect("write");
    // Generous budgets -> no roll, no report action, no panic even with no
    // archives.
    rotate_and_report(&active, &LogRotationConfig::default());
    assert!(active.exists());
    assert!(archive_names(dir.path()).is_empty());
}

#[test]
fn startup_rotation_never_deletes_archives() {
    // [ORB-10184] Regression: subscriber-init rotation is non-destructive. It
    // may roll an oversized active file (rename, non-destructive) but must
    // never unlink an archive — deletion belongs to `orbit gc logs --apply`.
    let dir = TempDir::new().expect("tempdir");
    let active = dir.path().join("orbit.jsonl");
    // An oversized active file so the roll path is exercised too.
    std::fs::write(&active, "x".repeat(500)).expect("write active");
    // Archives that BOTH the age and size budgets would otherwise reclaim.
    let over_age = dir.path().join("orbit.jsonl.OVERAGE");
    let over_size = dir.path().join("orbit.jsonl.OVERSIZE");
    std::fs::write(&over_age, "x".repeat(100)).expect("write over-age");
    std::fs::write(&over_size, "x".repeat(100)).expect("write over-size");
    backdate(&over_age, Duration::from_secs(30 * 86_400));

    let config = LogRotationConfig {
        retention_days: 7,
        max_total_bytes: 50, // far below the archive footprint
        max_file_bytes: 100, // active (500 bytes) is oversized -> rolls
    };
    rotate_and_report(&active, &config);

    // The roll happened (non-destructive rename), but no pre-existing archive
    // was deleted.
    assert!(
        over_age.exists(),
        "an over-age archive must survive startup rotation"
    );
    assert!(
        over_size.exists(),
        "an over-size archive must survive startup rotation"
    );
    // Sanity: the classifier would indeed have flagged these, proving the
    // no-delete guarantee is not vacuous.
    assert!(
        !planned(&active, config.retention_window(), config.max_total_bytes).is_empty(),
        "the archives were genuinely reclaim-eligible"
    );
}

#[test]
fn concurrent_writers_do_not_corrupt_jsonl_lines() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("orbit.jsonl");
    const PER_WRITER: usize = 300;

    let handles: Vec<_> = (0..2)
        .map(|writer_id| {
            let path = path.clone();
            std::thread::spawn(move || {
                for seq in 0..PER_WRITER {
                    // Re-open per line (O_APPEND) to mirror separate processes
                    // appending concurrently; short lines stay under PIPE_BUF.
                    let mut file = append_private_file(&path).expect("append open");
                    let mut line = serde_json::to_string(
                        &serde_json::json!({"writer": writer_id, "seq": seq}),
                    )
                    .expect("serialize");
                    // One write() per line, newline included — mirrors the fmt
                    // layer, which write_all()s the whole formatted event.
                    // (`writeln!` issues a second syscall for the `\n`, which
                    // can interleave between concurrent appenders.)
                    line.push('\n');
                    file.write_all(line.as_bytes()).expect("write line");
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("join writer");
    }

    let content = std::fs::read_to_string(&path).expect("read log");
    let mut count = 0;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|error| panic!("corrupt interleaved line `{line}`: {error}"));
        count += 1;
    }
    assert_eq!(
        count,
        2 * PER_WRITER,
        "every line from both writers must parse"
    );
}

#[test]
fn from_parts_validates_and_converts_units() {
    assert_eq!(
        LogRotationConfig::from_parts(None, None, None).expect("defaults"),
        LogRotationConfig::default()
    );

    let config = LogRotationConfig::from_parts(Some(3), Some(200), Some(50)).expect("valid");
    assert_eq!(config.retention_days, 3);
    assert_eq!(config.max_total_bytes, 200 * 1024 * 1024);
    assert_eq!(config.max_file_bytes, 50 * 1024 * 1024);

    assert!(LogRotationConfig::from_parts(Some(0), None, None).is_err());
    assert!(LogRotationConfig::from_parts(None, Some(0), None).is_err());
    assert!(LogRotationConfig::from_parts(None, None, Some(0)).is_err());
    assert!(
        LogRotationConfig::from_parts(None, Some(10), Some(50)).is_err(),
        "a per-file budget larger than the total budget must be rejected"
    );
}
