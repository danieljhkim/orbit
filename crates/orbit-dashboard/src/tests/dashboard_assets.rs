use axum::body::to_bytes;
use axum::http::{HeaderValue, header};
use axum::response::Response;

use crate::{
    DASHBOARD_CSP, serve_app_js, serve_audit_js, serve_common_js, serve_diagnostics_js,
    serve_index, serve_log_tail_js, serve_markdown_js, serve_marked_js, serve_purify_js,
    serve_review_threads_js, serve_router_js, serve_run_detail_js, serve_runs_js,
    serve_scoreboard_js, serve_tasks_js,
};

#[tokio::test]
async fn dashboard_html_and_js_routes_emit_csp() {
    let routes = [
        ("index", serve_index().await),
        ("marked", serve_marked_js().await),
        ("purify", serve_purify_js().await),
        ("app", serve_app_js().await),
        ("common", serve_common_js().await),
        ("markdown", serve_markdown_js().await),
        ("tasks", serve_tasks_js().await),
        ("audit", serve_audit_js().await),
        ("scoreboard", serve_scoreboard_js().await),
        ("log_tail", serve_log_tail_js().await),
        ("diagnostics", serve_diagnostics_js().await),
        ("router", serve_router_js().await),
        ("runs", serve_runs_js().await),
        ("run_detail", serve_run_detail_js().await),
        ("review_threads", serve_review_threads_js().await),
    ];

    for (name, response) in routes {
        assert_eq!(
            response.headers().get(header::CONTENT_SECURITY_POLICY),
            Some(&HeaderValue::from_static(DASHBOARD_CSP)),
            "{name} route must emit the dashboard CSP"
        );
    }
}

