pub mod auth;
pub mod pr_checkout;
pub mod pr_checks;
pub mod pr_close;
pub mod pr_comment;
pub mod pr_comment_reply;
pub mod pr_comments;
pub mod pr_create;
pub mod pr_list;
pub mod pr_merge;
pub mod pr_review;
pub mod pr_view;
pub mod repo;

use orbit_types::OrbitError;
use serde_json::Value;

use crate::{ToolContext, ToolRegistry, require_str};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(auth::GithubAuthStatusTool);
    registry.register(repo::GithubRepoViewTool);
    registry.register(pr_create::GithubPrCreateTool);
    registry.register(pr_list::GithubPrListTool);
    registry.register(pr_view::GithubPrViewTool);
    registry.register(pr_checkout::GithubPrCheckoutTool);
    registry.register(pr_comment::GithubPrCommentTool);
    registry.register(pr_comment_reply::GithubPrCommentReplyTool);
    registry.register(pr_comments::GithubPrCommentsTool);
    registry.register(pr_review::GithubPrReviewTool);
    registry.register(pr_merge::GithubPrMergeTool);
    registry.register(pr_close::GithubPrCloseTool);
    registry.register(pr_checks::GithubPrChecksTool);
}

/// Extract a non-empty `pr` field from the tool input.
pub(super) fn require_pr(input: &Value) -> Result<String, OrbitError> {
    require_str(input, "pr")
}

/// Build an agent attribution footer line.
///
/// Returns `None` when `agent_name` is not set on the context (i.e. the tool
/// was not called from an agent execution path).
pub(super) fn agent_signature(ctx: &ToolContext, verb: &str) -> Option<String> {
    let agent = ctx.agent_name.as_deref()?;
    let model = ctx.model_name.as_deref().unwrap_or("unknown");
    Some(format!("*{verb} by: {agent} / {model}*"))
}

