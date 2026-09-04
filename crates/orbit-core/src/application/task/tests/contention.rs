use crate::application::task::contention::{LockSurface, compute_contention};

fn surface(task_id: &str, selectors: &[&str]) -> LockSurface {
    LockSurface {
        task_id: task_id.to_string(),
        selectors: selectors.iter().map(|s| (*s).to_string()).collect(),
    }
}

#[test]
fn a_selector_only_one_task_claims_is_not_contention() {
    let report = compute_contention(&[
        surface("T-1", &["file:src/a.rs"]),
        surface("T-2", &["file:src/b.rs"]),
    ]);

    assert!(report.hotspots.is_empty());
    assert_eq!(report.constrained, 2);
    assert_eq!(report.groups, 2, "disjoint tasks are separate clusters");
    assert_eq!(report.largest_group, 1);
    assert_eq!(report.parallel_floor(), 2);
}

#[test]
fn two_tasks_on_one_file_contend_and_form_a_single_cluster() {
    let report = compute_contention(&[
        surface("T-1", &["file:src/a.rs"]),
        surface("T-2", &["file:src/a.rs"]),
    ]);

    assert_eq!(report.hotspots.len(), 1);
    assert_eq!(report.hotspots[0].selector, "file:src/a.rs");
    assert_eq!(report.hotspots[0].task_ids, vec!["T-1", "T-2"]);
    assert_eq!(report.groups, 1);
    assert_eq!(report.largest_group, 2);
    assert_eq!(report.parallel_floor(), 1);
}

#[test]
fn contention_follows_selector_containment_not_string_equality() {
    // `dir:src` covers everything beneath it, so these two tasks conflict even
    // though neither declares the other's selector.
    let report = compute_contention(&[
        surface("T-1", &["dir:src"]),
        surface("T-2", &["file:src/nested/deep.rs"]),
    ]);

    assert_eq!(
        report.groups, 1,
        "a directory claim reaches its descendants"
    );
    assert_eq!(report.largest_group, 2);
    assert_eq!(
        report.hotspots.len(),
        2,
        "both selectors are contended: each overlaps a surface held by the other task"
    );
}

#[test]
fn a_cluster_links_transitively_through_a_shared_middle_task() {
    // A and C never touch the same file; B touches both. All three land in one
    // cluster, which is why the group count is a floor on parallelism and not
    // the achievable maximum — A and C could in fact run together.
    let report = compute_contention(&[
        surface("T-A", &["file:src/a.rs"]),
        surface("T-B", &["file:src/a.rs", "file:src/c.rs"]),
        surface("T-C", &["file:src/c.rs"]),
    ]);

    assert_eq!(report.groups, 1);
    assert_eq!(report.largest_group, 3);
    assert_eq!(report.parallel_floor(), 1);
}

#[test]
fn a_task_declaring_nothing_contends_with_no_one_and_raises_the_floor() {
    let report = compute_contention(&[
        surface("T-1", &["file:src/a.rs"]),
        surface("T-2", &["file:src/a.rs"]),
        surface("T-3", &[]),
    ]);

    assert_eq!(report.constrained, 2);
    assert_eq!(report.unconstrained, 1);
    assert_eq!(report.pending(), 3);
    assert_eq!(report.groups, 1);
    assert_eq!(
        report.parallel_floor(),
        2,
        "one per cluster, plus the task that locks nothing"
    );
}

#[test]
fn hotspots_rank_by_contention_then_by_selector() {
    let report = compute_contention(&[
        surface("T-1", &["file:src/hot.rs", "file:src/b.rs"]),
        surface("T-2", &["file:src/hot.rs", "file:src/a.rs"]),
        surface("T-3", &["file:src/hot.rs"]),
        surface("T-4", &["file:src/a.rs"]),
        surface("T-5", &["file:src/b.rs"]),
    ]);

    let ranked: Vec<(&str, usize)> = report
        .hotspots
        .iter()
        .map(|hotspot| (hotspot.selector.as_str(), hotspot.tasks()))
        .collect();
    assert_eq!(
        ranked,
        vec![
            ("file:src/hot.rs", 3),
            ("file:src/a.rs", 2),
            ("file:src/b.rs", 2),
        ],
        "most contended first; the two-task rows tie and fall back to selector order"
    );
}

#[test]
fn an_empty_population_reports_nothing_rather_than_a_clean_bill() {
    let report = compute_contention(&[]);

    assert_eq!(report.pending(), 0);
    assert_eq!(report.groups, 0);
    assert_eq!(report.largest_group, 0);
    assert_eq!(report.parallel_floor(), 0);
    assert!(report.hotspots.is_empty());
}

#[test]
fn every_task_sharing_one_file_collapses_the_floor_to_one() {
    let surfaces: Vec<LockSurface> = (1..=6)
        .map(|index| LockSurface {
            task_id: format!("T-{index}"),
            selectors: vec!["file:src/registry.rs".to_string()],
        })
        .collect();

    let report = compute_contention(&surfaces);

    assert_eq!(report.hotspots.len(), 1);
    assert_eq!(report.hotspots[0].tasks(), 6);
    assert_eq!(report.groups, 1);
    assert_eq!(report.largest_group, 6);
    assert_eq!(
        report.parallel_floor(),
        1,
        "a single file serializes them all"
    );
}
