use orbit_common::OrbitError;
use serde_json::{Value, json};

use crate::context::RuntimeHost;

use super::super::super::input::{
    input_string_field, json_number_to_string, required_input_string,
};
use super::super::base_obsolescence::ensure_base_can_still_land;
use super::super::freshness::branch_freshness_against_ref;
use super::super::git::git_output;
use super::super::handoff::{
    FailedHandoffPhase, HandoffContext, load_handoff_context, record_failed_handoff,
};
use super::super::operations;
use super::body::{
    GITHUB_PR_BODY_BYTE_LIMIT, bound_pr_body, build_batch_pr_body, default_pr_title,
};

pub(in crate::executor::automation) fn pr_open<H: RuntimeHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let context = load_handoff_context(host, input, "pr_open")?;
    match open_or_reuse_pr(host, input, &context) {
        Ok(output) => Ok(output),
        Err((phase, error)) => {
            record_failed_handoff(host, &context, input, phase, &error)?;
            Err(error)
        }
    }
}

fn open_or_reuse_pr<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
    context: &HandoffContext,
) -> Result<Value, (FailedHandoffPhase, OrbitError)> {
    let head = required_input_string(input, "head").map_err(invalid_prepare)?;
    let base = required_input_string(input, "base").map_err(invalid_prepare)?;
    let base_ref = required_input_string(input, "base_ref").map_err(invalid_prepare)?;
    let base_sha = required_input_string(input, "base_sha").map_err(invalid_prepare)?;
    let current_branch = git_output(
        &context.workspace_path,
        &["rev-parse", "--abbrev-ref", "HEAD"],
    )
    .map_err(invalid_prepare)?;
    if current_branch.trim() != head {
        return Err((
            FailedHandoffPhase::PrLookup,
            OrbitError::Execution(format!(
                "pr_open: prepared branch '{head}' is not checked out (found '{}')",
                current_branch.trim()
            )),
        ));
    }
    // ORB-10644: divergence against the pinned base says nothing about whether
    // that base is still a branch work can land through. A base that merged and
    // was deleted (or restored to its pre-merge tip) still resolves, so every
    // later step would report success against a PR nobody merges again.
    ensure_base_can_still_land(&context.workspace_path, "pr_open", base, base_sha, input)
        .map_err(|error| (FailedHandoffPhase::ObsoleteBase, error))?;
    let freshness = branch_freshness_against_ref(&context.workspace_path, head, base_ref, base_sha)
        .map_err(invalid_prepare)?;
    if freshness.commits_behind != 0 || freshness.commits_ahead == 0 {
        return Err((
            FailedHandoffPhase::EmptyBranch,
            OrbitError::Execution(format!(
                "pr_open: prepared head '{head}' must be ahead of and not behind base checkpoint '{base_sha}'"
            )),
        ));
    }
    let diff_output = git_output(
        &context.workspace_path,
        &["diff", "--name-only", &format!("{base_sha}...{head}")],
    )
    .map_err(invalid_prepare)?;
    let changed_files = diff_output
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    let title = input_string_field(input, "title")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_pr_title(&context.tasks));
    let pr_config = host.pr_config();
    let pr_opener_model = host.actor_model_identity();
    let body = input_string_field(input, "body")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            build_batch_pr_body(
                &context.tasks,
                &freshness,
                &changed_files,
                &pr_config,
                pr_opener_model.as_deref(),
            )
        });
    let body = bound_pr_body(body, &context.tasks);
    match find_pr_by_head(host, &context.workspace_path, head) {
        Ok(Some((pr_number, pr_url))) => Ok(pr_output(PrOutput {
            decision: "reused",
            pr_created: false,
            pr_reused: true,
            pr_number,
            pr_url,
            base,
            head,
            base_ref,
            base_sha,
            freshness: &freshness,
        })),
        Ok(None) => {
            tracing::info!(
                constructed_body_bytes = body.len(),
                allowed_body_bytes = GITHUB_PR_BODY_BYTE_LIMIT,
                "creating pull request with bounded body projection"
            );
            let created = host
                .run_private_vcs_operation(
                    operations::PR_CREATE,
                    json!({
                        "title": title,
                        "body": body,
                        "base": base,
                        "head": head,
                        "workspace_path": context.workspace_path,
                    }),
                )
                .map_err(|error| (FailedHandoffPhase::PrCreate, error))?;
            let pr_url = created
                .get("url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    (
                        FailedHandoffPhase::PrCreate,
                        OrbitError::Execution(
                            "private automation VCS PR create did not return a PR url".to_string(),
                        ),
                    )
                })?;
            let (pr_number, viewed_url) = view_pr(host, &context.workspace_path, &pr_url)
                .map_err(|error| (FailedHandoffPhase::PrView, error))?;
            Ok(pr_output(PrOutput {
                decision: "performed",
                pr_created: true,
                pr_reused: false,
                pr_number,
                pr_url: viewed_url.or(Some(pr_url)),
                base,
                head,
                base_ref,
                base_sha,
                freshness: &freshness,
            }))
        }
        Err(error) => Err((FailedHandoffPhase::PrLookup, error)),
    }
}

