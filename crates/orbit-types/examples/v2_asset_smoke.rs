//! Smoke: load every v1 activity + job YAML through the v2 asset loader and
//! parse every v2 reference activity YAML. Used as the AC2 / AC1 validation
//! path per the Phase 2 plan in T20260418-2010.
//!
//! Usage:
//!     cargo run -p orbit-types --example v2_asset_smoke
//!
//! Exits non-zero if any file fails to parse.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use orbit_types::v2::{ActivityAsset, JobAsset, load_activity_asset, load_job_asset};

fn main() -> ExitCode {
    let root = workspace_root();
    let activities_dir = root.join("crates/orbit-core/assets/activities");
    let jobs_dir = root.join("crates/orbit-core/assets/jobs");

    let mut failures: Vec<String> = Vec::new();
    let mut counts = Counts::default();

    for path in walk_yaml(&activities_dir) {
        match fs::read_to_string(&path).and_then(|s| Ok((path.clone(), s))) {
            Ok((path, yaml)) => match load_activity_asset(&yaml) {
                Ok(ActivityAsset::V1(_)) => counts.activity_v1 += 1,
                Ok(ActivityAsset::V2(_)) => counts.activity_v2 += 1,
                Err(err) => failures.push(format!("{}: {}", path.display(), err)),
            },
            Err(err) => failures.push(format!("{}: {}", path.display(), err)),
        }
    }

    for path in walk_yaml(&jobs_dir) {
        match fs::read_to_string(&path).and_then(|s| Ok((path.clone(), s))) {
            Ok((path, yaml)) => match load_job_asset(&yaml) {
                Ok(JobAsset::V1(_)) => counts.job_v1 += 1,
                Ok(JobAsset::V2(_)) => counts.job_v2 += 1,
                Err(err) => failures.push(format!("{}: {}", path.display(), err)),
            },
            Err(err) => failures.push(format!("{}: {}", path.display(), err)),
        }
    }

    println!("v1 activities loaded: {}", counts.activity_v1);
    println!("v2 activities loaded: {}", counts.activity_v2);
    println!("v1 jobs loaded:       {}", counts.job_v1);
    println!("v2 jobs loaded:       {}", counts.job_v2);

    if failures.is_empty() {
        println!("all assets loaded without error");
        ExitCode::SUCCESS
    } else {
        eprintln!("\n{} asset(s) failed to load:", failures.len());
        for failure in &failures {
            eprintln!("  - {}", failure);
        }
        ExitCode::FAILURE
    }
}

#[derive(Default)]
struct Counts {
    activity_v1: usize,
    activity_v2: usize,
    job_v1: usize,
    job_v2: usize,
}

fn walk_yaml(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk_yaml(&path));
            } else if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                out.push(path);
            }
        }
    }
    out
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at the orbit-types crate; workspace root is two up.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}
