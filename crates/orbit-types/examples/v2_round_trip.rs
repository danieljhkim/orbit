//! Round-trip smoke for v2 reference YAMLs (closes T20260418-2010 AC1).
//!
//! For each v2 reference under `crates/orbit-core/assets/activities/v2_reference/`:
//!   1. Load the YAML through `load_activity_asset` → `ActivityV2`.
//!   2. Re-serialize via `ResourceEnvelope<ActivityV2>` back to YAML.
//!   3. Parse both the source YAML and the re-serialized YAML as `serde_yaml::Value`s.
//!   4. Assert the two `Value`s compare equal (**semantic** round-trip — stricter
//!      byte-identical YAML would require controlling quoting/ordering choices
//!      in `serde_yaml`'s emitter; the AC bar is "equal modulo trailing
//!      whitespace" which we interpret as semantic equality).
//!
//! Also covers the kind-mismatch validation case (AC11): attempts to load an
//! in-memory YAML with `kind: Job` as an Activity and asserts the loader
//! returns `AssetLoadError::KindMismatch`.
//!
//! Usage:
//!     cargo run -p orbit-types --example v2_round_trip

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use orbit_types::v2::{ActivityAsset, load_activity_asset};
use orbit_types::{ActivityV2, ResourceEnvelope, ResourceKind, ResourceMetadata};

fn main() -> ExitCode {
    let mut failures: Vec<String> = Vec::new();

    for path in v2_reference_paths() {
        let source = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(err) => {
                failures.push(format!("{}: read: {err}", path.display()));
                continue;
            }
        };
        let asset = match load_activity_asset(&source) {
            Ok(ActivityAsset::V2(a)) => a,
            Ok(ActivityAsset::V1(_)) => {
                failures.push(format!("{}: parsed as v1, expected v2", path.display()));
                continue;
            }
            Err(err) => {
                failures.push(format!("{}: load: {err}", path.display()));
                continue;
            }
        };

        let envelope = ResourceEnvelope::<ActivityV2> {
            schema_version: 2,
            kind: ResourceKind::Activity,
            metadata: ResourceMetadata::named(asset.name.clone()),
            spec: asset.spec.clone(),
        };
        let re_serialized = match serde_yaml::to_string(&envelope) {
            Ok(s) => s,
            Err(err) => {
                failures.push(format!("{}: serialize: {err}", path.display()));
                continue;
            }
        };

        let source_value: serde_yaml::Value = match serde_yaml::from_str(&source) {
            Ok(v) => v,
            Err(err) => {
                failures.push(format!("{}: parse source: {err}", path.display()));
                continue;
            }
        };
        let round_value: serde_yaml::Value = match serde_yaml::from_str(&re_serialized) {
            Ok(v) => v,
            Err(err) => {
                failures.push(format!("{}: parse round: {err}", path.display()));
                continue;
            }
        };

        if source_value != round_value {
            failures.push(format!(
                "{}: semantic round-trip mismatch\n--- source ---\n{}\n--- round ---\n{}",
                path.display(),
                to_sorted_yaml(&source_value),
                to_sorted_yaml(&round_value),
            ));
        } else {
            println!("round-trip OK: {}", path.display());
        }
    }

    // AC11 kind-mismatch smoke.
    let mismatch_yaml = r#"schemaVersion: 2
kind: Job
metadata:
  name: mismatched
spec:
  type: shell
  description: oops
  program: echo
  args: []
  allowed_programs: [echo]
"#;
    match load_activity_asset(mismatch_yaml) {
        Err(orbit_types::v2::AssetLoadError::KindMismatch { expected, actual }) => {
            println!(
                "kind-mismatch smoke OK: expected={} actual={}",
                expected, actual
            );
        }
        Ok(_) => failures.push("kind-mismatch smoke: load succeeded but should have failed".into()),
        Err(err) => failures.push(format!("kind-mismatch smoke: wrong error variant: {err}")),
    }

    if failures.is_empty() {
        println!("\nall v2 round-trip + kind-mismatch smokes passed");
        ExitCode::SUCCESS
    } else {
        eprintln!("\n{} failure(s):", failures.len());
        for f in &failures {
            eprintln!("  - {f}");
        }
        ExitCode::FAILURE
    }
}

fn v2_reference_paths() -> Vec<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root");
    let dir = root.join("crates/orbit-core/assets/activities/v2_reference");
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("yaml") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn to_sorted_yaml(v: &serde_yaml::Value) -> String {
    serde_yaml::to_string(v).unwrap_or_default()
}
