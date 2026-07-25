use std::path::{Path, PathBuf};

use orbit_common::types::activity_job::{
    CatalogDirectory, CatalogDirectoryList, V2JobCatalog, catalog_error_to_orbit,
};
use orbit_common::types::{JobKind, JobRun, JobScheduleState, JobV2, NotFoundKind, OrbitError};
use orbit_common::utility::fs::write_text_with_parent;
use serde_json::Value;

use crate::OrbitRuntime;

/// Shippable default workflow assets, seeded under
/// `<orbit_root>/resources/jobs/<name>.yaml` on `orbit init`. The entries
/// here are the admission-controlled task shipment workflows
/// (auto / gate / local / pr), the planning-duel workflow, and the
/// failed-run triage workflow [ORB-10129].
/// Example and smoke fixtures live
/// under `crates/orbit-core/assets/jobs/examples/` and are NOT seeded —
/// they exist for `crates/orbit-engine/examples/v2_job_runtime_smoke.rs`
/// only.
const DEFAULT_JOB_FILES: &[(&str, &str)] = &[
    (
        "auto_task_scheduler_pipeline",
        include_str!("../../../assets/jobs/auto_task_scheduler_pipeline.yaml"),
    ),
    (
        "job_duel_plan_pipeline",
        include_str!("../../../assets/jobs/job_duel_plan_pipeline.yaml"),
    ),
    (
        "task_auto_pipeline",
        include_str!("../../../assets/jobs/task_auto_pipeline.yaml"),
    ),
    (
        "task_gate_pipeline",
        include_str!("../../../assets/jobs/task_gate_pipeline.yaml"),
    ),
    (
        "task_local_pipeline",
        include_str!("../../../assets/jobs/task_local_pipeline.yaml"),
    ),
    (
        "task_pr_pipeline",
        include_str!("../../../assets/jobs/task_pr_pipeline.yaml"),
    ),
    (
        "task_review_pipeline",
        include_str!("../../../assets/jobs/task_review_pipeline.yaml"),
    ),
    (
        "task_triage_pipeline",
        include_str!("../../../assets/jobs/task_triage_pipeline.yaml"),
    ),
    (
        "workspace_ship_pipeline",
        include_str!("../../../assets/jobs/workspace_ship_pipeline.yaml"),
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

impl OrbitRuntime {
    pub fn list_job_catalog_with_last_run(
        &self,
        include_disabled: bool,
        filter: JobCatalogFilter,
    ) -> Result<Vec<(JobCatalogEntry, Option<JobRun>)>, OrbitError> {
        use orbit_store::JobRunQuery;

        let v2_jobs = self.load_v2_job_assets()?;
        let mut result = Vec::new();

        for (job_id, path, spec) in v2_jobs.iter() {
            if !include_disabled && spec.state == JobScheduleState::Disabled {
                continue;
            }
            if !matches_job_filter(spec.kind, filter) {
                continue;
            }
            let last_run = self
                .stores()
                .jobs()
                .list_runs_filtered(&JobRunQuery {
                    job_id: Some(job_id.to_string()),
                    state: None,
                    created_since: None,
                    limit: Some(1),
                })
                .ok()
                .and_then(|runs| runs.into_iter().next());
            result.push((
                JobCatalogEntry {
                    job_id: job_id.to_string(),
                    path: path.to_path_buf(),
                    spec: spec.clone(),
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
            .map(|(path, spec)| JobCatalogEntry {
                job_id: job_id.to_string(),
                path: path.to_path_buf(),
                spec: spec.clone(),
            })
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Job, job_id.to_string()))
    }

    fn load_v2_job_assets(&self) -> Result<V2JobCatalog, OrbitError> {
        self.load_v2_job_catalog(self.v2_job_asset_dirs())
    }

    fn load_v2_job_catalog(
        &self,
        dirs: Vec<CatalogDirectory<V2JobCatalogDirKind>>,
    ) -> Result<V2JobCatalog, OrbitError> {
        let mut catalog = V2JobCatalog::new();
        for dir in dirs {
            if dir.path().is_dir() {
                catalog
                    .load_dir_prefer_existing(dir.path())
                    .map_err(catalog_error_to_orbit)?;
            }
        }
        Ok(catalog)
    }

    fn v2_job_asset_dirs(&self) -> Vec<CatalogDirectory<V2JobCatalogDirKind>> {
        self.v2_job_asset_dirs_with_env(v2_job_env_dirs().as_deref())
    }

    fn v2_job_asset_dirs_with_env(
        &self,
        env_dirs: Option<&str>,
    ) -> Vec<CatalogDirectory<V2JobCatalogDirKind>> {
        let mut dirs = CatalogDirectoryList::default();

        push_v2_job_env_dirs(&mut dirs, env_dirs);
        dirs.push(
            self.paths().jobs_dir.clone(),
            V2JobCatalogDirKind::Workspace,
        );
        dirs.push(
            self.paths().global_dir.join("resources/jobs"),
            V2JobCatalogDirKind::Global,
        );
        dirs.into_vec()
    }

    pub(crate) fn load_v2_job_asset_by_name(
        &self,
        job_id: &str,
    ) -> Result<(PathBuf, JobV2), OrbitError> {
        let catalog = self.load_v2_job_catalog(self.v2_job_asset_dirs_for_execution(job_id))?;
        catalog
            .get(job_id)
            .map(|(path, spec)| (path.to_path_buf(), spec.clone()))
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Job, job_id.to_string()))
    }

    fn v2_job_asset_dirs_for_execution(
        &self,
        job_id: &str,
    ) -> Vec<CatalogDirectory<V2JobCatalogDirKind>> {
        self.v2_job_asset_dirs_for_execution_with_env(job_id, v2_job_env_dirs().as_deref())
    }

    fn v2_job_asset_dirs_for_execution_with_env(
        &self,
        job_id: &str,
        env_dirs: Option<&str>,
    ) -> Vec<CatalogDirectory<V2JobCatalogDirKind>> {
        let mut dirs = CatalogDirectoryList::default();

        // L-0060 / ORB-00356: name-based execution keeps shipped defaults
        // authoritative over workspace catalogs.
        push_v2_job_env_dirs(&mut dirs, env_dirs);
        dirs.push(
            self.paths().global_dir.join("resources/jobs"),
            V2JobCatalogDirKind::Global,
        );
        if !is_default_job_name(job_id) {
            dirs.push(
                self.paths().jobs_dir.clone(),
                V2JobCatalogDirKind::Workspace,
            );
        }
        dirs.into_vec()
    }
}

fn v2_job_env_dirs() -> Option<String> {
    std::env::var("ORBIT_JOB_DIR")
        .ok()
        .or_else(|| std::env::var("ORBIT_V2_JOB_DIR").ok())
}

fn push_v2_job_env_dirs(
    dirs: &mut CatalogDirectoryList<V2JobCatalogDirKind>,
    env_dirs: Option<&str>,
) {
    if let Some(raw) = env_dirs {
        for entry in raw.split(':').filter(|value| !value.is_empty()) {
            dirs.push(PathBuf::from(entry), V2JobCatalogDirKind::Explicit);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum V2JobCatalogDirKind {
    Explicit,
    Workspace,
    Global,
}

fn is_default_job_name(job_id: &str) -> bool {
    DEFAULT_JOB_FILES
        .iter()
        .any(|(default_job_id, _)| *default_job_id == job_id)
}

fn matches_job_filter(kind: JobKind, filter: JobCatalogFilter) -> bool {
    match filter {
        JobCatalogFilter::WorkflowsOnly => kind == JobKind::Workflow,
        JobCatalogFilter::All => true,
        JobCatalogFilter::Kind(expected) => kind == expected,
    }
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

    use orbit_common::types::activity_job::{V2ActivityCatalog, resolve_job_target_refs};
    use orbit_common::types::{
        ActivityV2Spec, JobV2Step, JobV2StepBody, load_activity_asset, load_job_asset,
    };
    use serde_json::Value;
    use std::collections::BTreeSet;
    use tempfile::tempdir;

    use crate::command::activity::DEFAULT_ACTIVITY_FILES;

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

    fn default_activity_catalog() -> V2ActivityCatalog {
        let mut catalog = V2ActivityCatalog::new();
        for (name, yaml) in DEFAULT_ACTIVITY_FILES {
            let asset = load_activity_asset(yaml)
                .unwrap_or_else(|err| panic!("default activity {name} should parse: {err}"));
            assert_eq!(&asset.name, name);
            catalog.insert(*name, asset.spec);
        }
        catalog
    }

    fn assert_condition_tokens_are_paths(condition: &str) {
        let mut remaining = condition;
        while let Some(start) = remaining.find("{{") {
            let after_start = &remaining[start + 2..];
            let end = after_start
                .find("}}")
                .unwrap_or_else(|| panic!("unterminated template token in {condition:?}"));
            let token = after_start[..end].trim();
            assert!(
                !["==", "!=", "&&", "||", ">", "<"]
                    .iter()
                    .any(|op| token.contains(op)),
                "template token {token:?} in condition {condition:?} must be a path; put comparisons outside the braces",
            );
            remaining = &after_start[end + 2..];
        }
    }

    fn assert_step_condition_tokens_are_paths(step: &orbit_common::types::JobV2Step) {
        if let Some(when) = &step.when {
            assert_condition_tokens_are_paths(when);
        }
        match &step.body {
            JobV2StepBody::Parallel { parallel } => {
                for branch in &parallel.branches {
                    assert_step_condition_tokens_are_paths(branch);
                }
            }
            JobV2StepBody::FanOut { fan_out, .. } => {
                assert_step_condition_tokens_are_paths(&fan_out.worker);
            }
            JobV2StepBody::Loop { loop_ } => {
                if let Some(break_when) = &loop_.break_when {
                    assert_condition_tokens_are_paths(break_when);
                }
                for child in &loop_.steps {
                    assert_step_condition_tokens_are_paths(child);
                }
            }
            JobV2StepBody::TargetRef(_) | JobV2StepBody::Target(_) => {}
        }
    }

    #[test]
    fn seeded_jobs_include_planning_duel_pipeline() {
        let (_root, runtime, global_root, _workspace_root) = test_runtime();
        seed_default_jobs(&global_root.join("resources/jobs"), true).expect("seed default jobs");

        let entry = runtime
            .show_job_catalog_entry("job_duel_plan_pipeline")
            .expect("planning duel job is seeded");
        assert_eq!(entry.spec.kind, JobKind::Workflow);
        assert_eq!(entry.spec.steps.len(), 1);
        assert_eq!(entry.spec.steps[0].id, "run_planning_duel");
    }

    #[test]
    fn default_job_target_refs_resolve_against_default_activities() {
        let catalog = default_activity_catalog();

        for (job_name, yaml) in DEFAULT_JOB_FILES {
            let mut asset = load_job_asset(yaml)
                .unwrap_or_else(|err| panic!("default job {job_name} should parse: {err}"));
            resolve_job_target_refs(&mut asset.spec, &catalog)
                .unwrap_or_else(|err| panic!("default job {job_name} refs resolve: {err}"));
        }
    }

    #[test]
    fn local_task_pipeline_commits_before_merge() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_local_pipeline").then_some(*yaml))
            .expect("task local pipeline default exists");
        let asset = load_job_asset(yaml).expect("parse task local pipeline");
        let root_step_ids = asset
            .spec
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();

        let commit_index = root_step_ids
            .iter()
            .position(|id| *id == "commit")
            .expect("task local pipeline has commit step");
        let merge_index = root_step_ids
            .iter()
            .position(|id| *id == "merge")
            .expect("task local pipeline has merge step");
        assert!(
            commit_index < merge_index,
            "task local pipeline must commit before merge"
        );
    }

    #[test]
    fn ship_review_controls_propagate_through_auto_and_gate_pipelines() {
        let auto_yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_auto_pipeline").then_some(*yaml))
            .expect("task auto pipeline default exists");
        let auto = load_job_asset(auto_yaml).expect("parse task auto pipeline");
        let auto_defaults = auto
            .spec
            .default_input
            .as_ref()
            .expect("task auto pipeline default input");
        assert_eq!(auto_defaults["review"], false);
        assert_eq!(auto_defaults["review_crew"], Value::Null);

        let auto_dispatch = auto
            .spec
            .steps
            .iter()
            .find(|step| step.id == "dispatch")
            .expect("task auto pipeline dispatch step");
        let JobV2StepBody::FanOut { fan_out, .. } = &auto_dispatch.body else {
            panic!("task auto pipeline dispatch must be a fan-out");
        };
        let JobV2StepBody::TargetRef(auto_target) = &fan_out.worker.body else {
            panic!("task auto pipeline worker must reference invoke_and_wait");
        };
        let auto_run_input = &auto_target
            .default_input
            .as_ref()
            .expect("task auto pipeline dispatch input");
        assert_eq!(auto_run_input["job_name"], "task_gate_pipeline");
        let auto_run_input = &auto_run_input["run_input"];
        assert_eq!(auto_run_input["review"], "{{ input.review }}");
        assert_eq!(auto_run_input["review_crew"], "{{ input.review_crew }}");

        let gate_yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_gate_pipeline").then_some(*yaml))
            .expect("task gate pipeline default exists");
        let gate = load_job_asset(gate_yaml).expect("parse task gate pipeline");
        let gate_defaults = gate
            .spec
            .default_input
            .as_ref()
            .expect("task gate pipeline default input");
        assert_eq!(gate_defaults["review"], false);
        assert_eq!(gate_defaults["review_crew"], Value::Null);

        let gate_dispatch = gate
            .spec
            .steps
            .iter()
            .find(|step| step.id == "dispatch_child")
            .expect("task gate pipeline child dispatch step");
        let JobV2StepBody::TargetRef(gate_target) = &gate_dispatch.body else {
            panic!("task gate pipeline dispatch must reference invoke_and_wait");
        };
        let gate_run_input = &gate_target
            .default_input
            .as_ref()
            .expect("task gate pipeline dispatch input");
        assert_eq!(gate_run_input["job_name"], "task_{{ input.mode }}_pipeline");
        let gate_run_input = &gate_run_input["run_input"];
        assert_eq!(gate_run_input["review"], "{{ input.review }}");
        assert_eq!(gate_run_input["review_crew"], "{{ input.review_crew }}");
    }

    #[test]
    fn pr_review_materializes_one_exact_head_child_with_the_explicit_crew() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_pr_pipeline").then_some(*yaml))
            .expect("task PR pipeline exists");
        let asset = load_job_asset(yaml).expect("task PR pipeline parses");
        let defaults = asset.spec.default_input.as_ref().expect("default input");
        assert_eq!(defaults["review"], false);
        assert_eq!(defaults["review_crew"], Value::Null);
        assert!(
            asset
                .spec
                .steps
                .iter()
                .all(|step| step.id != "review_bundle"),
            "the implementation run must not inline the independent reviewer"
        );

        let review_steps = asset
            .spec
            .steps
            .iter()
            .filter(|step| {
                matches!(
                    &step.body,
                    JobV2StepBody::TargetRef(target)
                        if target.target == "activity:invoke_and_wait"
                            && target
                                .default_input
                                .as_ref()
                                .and_then(|input| input.get("job_name"))
                                .and_then(Value::as_str)
                                == Some("task_review_pipeline")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(review_steps.len(), 1, "exactly one review Run is submitted");
        let review = review_steps[0];
        assert_eq!(review.id, "independent_review");
        assert_eq!(review.retry, None);
        assert_eq!(review.recovery_activity, None);
        assert_eq!(
            review.when.as_deref(),
            Some(
                "{{ input.review }} == true && {{ steps.commit.output.skipped_no_diff_expected }} != true"
            )
        );
        let JobV2StepBody::TargetRef(target) = &review.body else {
            panic!("independent review must use invoke_and_wait");
        };
        assert_eq!(target.target, "activity:invoke_and_wait");
        let input = target
            .default_input
            .as_ref()
            .expect("review dispatch input");
        assert_eq!(input["job_name"], "task_review_pipeline");
        assert_eq!(input["dedupe_run_input_field"], "parent_run_id");
        let run_input = &input["run_input"];
        assert_eq!(run_input["crew"], "{{ input.review_crew }}");
        assert_eq!(
            run_input["parent_run_id"],
            "{{ steps.worktree.output.job_run_id }}"
        );
        assert_eq!(run_input["task_ids"], "{{ input.task_ids }}");
        assert_eq!(
            run_input["workspace_path"],
            "{{ steps.worktree.output.workspace_path }}"
        );
        assert_eq!(
            run_input["candidate_head"],
            "{{ steps.push.output.branch }}"
        );
        assert_eq!(
            run_input["candidate_head_sha"],
            "{{ steps.push.output.local_sha }}"
        );
        assert_eq!(
            run_input["pr_number"],
            "{{ steps.pr_open.output.pr_number }}"
        );

        let step_ids = asset
            .spec
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();
        let review_index = step_ids
            .iter()
            .position(|id| *id == "independent_review")
            .expect("review index");
        for prerequisite in ["push", "pr_open", "promote_tasks"] {
            assert!(
                step_ids
                    .iter()
                    .position(|id| *id == prerequisite)
                    .expect("phase")
                    < review_index,
                "review must run after {prerequisite}"
            );
        }

        for job_name in ["task_pr_pipeline", "task_local_pipeline"] {
            let yaml = DEFAULT_JOB_FILES
                .iter()
                .find_map(|(name, yaml)| (*name == job_name).then_some(*yaml))
                .unwrap_or_else(|| panic!("default job {job_name} exists"));
            let asset = load_job_asset(yaml).expect("leaf job parses");
            let implement = asset
                .spec
                .steps
                .iter()
                .find(|step| step.id == "implement_bundle")
                .expect("implement bundle");
            let JobV2StepBody::Loop { loop_ } = &implement.body else {
                panic!("implement bundle must be a loop");
            };
            let JobV2StepBody::TargetRef(implement_target) = &loop_.steps[0].body else {
                panic!("implement step must reference agent_implement");
            };
            assert!(
                implement_target
                    .default_input
                    .as_ref()
                    .expect("implement input")
                    .get("crew")
                    .is_none(),
                "review crew must not reach {job_name} implementation"
            );
        }
    }

    #[test]
    fn task_shipment_implementers_pin_workspace_and_repo_roots_to_the_worktree() {
        for job_name in ["task_local_pipeline", "task_pr_pipeline"] {
            let yaml = DEFAULT_JOB_FILES
                .iter()
                .find_map(|(name, yaml)| (*name == job_name).then_some(*yaml))
                .unwrap_or_else(|| panic!("default job {job_name} exists"));
            let asset =
                load_job_asset(yaml).unwrap_or_else(|error| panic!("parse {job_name}: {error}"));
            let implement_bundle = asset
                .spec
                .steps
                .iter()
                .find(|step| step.id == "implement_bundle")
                .expect("implement bundle");
            let JobV2StepBody::Loop { loop_ } = &implement_bundle.body else {
                panic!("{job_name} implement bundle must be a loop");
            };
            let JobV2StepBody::TargetRef(implement) = &loop_.steps[0].body else {
                panic!("{job_name} implement step must reference agent_implement");
            };
            let input = implement.default_input.as_ref().expect("implement input");
            for field in ["workspace_path", "repo_root"] {
                assert_eq!(
                    input[field], "{{ steps.worktree.output.workspace_path }}",
                    "{job_name} must pin {field} to the exact assigned worktree"
                );
            }
        }
    }

    #[test]
    fn task_shipment_commit_steps_use_the_worktree_base_checkpoint() {
        for job_name in ["task_local_pipeline", "task_pr_pipeline"] {
            let yaml = DEFAULT_JOB_FILES
                .iter()
                .find_map(|(name, yaml)| (*name == job_name).then_some(*yaml))
                .unwrap_or_else(|| panic!("default job {job_name} exists"));
            let asset =
                load_job_asset(yaml).unwrap_or_else(|error| panic!("parse {job_name}: {error}"));
            let commit = asset
                .spec
                .steps
                .iter()
                .find(|step| step.id == "commit")
                .expect("commit step");
            let JobV2StepBody::TargetRef(commit) = &commit.body else {
                panic!("{job_name} commit step must reference git_commit");
            };
            let input = commit.default_input.as_ref().expect("commit input");
            assert_eq!(
                input["base_ref"], "{{ steps.worktree.output.base_ref }}",
                "{job_name} must pass the exact worktree start-point ref"
            );
        }
    }

    #[test]
    fn independent_review_job_requires_structured_exact_head_verdict() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_review_pipeline").then_some(*yaml))
            .expect("review pipeline exists");
        let asset = load_job_asset(yaml).expect("review pipeline parses");
        assert_eq!(asset.spec.steps.len(), 2);
        let JobV2StepBody::TargetRef(review) = &asset.spec.steps[0].body else {
            panic!("review step must be an activity reference");
        };
        assert_eq!(review.target, "activity:agent_review");
        let review_input = review.default_input.as_ref().expect("review input");
        for field in [
            "task_ids",
            "workspace_path",
            "crew",
            "parent_run_id",
            "candidate_head",
            "candidate_head_sha",
            "pr_number",
        ] {
            assert!(review_input.get(field).is_some(), "missing {field}");
        }
        let JobV2StepBody::TargetRef(guard) = &asset.spec.steps[1].body else {
            panic!("verdict guard must be an activity reference");
        };
        assert_eq!(guard.target, "activity:independent_review_guard");
    }

    #[test]
    fn local_review_fails_before_worktree_or_implementation() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_local_pipeline").then_some(*yaml))
            .expect("local pipeline exists");
        let asset = load_job_asset(yaml).expect("local pipeline parses");
        let first = asset.spec.steps.first().expect("local pipeline has steps");
        assert_eq!(first.id, "reject_unsupported_review");
        assert_eq!(first.when.as_deref(), Some("{{ input.review }} == true"));
        assert!(matches!(
            &first.body,
            JobV2StepBody::TargetRef(target)
                if target.target == "activity:pipeline_success_guard"
        ));
        assert!(
            asset
                .spec
                .steps
                .iter()
                .all(|step| step.id != "review_bundle")
        );
    }

    #[test]
    fn pr_pipeline_models_handoff_phases_as_ordered_activity_checkpoints() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_pr_pipeline").then_some(*yaml))
            .expect("task pr pipeline default exists");
        let asset = load_job_asset(yaml).expect("parse task pr pipeline");
        let phases = asset
            .spec
            .steps
            .iter()
            .filter_map(|step| match &step.body {
                JobV2StepBody::TargetRef(target) => Some((
                    step.id.as_str(),
                    target.target.as_str(),
                    step.recovery_activity.as_deref(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            phases,
            vec![
                ("worktree", "activity:worktree_setup", None),
                (
                    "commit",
                    "activity:git_commit",
                    Some("step_failure_recovery")
                ),
                (
                    "prepare_branch",
                    "activity:pr_prepare",
                    Some("step_failure_recovery")
                ),
                (
                    "sync_base",
                    "activity:git_rebase",
                    Some("step_failure_recovery")
                ),
                ("push", "activity:git_push", Some("step_failure_recovery")),
                ("pr_open", "activity:pr_open", Some("step_failure_recovery")),
                (
                    "promote_tasks",
                    "activity:pr_promote",
                    Some("step_failure_recovery")
                ),
                ("independent_review", "activity:invoke_and_wait", None),
                (
                    "require_independent_review_success",
                    "activity:pipeline_success_guard",
                    None
                ),
                (
                    "promote_no_diff",
                    "activity:pr_promote",
                    Some("step_failure_recovery")
                ),
            ]
        );

        let pr_open = asset
            .spec
            .steps
            .iter()
            .find(|step| step.id == "pr_open")
            .expect("PR open phase");
        let JobV2StepBody::TargetRef(target) = &pr_open.body else {
            panic!("PR open must reference a focused activity");
        };
        let input = target.default_input.as_ref().expect("PR open input");
        for hidden_phase in ["scope", "rewrite_performed", "expected_remote_sha"] {
            assert!(
                input.get(hidden_phase).is_none(),
                "pr_open must not embed earlier {hidden_phase} phase input"
            );
        }
    }

    #[test]
    fn gate_pipeline_releases_reservation_before_child_success_guard() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_gate_pipeline").then_some(*yaml))
            .expect("task gate pipeline default exists");
        let asset = load_job_asset(yaml).expect("parse task gate pipeline");
        let root_step_ids = asset
            .spec
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();

        let dispatch_index = root_step_ids
            .iter()
            .position(|id| *id == "dispatch_child")
            .expect("task gate pipeline has child dispatch step");
        let release_index = root_step_ids
            .iter()
            .position(|id| *id == "release_reservation")
            .expect("task gate pipeline has reservation release step");
        let guard_index = root_step_ids
            .iter()
            .position(|id| *id == "require_child_success")
            .expect("task gate pipeline has child success guard step");
        assert!(
            dispatch_index < release_index,
            "reservation must release only after invoke_and_wait returns"
        );
        assert!(
            release_index < guard_index,
            "reservation must release before the child success guard can fail the run"
        );

        let dispatch = &asset.spec.steps[dispatch_index];
        match &dispatch.body {
            JobV2StepBody::TargetRef(target) => {
                assert_eq!(target.target, "activity:invoke_and_wait");
                let input = target.default_input.as_ref().expect("dispatch input");
                assert_eq!(
                    input["job_name"],
                    Value::String("task_{{ input.mode }}_pipeline".to_string())
                );
                assert_eq!(
                    input["admission_task_ids"],
                    Value::String("{{ input.task_ids }}".to_string())
                );
                assert_eq!(
                    input["admission_workflow"],
                    Value::String("worktree_setup".to_string())
                );
            }
            other => panic!("expected dispatch target ref, got {other:?}"),
        }

        let release = &asset.spec.steps[release_index];
        assert_eq!(
            release.when.as_deref(),
            Some(
                "{{ steps.dispatch_child.output.status }} != timeout && {{ steps.dispatch_child.output.status }} != pending && {{ steps.dispatch_child.output.status }} != running"
            )
        );
        match &release.body {
            JobV2StepBody::TargetRef(target) => {
                assert_eq!(target.target, "activity:release_locks");
                let input = target.default_input.as_ref().expect("release input");
                assert_eq!(
                    input["reservation_id"],
                    Value::String("{{ steps.reserve.output.reservation_id }}".to_string())
                );
            }
            other => panic!("expected release target ref, got {other:?}"),
        }

        let guard = &asset.spec.steps[guard_index];
        assert_eq!(
            guard.when.as_deref(),
            Some("{{ steps.reserve.output.reserved }} == true")
        );
        match &guard.body {
            JobV2StepBody::TargetRef(target) => {
                assert_eq!(target.target, "activity:pipeline_success_guard");
                let input = target.default_input.as_ref().expect("guard input");
                assert_eq!(
                    input["result"],
                    Value::String("{{ steps.dispatch_child.output }}".to_string())
                );
            }
            other => panic!("expected guard target ref, got {other:?}"),
        }
    }

    #[test]
    fn auto_pipeline_checks_gate_results_after_fan_in() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_auto_pipeline").then_some(*yaml))
            .expect("task auto pipeline default exists");
        let asset = load_job_asset(yaml).expect("parse task auto pipeline");
        let root_step_ids = asset
            .spec
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();

        let dispatch_index = root_step_ids
            .iter()
            .position(|id| *id == "dispatch")
            .expect("task auto pipeline has dispatch fan-out");
        let guard_index = root_step_ids
            .iter()
            .position(|id| *id == "require_gate_success")
            .expect("task auto pipeline has gate success guard");
        assert!(
            dispatch_index < guard_index,
            "gate results must be collected before the success guard runs"
        );

        let dispatch = &asset.spec.steps[dispatch_index];
        match &dispatch.body {
            JobV2StepBody::FanOut { fan_out, .. } => {
                assert_eq!(fan_out.max_workers, 5);
            }
            other => panic!("expected dispatch fan-out, got {other:?}"),
        }

        let guard = &asset.spec.steps[guard_index];
        assert_eq!(
            guard.when.as_deref(),
            Some("{{ steps.validate_bundles.output.bundle_count }} != 0")
        );
        match &guard.body {
            JobV2StepBody::TargetRef(target) => {
                assert_eq!(target.target, "activity:pipeline_success_guard");
                let input = target.default_input.as_ref().expect("guard input");
                assert_eq!(
                    input["results"],
                    Value::String("{{ steps.gate_results.output }}".to_string())
                );
            }
            other => panic!("expected guard target ref, got {other:?}"),
        }
    }

    #[test]
    fn gate_pipeline_default_reservation_ttl_covers_child_wait_budget() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_gate_pipeline").then_some(*yaml))
            .expect("task gate pipeline default exists");
        let asset = load_job_asset(yaml).expect("parse task gate pipeline");
        let default_input = asset
            .spec
            .default_input
            .as_ref()
            .expect("task gate pipeline default input");
        let ttl_seconds = default_input["ttl_seconds"]
            .as_u64()
            .expect("numeric ttl_seconds");
        let dispatch_timeout_seconds = default_input["dispatch_timeout_seconds"]
            .as_u64()
            .expect("numeric dispatch_timeout_seconds");

        assert!(
            ttl_seconds >= dispatch_timeout_seconds,
            "reservation TTL must cover the child dispatch wait budget"
        );
    }

    /// [ORB-10129] Structural invariants of the triage pipeline: it is
    /// single-flight (`max_active_runs: 1` — one half of the overlap
    /// guarantee, the routine's `overlap: forbid` is the other), an empty
    /// candidate list skips both downstream steps (clean no-op), and the
    /// lifecycle write is the deterministic `apply_dispositions` step, not
    /// the agent.
    #[test]
    fn triage_pipeline_is_single_flight_and_gates_on_candidates() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "task_triage_pipeline").then_some(*yaml))
            .expect("task triage pipeline default exists");
        let asset = load_job_asset(yaml).expect("parse task triage pipeline");
        assert_eq!(asset.spec.max_active_runs, 1);

        let step_ids = asset
            .spec
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            step_ids,
            ["list_candidates", "triage", "apply_dispositions"]
        );

        for step_id in ["triage", "apply_dispositions"] {
            let step = asset
                .spec
                .steps
                .iter()
                .find(|step| step.id == step_id)
                .expect("triage pipeline step");
            assert_eq!(
                step.when.as_deref(),
                Some("{{ steps.list_candidates.output.candidate_count }} != 0"),
                "step {step_id} must be skipped on an empty candidate list"
            );
        }

        let apply = asset
            .spec
            .steps
            .iter()
            .find(|step| step.id == "apply_dispositions")
            .expect("apply step");
        match &apply.body {
            JobV2StepBody::TargetRef(target) => {
                assert_eq!(target.target, "activity:apply_triage_dispositions");
                let input = target.default_input.as_ref().expect("apply input");
                assert_eq!(
                    input["dispositions"],
                    Value::String("{{ steps.triage.output.dispositions }}".to_string())
                );
                assert_eq!(
                    input["candidates"],
                    Value::String("{{ steps.list_candidates.output.candidates }}".to_string())
                );
            }
            other => panic!("expected apply target ref, got {other:?}"),
        }
    }

    #[test]
    fn workspace_ship_pipeline_resolves_then_waits_for_normal_auto_ship() {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == "workspace_ship_pipeline").then_some(*yaml))
            .expect("workspace ship pipeline default exists");
        let asset = load_job_asset(yaml).expect("parse workspace ship pipeline");
        assert_eq!(asset.spec.max_active_runs, 1);
        assert_eq!(asset.spec.steps.len(), 3);
        assert_eq!(asset.spec.steps[0].id, "resolve_ship_input");
        assert_eq!(asset.spec.steps[1].id, "ship");
        assert_eq!(asset.spec.steps[2].id, "require_ship_success");

        match &asset.spec.steps[0].body {
            JobV2StepBody::TargetRef(target) => {
                assert_eq!(target.target, "activity:resolve_workspace_ship_input");
            }
            other => panic!("expected resolver target ref, got {other:?}"),
        }
        match &asset.spec.steps[1].body {
            JobV2StepBody::TargetRef(target) => {
                assert_eq!(target.target, "activity:invoke_and_wait");
                let input = target.default_input.as_ref().expect("ship input");
                assert_eq!(input["job_name"], "task_auto_pipeline");
                assert_eq!(
                    input["run_input"],
                    Value::String("{{ steps.resolve_ship_input.output }}".to_string())
                );
                assert!(input.get("task_ids").is_none());
            }
            other => panic!("expected invoke-and-wait target ref, got {other:?}"),
        }
        match &asset.spec.steps[2].body {
            JobV2StepBody::TargetRef(target) => {
                assert_eq!(target.target, "activity:pipeline_success_guard");
            }
            other => panic!("expected success guard target ref, got {other:?}"),
        }
        assert!(!yaml.contains("auto_ship"));
        assert!(!yaml.contains("ship-sweep"));
        assert!(!yaml.contains("type: shell"));
    }

    #[test]
    fn default_jobs_template_only_declared_agent_loop_handoffs() {
        let agent_activity_names = DEFAULT_ACTIVITY_FILES
            .iter()
            .filter_map(|(name, yaml)| {
                let asset = load_activity_asset(yaml).ok()?;
                matches!(asset.spec.spec, ActivityV2Spec::AgentLoop(_)).then_some(*name)
            })
            .collect::<BTreeSet<_>>();
        let allowed_handoffs = BTreeSet::from([
            // [ORB-10129] The triage agent's dispositions flow into the
            // deterministic `apply_triage_dispositions` step, which bounds
            // them (candidates-only, environmental-only re-backlog, durable
            // budget) instead of trusting them.
            (
                "task_triage_pipeline",
                "triage",
                "steps.triage.output.dispositions",
            ),
            (
                "task_review_pipeline",
                "independent_review",
                "steps.independent_review.output.verdict",
            ),
            (
                "task_review_pipeline",
                "independent_review",
                "steps.independent_review.output.reviewed_head_sha",
            ),
        ]);

        for (job_name, yaml) in DEFAULT_JOB_FILES {
            let asset = load_job_asset(yaml)
                .unwrap_or_else(|err| panic!("default job {job_name} should parse: {err}"));
            let mut agent_step_ids = BTreeSet::new();
            for step in &asset.spec.steps {
                collect_agent_loop_step_ids(step, &agent_activity_names, &mut agent_step_ids);
            }

            if agent_step_ids.is_empty() {
                continue;
            }

            let mut template_strings = Vec::new();
            for step in &asset.spec.steps {
                collect_template_strings(step, &mut template_strings);
            }

            for agent_step_id in agent_step_ids {
                let forbidden = format!("steps.{agent_step_id}.output");
                for template in &template_strings {
                    let allowed =
                        allowed_handoffs
                            .iter()
                            .any(|(allowed_job, allowed_step, allowed_path)| {
                                *allowed_job == *job_name
                                    && *allowed_step == agent_step_id
                                    && template.contains(allowed_path)
                            });
                    assert!(
                        !template.contains(&forbidden) || allowed,
                        "default job {job_name} templates from agent_loop output: {template}"
                    );
                }
            }
        }
    }

    #[test]
    fn default_job_conditions_keep_comparisons_outside_template_tokens() {
        for (name, yaml) in DEFAULT_JOB_FILES {
            let asset = load_job_asset(yaml).unwrap_or_else(|err| {
                panic!("default job {name} should parse before condition checks: {err}")
            });
            for step in &asset.spec.steps {
                assert_step_condition_tokens_are_paths(step);
            }
        }
    }

    #[test]
    fn task_shipment_jobs_resolve_default_recovery_activity() {
        let catalog = default_activity_catalog();

        for job_name in ["task_local_pipeline", "task_pr_pipeline"] {
            let yaml = DEFAULT_JOB_FILES
                .iter()
                .find_map(|(name, yaml)| (*name == job_name).then_some(*yaml))
                .unwrap_or_else(|| panic!("default job {job_name} exists"));
            let mut asset = load_job_asset(yaml)
                .unwrap_or_else(|err| panic!("default job {job_name} should parse: {err}"));

            assert_eq!(asset.spec.recovery_activity.as_deref(), None);
            resolve_job_target_refs(&mut asset.spec, &catalog)
                .unwrap_or_else(|err| panic!("default job {job_name} refs resolve: {err}"));
            if job_name == "task_pr_pipeline" {
                assert_eq!(
                    asset.spec.failure_activity.as_deref(),
                    Some("pr_failure_handoff")
                );
                assert!(
                    asset.spec.resolved_failure_activity.is_some(),
                    "task PR terminal failure handoff must resolve from the shipped catalog"
                );
            } else {
                assert_eq!(asset.spec.failure_activity, None);
            }
            let recovery_steps = step_recovery_activities(&asset.spec);
            assert!(
                !recovery_steps.is_empty(),
                "default job {job_name} should wire recovery on direct shipment steps"
            );
            for (step_id, recovery_activity, resolved) in recovery_steps {
                assert_eq!(
                    recovery_activity.as_deref(),
                    Some("step_failure_recovery"),
                    "step {step_id} should use default recovery activity"
                );
                assert!(
                    resolved,
                    "step {step_id} should cache its recovery activity"
                );
            }
        }
    }

    #[test]
    fn orchestration_jobs_do_not_enable_generic_recovery() {
        for job_name in [
            "job_duel_plan_pipeline",
            "task_auto_pipeline",
            "task_gate_pipeline",
            "task_triage_pipeline",
            "workspace_ship_pipeline",
        ] {
            let yaml = DEFAULT_JOB_FILES
                .iter()
                .find_map(|(name, yaml)| (*name == job_name).then_some(*yaml))
                .unwrap_or_else(|| panic!("default job {job_name} exists"));
            let asset = load_job_asset(yaml)
                .unwrap_or_else(|err| panic!("default job {job_name} should parse: {err}"));

            assert_eq!(
                asset.spec.recovery_activity, None,
                "default job {job_name} should not generically recover child orchestration"
            );
        }
    }

    fn collect_agent_loop_step_ids<'a>(
        step: &'a JobV2Step,
        agent_activity_names: &BTreeSet<&str>,
        out: &mut BTreeSet<&'a str>,
    ) {
        match &step.body {
            JobV2StepBody::TargetRef(target) => {
                if let Some(activity_name) = target.target.strip_prefix("activity:")
                    && agent_activity_names.contains(activity_name)
                {
                    out.insert(step.id.as_str());
                }
            }
            JobV2StepBody::Target(target) => {
                if matches!(target.spec, ActivityV2Spec::AgentLoop(_)) {
                    out.insert(step.id.as_str());
                }
            }
            JobV2StepBody::Parallel { parallel } => {
                for child in &parallel.branches {
                    collect_agent_loop_step_ids(child, agent_activity_names, out);
                }
            }
            JobV2StepBody::FanOut { fan_out, .. } => {
                collect_agent_loop_step_ids(&fan_out.worker, agent_activity_names, out);
            }
            JobV2StepBody::Loop { loop_ } => {
                for child in &loop_.steps {
                    collect_agent_loop_step_ids(child, agent_activity_names, out);
                }
            }
        }
    }

    fn step_recovery_activities(job: &JobV2) -> Vec<(&str, &Option<String>, bool)> {
        let mut out = Vec::new();
        for step in &job.steps {
            collect_step_recovery_activities(step, &mut out);
        }
        out
    }

    fn collect_step_recovery_activities<'a>(
        step: &'a JobV2Step,
        out: &mut Vec<(&'a str, &'a Option<String>, bool)>,
    ) {
        if step.recovery_activity.is_some() {
            out.push((
                step.id.as_str(),
                &step.recovery_activity,
                step.resolved_recovery_activity.is_some(),
            ));
        }
        match &step.body {
            JobV2StepBody::Parallel { parallel } => {
                for child in &parallel.branches {
                    collect_step_recovery_activities(child, out);
                }
            }
            JobV2StepBody::FanOut { fan_out, .. } => {
                collect_step_recovery_activities(&fan_out.worker, out);
            }
            JobV2StepBody::Loop { loop_ } => {
                for child in &loop_.steps {
                    collect_step_recovery_activities(child, out);
                }
            }
            JobV2StepBody::TargetRef(_) | JobV2StepBody::Target(_) => {}
        }
    }

    fn collect_template_strings<'a>(step: &'a JobV2Step, out: &mut Vec<&'a str>) {
        if let Some(when) = &step.when {
            out.push(when);
        }

        match &step.body {
            JobV2StepBody::TargetRef(target) => {
                collect_value_strings(target.default_input.as_ref(), out);
            }
            JobV2StepBody::Target(target) => {
                collect_value_strings(target.default_input.as_ref(), out);
            }
            JobV2StepBody::Parallel { parallel } => {
                for child in &parallel.branches {
                    collect_template_strings(child, out);
                }
            }
            JobV2StepBody::FanOut { fan_out, .. } => {
                out.push(&fan_out.items);
                collect_template_strings(&fan_out.worker, out);
            }
            JobV2StepBody::Loop { loop_ } => {
                if let Some(items) = &loop_.items {
                    out.push(items);
                }
                if let Some(break_when) = &loop_.break_when {
                    out.push(break_when);
                }
                for child in &loop_.steps {
                    collect_template_strings(child, out);
                }
            }
        }
    }

    fn collect_value_strings<'a>(value: Option<&'a Value>, out: &mut Vec<&'a str>) {
        match value {
            Some(Value::String(text)) => out.push(text),
            Some(Value::Array(items)) => {
                for item in items {
                    collect_value_strings(Some(item), out);
                }
            }
            Some(Value::Object(map)) => {
                for item in map.values() {
                    collect_value_strings(Some(item), out);
                }
            }
            _ => {}
        }
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
    fn job_listing_directory_order_is_env_then_workspace_then_global() {
        let (root, runtime, global_root, workspace_root) = test_runtime();
        let env_dir = root.path().join("env-jobs");
        let workspace_dir = workspace_root.join("resources/jobs");
        let global_dir = global_root.join("resources/jobs");
        write_job(&env_dir.join("layered.yaml"), "layered", "env", 9);
        write_job(
            &workspace_dir.join("layered.yaml"),
            "layered",
            "workspace",
            7,
        );
        write_job(&global_dir.join("layered.yaml"), "layered", "global", 1);

        let dirs = runtime.v2_job_asset_dirs_with_env(Some(env_dir.to_str().expect("utf-8 path")));
        let actual = dirs
            .iter()
            .map(|dir| (dir.path().to_path_buf(), *dir.kind()))
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            [
                (env_dir.clone(), V2JobCatalogDirKind::Explicit),
                (workspace_dir, V2JobCatalogDirKind::Workspace),
                (global_dir, V2JobCatalogDirKind::Global),
            ]
        );

        let catalog = runtime.load_v2_job_catalog(dirs).expect("load catalog");
        let (path, job) = catalog.get("layered").expect("layered job");
        assert_eq!(path, env_dir.join("layered.yaml"));
        assert_eq!(job.max_active_runs, 9);
    }

    #[test]
    fn job_execution_directory_order_is_env_then_global_then_non_default_workspace() {
        let (root, runtime, global_root, workspace_root) = test_runtime();
        let env_dir = root.path().join("env-jobs");
        let workspace_dir = workspace_root.join("resources/jobs");
        let global_dir = global_root.join("resources/jobs");
        write_job(&env_dir.join("custom.yaml"), "custom", "env", 9);
        write_job(
            &env_dir.join("task_auto_pipeline.yaml"),
            "task_auto_pipeline",
            "env",
            9,
        );
        write_job(&workspace_dir.join("custom.yaml"), "custom", "workspace", 7);
        write_job(&global_dir.join("custom.yaml"), "custom", "global", 1);
        write_job(
            &workspace_dir.join("task_auto_pipeline.yaml"),
            "task_auto_pipeline",
            "workspace",
            7,
        );
        write_job(
            &global_dir.join("task_auto_pipeline.yaml"),
            "task_auto_pipeline",
            "global",
            1,
        );
        let env = env_dir.to_str().expect("utf-8 path");

        let custom_dirs = runtime.v2_job_asset_dirs_for_execution_with_env("custom", Some(env));
        let actual = custom_dirs
            .iter()
            .map(|dir| (dir.path().to_path_buf(), *dir.kind()))
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            [
                (env_dir.clone(), V2JobCatalogDirKind::Explicit),
                (global_dir.clone(), V2JobCatalogDirKind::Global),
                (workspace_dir.clone(), V2JobCatalogDirKind::Workspace),
            ]
        );
        let custom_catalog = runtime
            .load_v2_job_catalog(custom_dirs)
            .expect("load custom catalog");
        assert_eq!(
            custom_catalog.get("custom").map(|(path, _)| path),
            Some(env_dir.join("custom.yaml").as_path())
        );

        let default_dirs =
            runtime.v2_job_asset_dirs_for_execution_with_env("task_auto_pipeline", Some(env));
        let actual = default_dirs
            .iter()
            .map(|dir| (dir.path().to_path_buf(), *dir.kind()))
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            [
                (env_dir.clone(), V2JobCatalogDirKind::Explicit),
                (global_dir.clone(), V2JobCatalogDirKind::Global),
            ]
        );
        let default_catalog = runtime
            .load_v2_job_catalog(default_dirs)
            .expect("load default catalog");
        assert_eq!(
            default_catalog
                .get("task_auto_pipeline")
                .map(|(path, _)| path),
            Some(env_dir.join("task_auto_pipeline.yaml").as_path())
        );
    }

    #[test]
    fn workspace_job_overrides_global_default_in_catalog_lookup_but_not_execution_lookup() {
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
        assert_eq!(path, global_job);
        assert_eq!(spec.max_active_runs, 1);
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

    #[test]
    fn malformed_job_assets_remain_hard_catalog_errors() {
        let (_root, runtime, _global_root, workspace_root) = test_runtime();
        let malformed = workspace_root.join("resources/jobs/malformed.yaml");
        std::fs::create_dir_all(malformed.parent().expect("job path has parent"))
            .expect("create jobs dir");
        std::fs::write(&malformed, "schemaVersion: 2\nkind: Job\nspec: [")
            .expect("write malformed job");

        let err = runtime
            .list_job_catalog_with_last_run(true, JobCatalogFilter::All)
            .expect_err("malformed job should fail catalog loading");
        assert!(err.to_string().contains("malformed.yaml"), "{err}");
        assert!(err.to_string().contains("parse"), "{err}");
    }
}
