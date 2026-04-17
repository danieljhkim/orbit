use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use orbit_types::{OrbitError, Task};
use serde::{Deserialize, Serialize};

use crate::OrbitRuntime;

use super::paths::{context_workspace_root, extract_task_path_mentions, task_path_exists};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskLintReport {
    pub task_id: orbit_types::OrbitId,
    pub duration_ms: u64,
    pub finding_count: usize,
    pub findings: Vec<TaskLintFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskLintFinding {
    pub severity: TaskLintSeverity,
    pub check: String,
    pub message: String,
    pub fix_it: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskLintSeverity {
    Error,
    Warning,
}

impl OrbitRuntime {
    pub fn lint_task(&self, id: &str) -> Result<TaskLintReport, OrbitError> {
        let started_at = Instant::now();
        let task = self.get_task(id)?;
        let workspace_root =
            context_workspace_root(&self.paths().repo_root, task.workspace_path.as_deref());
        let description_paths = extract_task_path_mentions(&task.description);
        let mut findings = Vec::new();

        lint_context_file_paths(&task, &workspace_root, &mut findings);
        lint_description_paths(&description_paths, &workspace_root, &mut findings);
        lint_context_completeness(&task, &description_paths, &workspace_root, &mut findings);
        lint_acceptance_criteria(&task.acceptance_criteria, &mut findings);
        lint_identity_cleanup(&task, &mut findings);

        Ok(TaskLintReport {
            task_id: task.id,
            duration_ms: started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            finding_count: findings.len(),
            findings,
        })
    }
}

fn lint_context_file_paths(
    task: &Task,
    workspace_root: &Path,
    findings: &mut Vec<TaskLintFinding>,
) {
    for path in &task.context_files {
        if task_path_exists(workspace_root, path) {
            continue;
        }
        findings.push(TaskLintFinding {
            severity: TaskLintSeverity::Error,
            check: "path_validity".to_string(),
            message: format!("context file `{path}` does not exist in the task worktree"),
            fix_it: format!(
                "Remove `{path}` from `context_files` or replace it with an existing path under `{}`.",
                workspace_root.display()
            ),
        });
    }
}

fn lint_description_paths(
    mentioned_paths: &[String],
    workspace_root: &Path,
    findings: &mut Vec<TaskLintFinding>,
) {
    for path in mentioned_paths {
        if task_path_exists(workspace_root, path) {
            continue;
        }
        findings.push(TaskLintFinding {
            severity: TaskLintSeverity::Error,
            check: "path_validity".to_string(),
            message: format!("description references `{path}`, but that path does not exist"),
            fix_it: format!(
                "Update the task description to reference an existing file, or add `{path}` to the worktree."
            ),
        });
    }
}

fn lint_context_completeness(
    task: &Task,
    mentioned_paths: &[String],
    workspace_root: &Path,
    findings: &mut Vec<TaskLintFinding>,
) {
    let known_context: BTreeSet<&str> = task.context_files.iter().map(String::as_str).collect();
    for path in mentioned_paths {
        if !task_path_exists(workspace_root, path) || known_context.contains(path.as_str()) {
            continue;
        }
        findings.push(TaskLintFinding {
            severity: TaskLintSeverity::Warning,
            check: "context_completeness".to_string(),
            message: format!(
                "description references `{path}`, but it is missing from `context_files`"
            ),
            fix_it: format!("Add `{path}` to `context_files` so implementers get the right scope."),
        });
    }
}

fn lint_acceptance_criteria(acceptance_criteria: &[String], findings: &mut Vec<TaskLintFinding>) {
    const GENERIC_PHRASES: &[&str] = &[
        "implement the feature",
        "implement feature",
        "fix the bug",
        "fix bug",
        "make it work",
        "ensure it works",
        "support the change",
        "handle edge cases",
        "works correctly",
        "update as needed",
    ];
    const NON_DETERMINISTIC_TERMS: &[&str] = &[
        "appropriately",
        "reasonable",
        "clean",
        "intuitive",
        "user-friendly",
        "robust",
        "better",
        "improved",
        "as needed",
        "if needed",
    ];

    for criterion in acceptance_criteria {
        let trimmed = criterion.trim();
        if trimmed.is_empty() {
            findings.push(TaskLintFinding {
                severity: TaskLintSeverity::Warning,
                check: "ac_specificity".to_string(),
                message: "acceptance criterion is blank".to_string(),
                fix_it: "Replace blank acceptance criteria with observable outcomes.".to_string(),
            });
            continue;
        }

        let normalized = trimmed.to_lowercase();
        let has_observable_detail = trimmed.contains('`')
            || trimmed.contains('/')
            || trimmed.chars().any(|ch| ch.is_ascii_digit())
            || [
                "json", "warning", "error", "path", "status", "output", "under ",
            ]
            .iter()
            .any(|needle| normalized.contains(needle));
        let is_generic = GENERIC_PHRASES.iter().any(|phrase| normalized == *phrase);
        let is_too_short = trimmed.len() < 20;
        let is_non_deterministic = NON_DETERMINISTIC_TERMS
            .iter()
            .any(|term| normalized.contains(term));

        if is_too_short || is_generic || (is_non_deterministic && !has_observable_detail) {
            findings.push(TaskLintFinding {
                severity: TaskLintSeverity::Warning,
                check: "ac_specificity".to_string(),
                message: format!(
                    "acceptance criterion is too broad or non-deterministic: `{trimmed}`"
                ),
                fix_it: "Rewrite the criterion as an observable outcome: name the command, file, output, error, or measurable threshold.".to_string(),
            });
        }
    }
}

fn lint_identity_cleanup(task: &Task, findings: &mut Vec<TaskLintFinding>) {
    const STALE_IDENTITIES: &[(&str, &str)] = &[("orbit-map", "crates/orbit-knowledge")];

    for (needle, replacement) in STALE_IDENTITIES {
        let mut reported_locations = BTreeSet::new();
        if task.description.contains(needle) {
            reported_locations.insert("description".to_string());
        }
        if task.plan.contains(needle) {
            reported_locations.insert("plan".to_string());
        }
        for (index, criterion) in task.acceptance_criteria.iter().enumerate() {
            if criterion.contains(needle) {
                reported_locations.insert(format!("acceptance_criteria[{index}]"));
            }
        }

        for location in reported_locations {
            findings.push(TaskLintFinding {
                severity: TaskLintSeverity::Warning,
                check: "identity_cleanup".to_string(),
                message: format!(
                    "`{needle}` appears in {location}, but that repository identity is stale in this worktree"
                ),
                fix_it: format!(
                    "Replace `{needle}` with the current crate or path name, such as `{replacement}`."
                ),
            });
        }
    }
}
