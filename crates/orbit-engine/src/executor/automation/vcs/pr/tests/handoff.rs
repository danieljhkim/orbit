use std::fs;
use std::process::Command;

use serde_json::{Value, json};

use super::super::open::pr_open;
use super::super::promote::pr_promote;
use super::test_support::*;
use crate::context::TaskReadHost;
use crate::executor::automation::vcs::freshness::{prepare_pr_handoff, rebase_pr_branch};
use crate::executor::automation::vcs::push::push_batch_changes;
use orbit_common::types::TaskStatus;

#[test]
fn push_classifies_missing_current_fast_forward_remote_ahead_and_divergent_refs() {
    let missing = pr_workspace();
    git(
        &missing.repo,
        &["push", "origin", "--delete", "orbit/test-batch"],
    );
    let missing_host = PrOpenTestHost::new(Vec::new(), missing.repo.clone());
    assert_eq!(
        push_batch_changes(&missing_host, &generic_push_input(&missing.repo))
            .expect("create missing ref")["decision"],
        json!("performed_create")
    );

    let current = pr_workspace();
    let current_host = PrOpenTestHost::new(Vec::new(), current.repo.clone());
    assert_eq!(
        push_batch_changes(&current_host, &generic_push_input(&current.repo))
            .expect("reuse current ref")["decision"],
        json!("reused_current")
    );
    assert!(current_host.tool_calls().is_empty());

    let fast_forward = pr_workspace();
    fs::write(fast_forward.repo.join("fast-forward.txt"), "local\n").expect("write local");
    git(&fast_forward.repo, &["add", "fast-forward.txt"]);
    git(&fast_forward.repo, &["commit", "-m", "local follow-up"]);
    let fast_forward_host = PrOpenTestHost::new(Vec::new(), fast_forward.repo.clone());
    assert_eq!(
        push_batch_changes(&fast_forward_host, &generic_push_input(&fast_forward.repo))
            .expect("fast-forward remote")["decision"],
        json!("performed_fast_forward")
    );
    assert_eq!(
        fast_forward_host.tool_calls()[0].input["force_with_lease"],
        json!(false)
    );

    let remote_ahead = pr_workspace();
    let local_sha = git(&remote_ahead.repo, &["rev-parse", "HEAD"]);
    fs::write(remote_ahead.repo.join("remote-only.txt"), "remote\n").expect("write remote");
    git(&remote_ahead.repo, &["add", "remote-only.txt"]);
    git(&remote_ahead.repo, &["commit", "-m", "remote only"]);
    git(&remote_ahead.repo, &["push", "origin", "orbit/test-batch"]);
    git(&remote_ahead.repo, &["reset", "--hard", &local_sha]);
    let remote_ahead_host = PrOpenTestHost::new(Vec::new(), remote_ahead.repo.clone());
    let error = push_batch_changes(&remote_ahead_host, &generic_push_input(&remote_ahead.repo))
        .expect_err("remote-only commit must not be overwritten");
    assert!(error.to_string().contains("remote-only history"));
    assert!(remote_ahead_host.tool_calls().is_empty());

    let divergent = pr_workspace();
    let pre_divergence = git(&divergent.repo, &["rev-parse", "HEAD"]);
    fs::write(divergent.repo.join("remote-side.txt"), "remote\n").expect("write remote side");
    git(&divergent.repo, &["add", "remote-side.txt"]);
    git(&divergent.repo, &["commit", "-m", "remote side"]);
    git(&divergent.repo, &["push", "origin", "orbit/test-batch"]);
    let observed_remote = git(&divergent.repo, &["rev-parse", "HEAD"]);
    git(&divergent.repo, &["reset", "--hard", &pre_divergence]);
    fs::write(divergent.repo.join("local-side.txt"), "local\n").expect("write local side");
    git(&divergent.repo, &["add", "local-side.txt"]);
    git(&divergent.repo, &["commit", "-m", "local side"]);
    let divergent_host = PrOpenTestHost::new(Vec::new(), divergent.repo.clone());
    let mut input = generic_push_input(&divergent.repo);
    let error = push_batch_changes(&divergent_host, &input)
        .expect_err("generic divergence cannot authorize force push");
    assert!(error.to_string().contains("no durable rewrite checkpoint"));
    input["rewrite_performed"] = json!(true);
    input["rewrite_head_before"] = json!(pre_divergence);
    input["expected_remote_sha"] = input["rewrite_head_before"].clone();
    let error = push_batch_changes(&divergent_host, &input)
        .expect_err("stale expected remote SHA must not authorize force push");
    assert!(error.to_string().contains("no durable rewrite checkpoint"));
    assert!(divergent_host.tool_calls().is_empty());
    input["expected_remote_sha"] = json!(observed_remote);
    let result = push_batch_changes(&divergent_host, &input).expect("exact checkpoint authorizes");
    assert_eq!(result["decision"], json!("performed_force_with_lease"));
    let push = divergent_host
        .tool_calls()
        .into_iter()
        .find(|call| call.name == "git.push")
        .expect("force push call");
    assert_eq!(push.input["force_with_lease"], json!(true));
    assert_eq!(
        push.input["expected_remote_sha"],
        input["expected_remote_sha"]
    );
}

