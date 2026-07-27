use std::path::Path;

use tempfile::tempdir;

use super::super::catalog::{CatalogDirectoryList, V2ActivityCatalog, V2JobCatalog};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    First,
    Duplicate,
}

fn write(path: &Path, yaml: &str) {
    std::fs::create_dir_all(path.parent().expect("asset path has parent"))
        .expect("create asset dir");
    std::fs::write(path, yaml).expect("write asset");
}

#[test]
fn directory_list_keeps_first_occurrence_of_a_path() {
    let root = tempdir().expect("create tempdir");
    let path = root.path().join("catalog");
    std::fs::create_dir_all(&path).expect("create catalog dir");
    let mut dirs = CatalogDirectoryList::default();

    dirs.push(path.clone(), Layer::First);
    dirs.push(path.clone(), Layer::Duplicate);

    let dirs = dirs.into_vec();
    assert_eq!(dirs.len(), 1);
    assert_eq!(dirs[0].path(), path);
    assert_eq!(dirs[0].kind(), &Layer::First);
}

#[test]
fn typed_catalog_adapters_share_first_wins_layering() {
    let root = tempdir().expect("create tempdir");
    let high_activities = root.path().join("high-activities");
    let low_activities = root.path().join("low-activities");
    let high_jobs = root.path().join("high-jobs");
    let low_jobs = root.path().join("low-jobs");
    write(
        &high_activities.join("activity.yaml"),
        "schemaVersion: 2\nkind: Activity\nmetadata:\n  name: layered\nspec:\n  type: deterministic\n  description: high\n  action: high\n  config: {}\n",
    );
    write(
        &low_activities.join("activity.yaml"),
        "schemaVersion: 2\nkind: Activity\nmetadata:\n  name: layered\nspec:\n  type: deterministic\n  description: low\n  action: low\n  config: {}\n",
    );
    write(
        &high_jobs.join("job.yaml"),
        "schemaVersion: 2\nkind: Job\nmetadata:\n  name: layered\nspec:\n  state: enabled\n  kind: workflow\n  max_active_runs: 9\n  steps: []\n",
    );
    write(
        &low_jobs.join("job.yaml"),
        "schemaVersion: 2\nkind: Job\nmetadata:\n  name: layered\nspec:\n  state: enabled\n  kind: workflow\n  max_active_runs: 1\n  steps: []\n",
    );

    let mut activities = V2ActivityCatalog::new();
    activities
        .load_dir_skipping_retired_prefer_existing(&high_activities)
        .expect("load high activity layer");
    activities
        .load_dir_skipping_retired_prefer_existing(&low_activities)
        .expect("load low activity layer");
    assert_eq!(
        activities
            .get("layered")
            .map(|activity| activity.description.as_str()),
        Some("high")
    );

    let mut jobs = V2JobCatalog::new();
    jobs.load_dir_prefer_existing(&high_jobs)
        .expect("load high job layer");
    jobs.load_dir_prefer_existing(&low_jobs)
        .expect("load low job layer");
    assert_eq!(
        jobs.get("layered").map(|(_, job)| job.max_active_runs),
        Some(9)
    );
}