#[tokio::test]
async fn dashboard_index_self_hosts_markdown_runtime() {
    let body = response_body(serve_index().await).await;

    assert!(body.contains(r#"<script src="/static/marked.umd.js"></script>"#));
    assert!(body.contains(r#"<script src="/static/purify.min.js"></script>"#));
    assert!(!body.contains("cdn.jsdelivr.net"));
}

#[test]
fn dashboard_markdown_call_sites_use_sanitizing_wrapper() {
    let wrapper = include_str!("../../assets/dashboard/markdown.js");
    let app = include_str!("../../assets/dashboard/app.js");
    let tasks = include_str!("../../assets/dashboard/tasks.js");

    assert!(wrapper.contains("DOMPurify"));
    assert!(wrapper.contains(".sanitize("));
    assert!(wrapper.contains("marked[methodName]"));
    assert!(!app.contains("marked.parse"));
    assert!(!tasks.contains("marked.parse"));
    assert!(app.contains("renderMarkdown("));
    assert!(tasks.contains("renderMarkdown("));
    assert!(tasks.contains("renderMarkdownInline("));
}

#[test]
fn dashboard_surfaces_workspace_location() {
    // ORB-00037: the selector renders the selected workspace's path as a
    // secondary line, and each aggregate task shows its workspace location in
    // the Details box. Asserted against the embedded asset sources since the
    // dashboard has no JS test runner (see dashboard_markdown_call_sites above).
    let app = include_str!("../../assets/dashboard/app.js");
    let tasks = include_str!("../../assets/dashboard/tasks.js");

    // Selector secondary line: a dedicated element updated from the entry `root`.
    assert!(app.contains("workspace-path"));
    assert!(app.contains("updateWorkspacePath"));
    assert!(app.contains("ws.root"));

    // Task Details box: a "location" field driven by the tagged workspace_root.
    assert!(tasks.contains("workspace_root"));
    assert!(tasks.contains(r#"addField(rightCol, "location""#));
    assert!(tasks.contains("ws-location"));
}

#[test]
fn dashboard_guards_per_workspace_panels_in_aggregate_view() {
    // ORB-00039: in the aggregate "All workspaces" view there is no concrete
    // workspace, so the per-workspace endpoints (/api/crews, /api/tasks/locks,
    // /api/audit/summary, /api/review-threads, /api/scoreboard) must not be
    // fetched — they'd 400 — and their panels show a placeholder instead.
    // Asserted against the embedded asset sources since the dashboard has no JS
    // test runner (see dashboard_markdown_call_sites above).
    let app = include_str!("../../assets/dashboard/app.js");
    let common = include_str!("../../assets/dashboard/common.js");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    // A single aggregate-mode predicate, defined once and reused. ORB-00040 moved
    // it into the shared leaf module (common.js) so audit.js and scoreboard.js can
    // guard on the same live predicate; the raw check lives only inside the helper.
    assert!(
        common.contains("function isAggregateView()"),
        "aggregate-mode check must be factored into isAggregateView() in common.js"
    );
    assert_eq!(
        common
            .matches("multiWorkspace && !currentWorkspace")
            .count(),
        1,
        "the aggregate predicate must be defined once, not duplicated inline"
    );
    assert!(
        !app.contains("function isAggregateView()"),
        "isAggregateView() must be single-sourced in common.js, not redefined in app.js"
    );
    assert_eq!(
        app.matches("const aggregate = isAggregateView();").count(),
        2,
        "isAggregateView() must gate both fetchAndRenderTasks and activeRefreshJobs"
    );

    // Aggregate mode renders placeholders instead of fetching the per-workspace
    // summary / review-threads panels.
    assert!(
        app.contains("renderAggregatePlaceholders()"),
        "aggregate mode must render panel placeholders"
    );
    assert!(
        common.contains("Select a workspace to view this panel"),
        "skipped panels must show an inline placeholder prompt"
    );
    assert!(
        css.contains(".panel-placeholder"),
        "the aggregate placeholder must be styled"
    );

    // The per-workspace summary + review-thread jobs are pushed only in the
    // non-aggregate branch (previously unconditional array literals with a
    // trailing comma) — the old unconditional form must be gone.
    assert!(
        app.contains("jobs.push(fetchAndRenderSummary());"),
        "fetchAndRenderSummary must be pushed conditionally, not unconditionally"
    );
    assert!(
        !app.contains("fetchAndRenderSummary(),"),
        "fetchAndRenderSummary must no longer be an unconditional job"
    );
    // The review-thread panel (per-workspace) is likewise pushed only in the
    // non-aggregate branch, wrapped in the same conditional jobs.push.
    assert!(
        app.contains("fetchAndRenderReviewThreads().then(() => {"),
        "review-threads job must still exist for the concrete-workspace path"
    );

    // Task locks and crews are skipped in aggregate mode; the aggregate task list
    // (/api/tasks/all) still renders with a crew fallback rather than blocking.
    assert!(
        app.contains("if (!aggregate && !document.hidden) jobs.push(fetchAndRenderTaskLocks());"),
        "task locks fetch must be skipped in aggregate mode"
    );
    assert!(
        app.contains("const crews = aggregate ? Promise.resolve() : fetchAndCacheCrews();"),
        "crews fetch must be skipped in aggregate mode so the task list still renders"
    );
}

#[test]
fn dashboard_guards_remaining_panels_in_aggregate_view() {
    // ORB-00040: follow-up to ORB-00039. In the aggregate "All workspaces" view
    // the Audit tab (/api/audit, /api/diagnostics/denials), the Knowledge tab
    // (/api/learnings, /api/adrs, /api/frictions) and the scoreboard window
    // selector (/api/scoreboard?window=...) still fired per-workspace endpoints
    // that 400 and flipped conn-status to red. The isAggregateView() guard is
    // extended to all of them, sharing one predicate from common.js. Asserted
    // against the embedded asset sources (the dashboard has no JS test runner).
    let app = include_str!("../../assets/dashboard/app.js");
    let audit = include_str!("../../assets/dashboard/audit.js");
    let scoreboard = include_str!("../../assets/dashboard/scoreboard.js");
    let common = include_str!("../../assets/dashboard/common.js");

    // The predicate + placeholder helper are shared from the leaf module so all
    // three view modules guard on the same live state without a circular import.
    assert!(
        common.contains("export function isAggregateView()"),
        "isAggregateView() must be exported from common.js for cross-module reuse"
    );
    assert!(
        common.contains("export function setMultiWorkspace("),
        "the multi-workspace flag setter must be exported from common.js"
    );
    assert!(
        common.contains("export function renderPanelPlaceholder("),
        "the shared placeholder renderer must be exported from common.js"
    );
    // Multi-workspace mode is fed once, from the workspace-discovery path.
    assert!(
        app.contains("setMultiWorkspace(dashboardWorkspaces.length > 1)"),
        "aggregate mode must be driven by the discovered workspace count"
    );

    // Audit tab: both subtabs skip their per-workspace fetch and show a
    // placeholder in aggregate mode (events -> /api/audit, policy -> denials).
    assert!(
        audit.contains("isAggregateView") && audit.contains("renderPanelPlaceholder"),
        "audit.js must import the shared aggregate guard"
    );
    assert!(
        audit.contains(r#"renderPanelPlaceholder("audit-body")"#),
        "audit events subtab must show a placeholder in aggregate mode"
    );
    assert!(
        audit.contains(r#"renderPanelPlaceholder("audit-policy-body")"#),
        "audit policy subtab must show a placeholder in aggregate mode"
    );

    // Knowledge tab: each subtab's fetch is guarded at the chokepoint, so the
    // auto-refresh, tab-activation and search-box entry points are all covered.
    for body in ["learnings-body", "adrs-body", "frictions-body"] {
        assert!(
            app.contains(&format!(r#"renderPanelPlaceholder("{body}")"#)),
            "knowledge {body} must show a placeholder in aggregate mode"
        );
    }

    // Scoreboard: the tab body shows a placeholder, and the user-initiated window
    // re-fetch is a no-op in aggregate mode (not just the auto-refresh boot fetch).
    assert!(
        app.contains(r#"renderPanelPlaceholder("scoreboard-body")"#),
        "scoreboard panel must show a placeholder in aggregate mode"
    );
    assert!(
        scoreboard.contains("if (isAggregateView()) return;"),
        "the scoreboard window selector must skip its re-fetch in aggregate mode"
    );

    // The guards read the live predicate before fetching (not a stale captured
    // flag), so switching to "All workspaces" after a concrete selection is safe.
    assert!(
        app.matches("if (isAggregateView())").count() >= 3,
        "each knowledge fetch must guard on the live aggregate predicate"
    );
    assert!(
        audit.matches("if (isAggregateView())").count() >= 2,
        "both audit fetches must guard on the live aggregate predicate"
    );
}

async fn response_body(response: Response) -> String {
    let bytes = match to_bytes(response.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(error) => panic!("read response body: {error}"),
    };
    match String::from_utf8(bytes.to_vec()) {
        Ok(body) => body,
        Err(error) => panic!("response body is not UTF-8: {error}"),
    }
}
