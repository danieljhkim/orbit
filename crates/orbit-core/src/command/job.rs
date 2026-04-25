use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orbit_common::types::{JobKind, JobRun, JobScheduleState, JobV2, OrbitError, load_job_asset};
use orbit_common::utility::fs::write_text_with_parent;
use serde_json::Value;

use crate::OrbitRuntime;

/// Shippable default workflow assets, seeded under
/// `<orbit_root>/resources/jobs/<name>.yaml` on `orbit init`. The five
/// entries here are the admission-controlled task shipment workflows
/// (auto / epic / gate / local / pr). Example and smoke fixtures live
/// under `crates/orbit-core/assets/jobs/examples/` and are NOT seeded —
/// they exist for `crates/orbit-engine/examples/v2_job_runtime_smoke.rs`
/// only.
const DEFAULT_JOB_FILES: &[(&str, &str)] = &[
    (
        "task_auto_pipeline",
        include_str!("../../assets/jobs/task_auto_pipeline.yaml"),
    ),
    (
        "task_epic_pipeline",
        include_str!("../../assets/jobs/task_epic_pipeline.yaml"),
    ),
    (
        "task_gate_pipeline",
        include_str!("../../assets/jobs/task_gate_pipeline.yaml"),
    ),
    (
        "task_local_pipeline",
        include_str!("../../assets/jobs/task_local_pipeline.yaml"),
    ),
    (
        "task_pr_pipeline",
        include_str!("../../assets/jobs/task_pr_pipeline.yaml"),
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobCatalogFilter {
    WorkflowsOnly,
    All,
    Kind(JobKind),
}

#[derive(Debug, Clone)]
pub struct JobCatalogEntry {
    pub job_id: String,
    pub path: PathBuf,
    pub spec: JobV2,
}

impl JobCatalogEntry {
    pub fn kind(&self) -> JobKind {
        self.spec.kind
    }

    pub fn state(&self) -> JobScheduleState {
        self.spec.state
    }

    pub fn max_active_runs(&self) -> u32 {
        self.spec.max_active_runs
    }

    pub fn default_input(&self) -> Option<&Value> {
        self.spec.default_input.as_ref()
    }
}

#[derive(Debug, Clone)]
struct V2JobAssetEntry {
    path: PathBuf,
    spec: JobV2,
}

impl OrbitRuntime {
    pub fn list_job_catalog_with_last_run(
        &self,
        include_disabled: bool,
        filter: JobCatalogFilter,
    ) -> Result<Vec<(JobCatalogEntry, Option<JobRun>)>, OrbitError> {
        use orbit_store::JobRunQuery;

        let v2_jobs = self.load_v2_job_assets()?;
        let mut result = Vec::new();

        for (job_id, asset) in &v2_jobs {
            if !include_disabled && asset.spec.state == JobScheduleState::Disabled {
                continue;
            }
            if !matches_job_filter(asset.spec.kind, filter) {
                continue;
            }
            let last_run = self
                .stores()
                .jobs()
                .list_runs_filtered(&JobRunQuery {
                    job_id: Some(job_id.clone()),
                    state: None,
                    created_since: None,
                    limit: Some(1),
                })
                .ok()
                .and_then(|runs| runs.into_iter().next());
            result.push((
                JobCatalogEntry {
                    job_id: job_id.clone(),
                    path: asset.path.clone(),
                    spec: asset.spec.clone(),
                },
                last_run,
            ));
        }

        result.sort_by(|left, right| left.0.job_id.cmp(&right.0.job_id));
        Ok(result)
    }

    pub fn show_job_catalog_entry(&self, job_id: &str) -> Result<JobCatalogEntry, OrbitError> {
        let v2_jobs = self.load_v2_job_assets()?;
        v2_jobs
            .get(job_id)
            .map(|asset| JobCatalogEntry {
                job_id: job_id.to_string(),
                path: asset.path.clone(),
                spec: asset.spec.clone(),
            })
            .ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))
    }

    fn load_v2_job_assets(&self) -> Result<BTreeMap<String, V2JobAssetEntry>, OrbitError> {
        let mut entries = BTreeMap::new();
        for dir in self.v2_job_asset_dirs() {
            if dir.is_dir() {
                load_v2_job_assets_from_dir(&dir, &mut entries)?;
            }
        }
        Ok(entries)
    }

    fn v2_job_asset_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
        let push_unique = |dirs: &mut Vec<PathBuf>,
                           seen: &mut std::collections::BTreeSet<PathBuf>,
                           path: PathBuf| {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if seen.insert(canonical) {
                dirs.push(path);
            }
        };

        let env_dirs = std::env::var("ORBIT_JOB_DIR")
            .ok()
            .or_else(|| std::env::var("ORBIT_V2_JOB_DIR").ok());
        if let Some(raw) = env_dirs {
            for entry in raw.split(':').filter(|value| !value.is_empty()) {
                push_unique(&mut dirs, &mut seen, PathBuf::from(entry));
            }
        }

        push_unique(&mut dirs, &mut seen, self.paths().jobs_dir.clone());
        push_unique(
            &mut dirs,
            &mut seen,
            self.paths().global_dir.join("resources/jobs"),
        );
        dirs
    }

    pub(crate) fn load_v2_job_asset_by_name(
        &self,
        job_id: &str,
    ) -> Result<(PathBuf, JobV2), OrbitError> {
        let mut selected = None;
        for dir in self.v2_job_asset_dirs() {
            if dir.is_dir() {
                if let Some(found) = find_v2_job_asset_in_dir(&dir, job_id)?
                    && selected.is_none()
                {
                    selected = Some(found);
                }
            }
        }
        selected.ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))
    }
}