/// Create or reuse a PR for an already-pushed recovery branch without the
/// normal freshness/success gate. The caller owns candidate validation and
/// must block, never promote, the associated task.
pub(in crate::executor::automation::vcs) fn open_or_reuse_unchecked<H: RuntimeHost + ?Sized>(
    host: &H,
    workspace_path: &std::path::Path,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
) -> Result<(String, Option<String>, bool), OrbitError> {
    let body = bound_pr_body(body.to_string(), &[]);
    if let Some((number, url)) = find_pr_by_head(host, workspace_path, head)? {
        return Ok((number, url, false));
    }
    tracing::info!(
        constructed_body_bytes = body.len(),
        allowed_body_bytes = GITHUB_PR_BODY_BYTE_LIMIT,
        "creating unchecked pull request with bounded body projection"
    );
    let created = host.run_private_vcs_operation(
        operations::PR_CREATE,
        json!({
            "title": title,
            "body": body,
            "base": base,
            "head": head,
            "workspace_path": workspace_path,
        }),
    )?;
    let url = created
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OrbitError::Execution(
                "private automation VCS PR create did not return a PR url".to_string(),
            )
        })?;
    let (number, viewed_url) = view_pr(host, workspace_path, url)?;
    Ok((number, viewed_url.or_else(|| Some(url.to_string())), true))
}

fn invalid_prepare(error: OrbitError) -> (FailedHandoffPhase, OrbitError) {
    (FailedHandoffPhase::PrLookup, error)
}

fn view_pr<H: RuntimeHost + ?Sized>(
    host: &H,
    workspace_path: &std::path::Path,
    selector: &str,
) -> Result<(String, Option<String>), OrbitError> {
    let value = host.run_private_vcs_operation(
        operations::PR_VIEW,
        json!({ "pr": selector, "workspace_path": workspace_path }),
    )?;
    let pull_request = value.get("pull_request").ok_or_else(|| {
        OrbitError::Execution(
            "private automation VCS PR view did not return pull_request metadata".to_string(),
        )
    })?;
    let pr_number = pull_request
        .get("number")
        .and_then(json_number_to_string)
        .ok_or_else(|| {
            OrbitError::Execution(
                "private automation VCS PR view did not return a PR number".to_string(),
            )
        })?;
    let pr_url = pull_request
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Ok((pr_number, pr_url))
}

fn find_pr_by_head<H: RuntimeHost + ?Sized>(
    host: &H,
    workspace_path: &std::path::Path,
    head: &str,
) -> Result<Option<(String, Option<String>)>, OrbitError> {
    let value = host.run_private_vcs_operation(
        operations::PR_LIST,
        json!({ "head": head, "state": "open", "workspace_path": workspace_path }),
    )?;
    let pull_requests = value
        .get("pull_requests")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            OrbitError::Execution(
                "private automation VCS PR list did not return pull_requests metadata".to_string(),
            )
        })?;

    let mut matching_pr_number = None;
    for pull_request in pull_requests {
        let listed_head = pull_request
            .get("headRefName")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                OrbitError::Execution(
                    "private automation VCS PR list returned a pull request without headRefName"
                        .to_string(),
                )
            })?;
        if listed_head != head {
            continue;
        }
        let pr_number = pull_request
            .get("number")
            .and_then(json_number_to_string)
            .ok_or_else(|| {
                OrbitError::Execution(
                    "private automation VCS PR list returned a matching pull request without a number"
                        .to_string(),
                )
            })?;
        if matching_pr_number.replace(pr_number).is_some() {
            return Err(OrbitError::Execution(format!(
                "private automation VCS PR list returned multiple open pull requests for head branch '{head}'"
            )));
        }
    }

    matching_pr_number
        .map(|pr_number| view_pr(host, workspace_path, &pr_number))
        .transpose()
}

struct PrOutput<'a> {
    decision: &'a str,
    pr_created: bool,
    pr_reused: bool,
    pr_number: String,
    pr_url: Option<String>,
    base: &'a str,
    head: &'a str,
    base_ref: &'a str,
    base_sha: &'a str,
    freshness: &'a super::super::freshness::BranchFreshness,
}

fn pr_output(output: PrOutput<'_>) -> Value {
    json!({
        "phase": "pr_open",
        "decision": output.decision,
        "pr_created": output.pr_created,
        "pr_reused": output.pr_reused,
        "pr_number": output.pr_number,
        "pr_url": output.pr_url,
        "base": output.base,
        "head": output.head,
        "base_ref": output.base_ref,
        "base_sha": output.base_sha,
        "commits_behind": output.freshness.commits_behind,
        "commits_ahead": output.freshness.commits_ahead,
    })
}