/// Append an agent attribution footer to a body string, if identity is available.
pub(super) fn append_signature(body: &str, ctx: &ToolContext, verb: &str) -> String {
    match agent_signature(ctx, verb) {
        Some(sig) => format!("{body}\n\n{sig}"),
        None => body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{ToolContext, ToolRegistry};

    fn registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register_builtins();
        r
    }

    // --- Registration ---

    #[test]
    fn github_tools_are_registered() {
        let r = registry();
        let names: Vec<_> = r.schemas().into_iter().map(|s| s.name).collect();
        for expected in &[
            "github.auth.status",
            "github.repo.view",
            "github.pr.create",
            "github.pr.list",
            "github.pr.view",
            "github.pr.checkout",
            "github.pr.comment",
            "github.pr.comment.reply",
            "github.pr.comments",
            "github.pr.review",
            "github.pr.merge",
            "github.pr.close",
            "github.pr.checks",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing tool: {expected}"
            );
        }
    }

    // --- Input validation ---

    #[test]
    fn pr_create_rejects_missing_title() {
        let err = super::pr_create::build_exec_request(
            &ToolContext::default(),
            &json!({
                "head": "feature",
                "base": "main",
                "body": "desc",
            }),
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("title"), "{err}");
    }

    #[test]
    fn pr_create_rejects_missing_base() {
        let err = super::pr_create::build_exec_request(
            &ToolContext::default(),
            &json!({
                "title": "T",
                "head": "feature",
                "body": "desc",
            }),
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("base"), "{err}");
    }

    #[test]
    fn pr_create_rejects_missing_head() {
        let err = super::pr_create::build_exec_request(
            &ToolContext::default(),
            &json!({
                "title": "T",
                "base": "main",
                "body": "desc",
            }),
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("head"), "{err}");
    }

    #[test]
    fn pr_create_rejects_missing_body_and_body_file() {
        let err = super::pr_create::build_exec_request(
            &ToolContext::default(),
            &json!({
                "title": "T",
                "base": "main",
                "head": "feature",
            }),
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("body"), "{err}");
    }

    #[test]
    fn pr_comment_rejects_missing_pr() {
        let err = super::pr_comment::build_exec_request(
            &ToolContext::default(),
            &json!({ "body": "msg" }),
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("pr"), "{err}");
    }

    #[test]
    fn pr_comment_rejects_missing_body() {
        let err =
            super::pr_comment::build_exec_request(&ToolContext::default(), &json!({ "pr": "42" }))
                .expect_err("must fail");
        assert!(err.to_string().contains("body"), "{err}");
    }

    #[test]
    fn pr_comment_reply_rejects_missing_repo() {
        let err = super::pr_comment_reply::build_exec_request(
            &ToolContext::default(),
            &json!({ "pr": "42", "comment_id": "123", "body": "reply" }),
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("repo"), "{err}");
    }

    #[test]
    fn pr_comment_reply_rejects_missing_comment_id() {
        let err = super::pr_comment_reply::build_exec_request(
            &ToolContext::default(),
            &json!({ "repo": "owner/repo", "pr": "42", "body": "reply" }),
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("comment_id"), "{err}");
    }

    #[test]
    fn pr_comment_reply_builds_correct_api_endpoint() {
        let (req, _body) = super::pr_comment_reply::build_exec_request(
            &ToolContext::default(),
            &json!({
                "repo": "owner/repo",
                "pr": "34",
                "comment_id": "12345",
                "body": "looks good",
            }),
        )
        .expect("valid");
        assert_eq!(req.program, "gh");
        assert_eq!(req.args[0], "api");
        assert_eq!(
            req.args[1],
            "repos/owner/repo/pulls/34/comments/12345/replies"
        );
        assert_eq!(req.args[2], "-f");
        assert_eq!(req.args[3], "body=looks good");
    }

    #[test]
    fn pr_comment_reply_appends_agent_signature() {
        let ctx = ToolContext {
            agent_name: Some("claude".to_string()),
            model_name: Some("opus-4.6".to_string()),
            ..Default::default()
        };
        let (req, _body) = super::pr_comment_reply::build_exec_request(
            &ctx,
            &json!({
                "repo": "owner/repo",
                "pr": "34",
                "comment_id": "12345",
                "body": "fixed",
            }),
        )
        .expect("valid");
        let body_arg = &req.args[3];
        assert!(
            body_arg.ends_with("\n\n*Reviewed by: claude / opus-4.6*"),
            "body missing signature: {body_arg}"
        );
    }

    #[test]
    fn pr_review_rejects_missing_action() {
        let err =
            super::pr_review::build_exec_request(&ToolContext::default(), &json!({ "pr": "42" }))
                .expect_err("must fail");
        assert!(err.to_string().contains("action"), "{err}");
    }

    #[test]
    fn pr_review_rejects_invalid_action() {
        let err = super::pr_review::build_exec_request(
            &ToolContext::default(),
            &json!({ "pr": "42", "action": "lgtm" }),
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("action"), "{err}");
    }

    #[test]
    fn pr_review_request_changes_requires_body() {
        let err = super::pr_review::build_exec_request(
            &ToolContext::default(),
            &json!({
                "pr": "42",
                "action": "request-changes",
            }),
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("body"), "{err}");
    }

    #[test]
    fn pr_merge_rejects_invalid_strategy() {
        let err = super::pr_merge::build_exec_request(&json!({
            "pr": "42",
            "strategy": "fast-forward",
        }))
        .expect_err("must fail");
        assert!(err.to_string().contains("strategy"), "{err}");
    }

    // --- Command construction ---

    #[test]
    fn pr_create_builds_correct_args() {
        let req = super::pr_create::build_exec_request(
            &ToolContext::default(),
            &json!({
                "title": "my PR",
                "base": "main",
                "head": "feature/foo",
                "body": "description",
            }),
        )
        .expect("valid input");
        assert_eq!(req.program, "gh");
        assert!(req.args.contains(&"create".to_string()));
        assert!(req.args.contains(&"--title".to_string()));
        assert!(req.args.contains(&"my PR".to_string()));
        assert!(req.args.contains(&"--base".to_string()));
        assert!(req.args.contains(&"main".to_string()));
        assert!(req.args.contains(&"--head".to_string()));
        assert!(req.args.contains(&"feature/foo".to_string()));
        assert!(req.args.contains(&"--label".to_string()));
        assert!(req.args.contains(&"orbit".to_string()));
    }

    #[test]
    fn pr_create_uses_custom_label() {
        let req = super::pr_create::build_exec_request(
            &ToolContext::default(),
            &json!({
                "title": "T",
                "base": "main",
                "head": "branch",
                "body": "b",
                "label": "custom",
            }),
        )
        .expect("valid");
        let label_pos = req.args.iter().position(|a| a == "--label").unwrap();
        assert_eq!(req.args[label_pos + 1], "custom");
    }

    #[test]
    fn pr_create_uses_body_file_when_body_absent() {
        let req = super::pr_create::build_exec_request(
            &ToolContext::default(),
            &json!({
                "title": "T",
                "base": "main",
                "head": "branch",
                "body_file": "/tmp/pr.md",
            }),
        )
        .expect("valid");
        assert!(req.args.contains(&"--body-file".to_string()));
        assert!(req.args.contains(&"/tmp/pr.md".to_string()));
    }

    #[test]
    fn pr_merge_defaults_to_squash_and_delete_branch() {
        let req = super::pr_merge::build_exec_request(&json!({ "pr": "42" })).expect("valid");
        assert!(req.args.contains(&"--squash".to_string()));
        assert!(req.args.contains(&"--delete-branch".to_string()));
    }

    #[test]
    fn pr_merge_omits_delete_branch_when_false() {
        let req = super::pr_merge::build_exec_request(&json!({
            "pr": "42",
            "delete_branch": false,
        }))
        .expect("valid");
        assert!(!req.args.contains(&"--delete-branch".to_string()));
    }

    #[test]
    fn pr_review_approve_builds_correct_args() {
        let req = super::pr_review::build_exec_request(
            &ToolContext::default(),
            &json!({
                "pr": "42",
                "action": "approve",
            }),
        )
        .expect("valid");
        assert!(req.args.contains(&"review".to_string()));
        assert!(req.args.contains(&"42".to_string()));
        assert!(req.args.contains(&"--approve".to_string()));
    }

    #[test]
    fn pr_review_request_changes_includes_body() {
        let req = super::pr_review::build_exec_request(
            &ToolContext::default(),
            &json!({
                "pr": "42",
                "action": "request-changes",
                "body": "fix it",
            }),
        )
        .expect("valid");
        assert!(req.args.contains(&"--request-changes".to_string()));
        assert!(req.args.contains(&"--body".to_string()));
        assert!(req.args.contains(&"fix it".to_string()));
    }

    #[test]
    fn pr_list_uses_orbit_label_when_provided() {
        let req = super::pr_list::build_exec_request(&json!({ "label": "orbit" })).expect("valid");
        assert!(req.args.contains(&"--label".to_string()));
        assert!(req.args.contains(&"orbit".to_string()));
    }

    #[test]
    fn pr_create_appends_agent_signature_when_identity_set() {
        let ctx = ToolContext {
            agent_name: Some("claude".to_string()),
            model_name: Some("opus-4.6".to_string()),
            ..Default::default()
        };
        let req = super::pr_create::build_exec_request(
            &ctx,
            &json!({
                "title": "T",
                "base": "main",
                "head": "branch",
                "body": "description",
            }),
        )
        .expect("valid");
        let body_pos = req.args.iter().position(|a| a == "--body").unwrap();
        let body = &req.args[body_pos + 1];
        assert!(
            body.ends_with("\n\n*Implemented by: claude / opus-4.6*"),
            "body missing signature: {body}"
        );
    }

    #[test]
    fn pr_review_appends_agent_signature_when_identity_set() {
        let ctx = ToolContext {
            agent_name: Some("codex".to_string()),
            model_name: Some("o3".to_string()),
            ..Default::default()
        };
        let req = super::pr_review::build_exec_request(
            &ctx,
            &json!({
                "pr": "42",
                "action": "request-changes",
                "body": "needs work",
            }),
        )
        .expect("valid");
        let body_pos = req.args.iter().position(|a| a == "--body").unwrap();
        let body = &req.args[body_pos + 1];
        assert!(
            body.ends_with("\n\n*Reviewed by: codex / o3*"),
            "body missing signature: {body}"
        );
    }

    #[test]
    fn pr_review_approve_appends_signature_as_body_when_identity_set() {
        let ctx = ToolContext {
            agent_name: Some("claude".to_string()),
            model_name: Some("sonnet-4".to_string()),
            ..Default::default()
        };
        let req = super::pr_review::build_exec_request(
            &ctx,
            &json!({
                "pr": "42",
                "action": "approve",
            }),
        )
        .expect("valid");
        let body_pos = req.args.iter().position(|a| a == "--body").unwrap();
        let body = &req.args[body_pos + 1];
        assert_eq!(body, "*Reviewed by: claude / sonnet-4*");
    }

    #[test]
    fn pr_comment_appends_agent_signature_when_identity_set() {
        let ctx = ToolContext {
            agent_name: Some("codex".to_string()),
            model_name: Some("o3".to_string()),
            ..Default::default()
        };
        let req = super::pr_comment::build_exec_request(
            &ctx,
            &json!({
                "pr": "42",
                "body": "looks good",
            }),
        )
        .expect("valid");
        let body_pos = req.args.iter().position(|a| a == "--body").unwrap();
        let body = &req.args[body_pos + 1];
        assert!(
            body.ends_with("\n\n*Reviewed by: codex / o3*"),
            "body missing signature: {body}"
        );
    }

    #[test]
    fn pr_create_omits_signature_when_no_identity() {
        let req = super::pr_create::build_exec_request(
            &ToolContext::default(),
            &json!({
                "title": "T",
                "base": "main",
                "head": "branch",
                "body": "description",
            }),
        )
        .expect("valid");
        let body_pos = req.args.iter().position(|a| a == "--body").unwrap();
        let body = &req.args[body_pos + 1];
        assert_eq!(body, "description");
    }

    #[test]
    fn pr_checks_builds_json_args() {
        let req = super::pr_checks::build_exec_request(&json!({ "pr": "99" })).expect("valid");
        assert!(req.args.contains(&"checks".to_string()));
        assert!(req.args.contains(&"--json".to_string()));
        assert!(req.args.contains(&"state,name".to_string()));
    }
}