fn matches_job_filter(kind: JobKind, filter: JobCatalogFilter) -> bool {
    match filter {
        JobCatalogFilter::WorkflowsOnly => kind == JobKind::Workflow,
        JobCatalogFilter::All => true,
        JobCatalogFilter::Kind(expected) => kind == expected,
    }
}

fn load_v2_job_assets_from_dir(
    dir: &Path,
    entries: &mut BTreeMap<String, V2JobAssetEntry>,
) -> Result<(), OrbitError> {
    let mut local_entries = BTreeMap::new();
    let mut local_sources = BTreeMap::new();
    collect_v2_job_assets_from_dir(dir, &mut local_entries, &mut local_sources)?;

    // v2_job_asset_dirs() is ordered from highest to lowest precedence.
    // Keep the first entry for each name, while still rejecting duplicates
    // inside an individual directory tree above.
    for (name, asset) in local_entries {
        entries.entry(name).or_insert(asset);
    }

    Ok(())
}

fn collect_v2_job_assets_from_dir(
    dir: &Path,
    entries: &mut BTreeMap<String, V2JobAssetEntry>,
    sources: &mut BTreeMap<String, PathBuf>,
) -> Result<(), OrbitError> {
    let iter = std::fs::read_dir(dir)
        .map_err(|err| OrbitError::InvalidInput(format!("read dir {}: {err}", dir.display())))?;
    for entry in iter {
        let entry = entry.map_err(|err| {
            OrbitError::InvalidInput(format!("read dir {}: {err}", dir.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_v2_job_assets_from_dir(&path, entries, sources)?;
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "yaml" || ext == "yml");
        if !is_yaml {
            continue;
        }
        let yaml = std::fs::read_to_string(&path).map_err(|err| {
            OrbitError::InvalidInput(format!("read file {}: {err}", path.display()))
        })?;
        let asset = load_job_asset(&yaml)
            .map_err(|err| OrbitError::InvalidInput(format!("parse {}: {err}", path.display())))?;
        if let Some(first) = sources.get(&asset.name) {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate v2 job name '{}' — defined in both {} and {}",
                asset.name,
                first.display(),
                path.display()
            )));
        }
        sources.insert(asset.name.clone(), path.clone());
        entries.insert(
            asset.name,
            V2JobAssetEntry {
                path,
                spec: asset.spec,
            },
        );
    }
    Ok(())
}

fn find_v2_job_asset_in_dir(
    dir: &Path,
    expected_job_id: &str,
) -> Result<Option<(PathBuf, JobV2)>, OrbitError> {
    let mut found = None;
    find_v2_job_asset_in_dir_inner(dir, expected_job_id, &mut found)?;
    Ok(found)
}

fn find_v2_job_asset_in_dir_inner(
    dir: &Path,
    expected_job_id: &str,
    found: &mut Option<(PathBuf, JobV2)>,
) -> Result<(), OrbitError> {
    let iter = std::fs::read_dir(dir)
        .map_err(|err| OrbitError::InvalidInput(format!("read dir {}: {err}", dir.display())))?;
    for entry in iter {
        let entry = entry.map_err(|err| {
            OrbitError::InvalidInput(format!("read dir {}: {err}", dir.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            find_v2_job_asset_in_dir_inner(&path, expected_job_id, found)?;
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "yaml" || ext == "yml");
        if !is_yaml {
            continue;
        }

        let yaml = match std::fs::read_to_string(&path) {
            Ok(yaml) => yaml,
            Err(_) => continue,
        };
        let asset = match load_job_asset(&yaml) {
            Ok(asset) => asset,
            Err(_) => continue,
        };
        if asset.name != expected_job_id {
            continue;
        }
        if let Some((first_path, _)) = found {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate v2 job name '{}' — defined in both {} and {}",
                expected_job_id,
                first_path.display(),
                path.display()
            )));
        }
        *found = Some((path, asset.spec));
    }
    Ok(())
}