#[test]
fn recovered_rebase_continues_remaining_handoff_phases_without_replay() {
    let workspace = rebase_conflict_pr_workspace();
    let task_id = "T20260716-RECOVERY";
    let host = PrOpenTestHost::new(
        vec![batch_task(
            task_id,
            "Recover rebase",
            "Outcome: success\nChanges:\n- Implemented once.",
        )],
        workspace.repo.clone(),
    )
    .with_activity_implementer("codex", "codex");
    let input = json!({
        "workspace_path": workspace.repo,
        "job_run_id": "batch-1",
        "completed_task_ids": [task_id],
        "base": "agent-main",
        "base_sync": "local",
    });
    let commits_before = git(
        &workspace.repo,
        &["rev-list", "--count", "agent-main..orbit/test-batch"],
    );

    let prepared = prepare_pr_handoff(&host, &input).expect("persist preparation checkpoint");
    assert_eq!(prepared["decision"], json!("rebase_required"));
    let rebase_input = rebase_input(&input, &prepared);
    rebase_pr_branch(&host, &rebase_input).expect_err("first rebase conflicts");

    fs::write(
        workspace.repo.join("src/lib.rs"),
        "pub fn diverged() {}\npub fn branch() {}\n",
    )
    .expect("resolve conflict");
    git(&workspace.repo, &["add", "src/lib.rs"]);
    let output = Command::new("git")
        .args(["-c", "core.editor=true", "rebase", "--continue"])
        .current_dir(&workspace.repo)
        .output()
        .expect("continue rebase");
    assert!(
        output.status.success(),
        "rebase continue failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let synced = rebase_pr_branch(&host, &rebase_input).expect("reuse recovered rewrite");
    assert_eq!(synced["decision"], json!("reused_recovery"));
    assert_eq!(synced["rewritten"], json!(true));
    assert_eq!(
        git(
            &workspace.repo,
            &[
                "rev-list",
                "--count",
                &format!(
                    "{}..orbit/test-batch",
                    synced["base_sha"].as_str().expect("base sha")
                )
            ]
        ),
        commits_before,
        "recovery must rewrite the existing task commit, not create a second one"
    );

    let push = push_batch_changes(&host, &push_input(&input, &synced)).expect("safe force push");
    assert_eq!(push["decision"], json!("performed_force_with_lease"));

    host.queue_tool_error("github.pr.view", "local persistence/view failed");
    let open_input = open_input(&input, &synced);
    pr_open(&host, &open_input).expect_err("first create loses local view result");
    let opened = pr_open(&host, &open_input).expect("retry reuses external PR");
    assert_eq!(opened["decision"], json!("reused"));
    assert_eq!(
        host.tool_calls()
            .iter()
            .filter(|call| call.name == "github.pr.create")
            .count(),
        1
    );

    let promoted = pr_promote(
        &host,
        &json!({
            "workspace_path": workspace.repo,
            "job_run_id": "batch-1",
            "completed_task_ids": [task_id],
            "pr_number": opened["pr_number"],
            "pr_url": opened["pr_url"],
        }),
    )
    .expect("promote after reused PR");
    assert_eq!(promoted["decision"], json!("performed"));
    let task = host.get_task(task_id).expect("task");
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.github_pr_number(), Some("42"));
}

fn generic_push_input(repo: &std::path::Path) -> Value {
    json!({
        "workspace_path": repo,
        "branch": "orbit/test-batch",
    })
}

fn rebase_input(common: &Value, prepared: &Value) -> Value {
    json!({
        "workspace_path": common["workspace_path"],
        "job_run_id": common["job_run_id"],
        "completed_task_ids": common["completed_task_ids"],
        "head": prepared["head"],
        "head_sha": prepared["head_sha"],
        "base": prepared["base"],
        "base_ref": prepared["base_ref"],
        "base_sha": prepared["base_sha"],
        "remote_sha": prepared["remote_sha"],
        "commits_behind": prepared["commits_behind"],
        "sync_required": prepared["sync_required"],
    })
}

fn push_input(common: &Value, synced: &Value) -> Value {
    json!({
        "workspace_path": common["workspace_path"],
        "job_run_id": common["job_run_id"],
        "completed_task_ids": common["completed_task_ids"],
        "branch": synced["head"],
        "rewrite_performed": synced["rewritten"],
        "rewrite_head_before": synced["head_sha_before"],
        "expected_remote_sha": synced["remote_sha_before"],
    })
}

fn open_input(common: &Value, synced: &Value) -> Value {
    json!({
        "workspace_path": common["workspace_path"],
        "job_run_id": common["job_run_id"],
        "completed_task_ids": common["completed_task_ids"],
        "head": synced["head"],
        "base": synced["base"],
        "base_ref": synced["base_ref"],
        "base_sha": synced["base_sha"],
    })
}