/// Seed every entry in [`DEFAULT_JOB_FILES`] as a YAML file under
/// `jobs_dir`. Mirrors the activity / skill / policy seeding pattern:
/// the workflow YAML is embedded in the binary via `include_str!` and
/// copied out on `orbit init` so the job loader can discover it without
/// depending on a git checkout of this repo.
///
/// When `overwrite` is false, existing files are preserved — users who've
/// edited a previously-seeded workflow won't lose their changes on re-init.
pub(crate) fn seed_default_jobs(jobs_dir: &Path, overwrite: bool) -> Result<usize, OrbitError> {
    let mut count = 0usize;
    for (name, content) in DEFAULT_JOB_FILES {
        let path = jobs_dir.join(format!("{name}.yaml"));
        if !overwrite && path.exists() {
            continue;
        }
        write_text_with_parent(&path, content)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime, global_root, workspace_root)
    }

    fn write_job(path: &Path, name: &str, action: &str, max_active_runs: u32) {
        let yaml = format!(
            r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  max_active_runs: {max_active_runs}
  steps:
    - id: marker
      spec:
        type: deterministic
        action: {action}
        config: {{}}
"#
        );
        std::fs::create_dir_all(path.parent().expect("job path has parent"))
            .expect("create job dir");
        std::fs::write(path, yaml).expect("write job yaml");
    }

    #[test]
    fn workspace_job_overrides_global_default_in_catalog_listing() {
        let (_root, runtime, global_root, workspace_root) = test_runtime();
        let global_job = global_root.join("resources/jobs/task_auto_pipeline.yaml");
        let workspace_job = workspace_root.join("resources/jobs/task_auto_pipeline.yaml");
        write_job(&global_job, "task_auto_pipeline", "global_action", 1);
        write_job(&workspace_job, "task_auto_pipeline", "workspace_action", 7);

        let jobs = runtime
            .list_job_catalog_with_last_run(true, JobCatalogFilter::All)
            .expect("list job catalog");
        let matches = jobs
            .iter()
            .filter(|(entry, _)| entry.job_id == "task_auto_pipeline")
            .collect::<Vec<_>>();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0.path, workspace_job);
        assert_eq!(matches[0].0.spec.max_active_runs, 7);
    }

    #[test]
    fn workspace_job_overrides_global_default_in_name_lookup() {
        let (_root, runtime, global_root, workspace_root) = test_runtime();
        let global_job = global_root.join("resources/jobs/task_auto_pipeline.yaml");
        let workspace_job = workspace_root.join("resources/jobs/task_auto_pipeline.yaml");
        write_job(&global_job, "task_auto_pipeline", "global_action", 1);
        write_job(&workspace_job, "task_auto_pipeline", "workspace_action", 7);

        let entry = runtime
            .show_job_catalog_entry("task_auto_pipeline")
            .expect("catalog entry");
        assert_eq!(entry.path, workspace_job);
        assert_eq!(entry.spec.max_active_runs, 7);

        let (path, spec) = runtime
            .load_v2_job_asset_by_name("task_auto_pipeline")
            .expect("job lookup");
        assert_eq!(path, workspace_job);
        assert_eq!(spec.max_active_runs, 7);
    }

    #[test]
    fn duplicate_jobs_within_one_catalog_directory_remain_invalid() {
        let (_root, runtime, _global_root, workspace_root) = test_runtime();
        let jobs_dir = workspace_root.join("resources/jobs");
        write_job(&jobs_dir.join("first.yaml"), "duplicate_job", "first", 1);
        write_job(
            &jobs_dir.join("nested/second.yaml"),
            "duplicate_job",
            "second",
            1,
        );

        let err = runtime
            .show_job_catalog_entry("duplicate_job")
            .expect_err("duplicate job name should fail");
        assert!(
            err.to_string()
                .contains("duplicate v2 job name 'duplicate_job'"),
            "{err}"
        );
    }
}
