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
    // ORB-10124: the selector shows only the selected workspace's label — the
    // secondary filesystem-path line (ORB-00037) was removed as distracting
    // implementation detail. Each aggregate task still shows its workspace
    // location in the Details box (a separate, unrelated feature). Asserted
    // against the embedded asset sources since the dashboard has no JS test
    // runner (see dashboard_markdown_call_sites above).
    let app = include_str!("../../assets/dashboard/app.js");
    let tasks = include_str!("../../assets/dashboard/tasks.js");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    // Selector secondary line must be gone, along with its update helper and style.
    assert!(
        !app.contains("workspace-path"),
        "the selector must no longer render a secondary filesystem-path line"
    );
    assert!(
        !app.contains("updateWorkspacePath"),
        "updateWorkspacePath must be removed along with the path line it rendered"
    );
    assert!(
        !css.contains(".workspace-path"),
        "the workspace-path CSS rule must be removed with its markup"
    );

    // Task Details box: a "location" field driven by the tagged workspace_root.
    assert!(tasks.contains("workspace_root"));
    assert!(tasks.contains(r#"addField(rightCol, "location""#));
    assert!(tasks.contains("ws-location"));
}

#[test]
fn dashboard_task_actions_route_to_selected_workspace() {
    // ORB-10124: approve/reject/archive built their request with a raw
    // `fetch()`, bypassing the `withWorkspace()` helper that every other
    // dashboard request goes through (fetchJson/requestJson in common.js).
    // Against a remote registered workspace the mutation silently applied to
    // the default workspace (or 400'd) instead of the selected one, so the
    // dashboard never reflected the change. Asserted against the embedded
    // asset sources since the dashboard has no JS test runner.
    let tasks = include_str!("../../assets/dashboard/tasks.js");
    let common = include_str!("../../assets/dashboard/common.js");

    assert!(
        common.contains("export function withWorkspace("),
        "withWorkspace must be exported from common.js so other modules can reuse it"
    );
    assert!(
        tasks.contains("withWorkspace } from './common.js'"),
        "tasks.js must import withWorkspace from common.js"
    );
    assert!(
        tasks.contains(
            "fetch(withWorkspace(opts.path || `/api/tasks/${encodeURIComponent(task.id)}/${kind}`)"
        ),
        "runAction (approve/reject/archive) must route its request through withWorkspace"
    );
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

#[test]
fn dashboard_guards_diagnostics_and_detail_panels_in_aggregate_view() {
    // ORB-00044: follow-up to ORB-00039/00040 closing the two remaining
    // aggregate-mode gaps. (1) The Diagnostics tab is fed exclusively by
    // per-workspace endpoints (/api/job-runs plus /api/diagnostics/metrics,
    // /errors, /friction and /implement_one all take the `Ws` extractor and 400
    // without a concrete workspace), so in aggregate mode the whole tab branch
    // of activeRefreshJobs is skipped and placeholders render instead. (2) The
    // knowledge detail panels (learning-detail / adr-detail / friction-detail)
    // previously kept stale content with live supersede/accept/resolve/patch
    // buttons after switching to "All workspaces"; they now show the shared
    // placeholder too. Asserted against the embedded asset sources (the
    // dashboard has no JS test runner).
    let app = include_str!("../../assets/dashboard/app.js");
    let router = include_str!("../../assets/dashboard/router.js");

    fn index_of(source: &str, name: &str, needle: &str) -> usize {
        match source.find(needle) {
            Some(index) => index,
            None => panic!("{name} must contain `{needle}`"),
        }
    }

    // Diagnostics: the aggregate guard sits at the top of the diagnostics
    // branch, before any of the per-workspace fetch sites — every diagnostics
    // fetch in activeRefreshJobs is unreachable in aggregate mode.
    let branch = index_of(app, "app.js", r#"if (activeTab === "diagnostics") {"#);
    let guard = index_of(
        app,
        "app.js",
        "renderDiagnosticsPlaceholders();\n      return jobs;",
    );
    assert!(
        branch < guard,
        "the aggregate guard must live inside the diagnostics branch"
    );
    for fetch in [
        "/api/diagnostics/metrics",
        "/api/diagnostics/errors",
        "/api/diagnostics/implement_one",
        "fetchAndRenderRuns()",
    ] {
        let fetch_at = index_of(app, "app.js", fetch);
        assert!(
            guard < fetch_at,
            "diagnostics fetch `{fetch}` must come after the aggregate early-return"
        );
    }
    // fetchAndRenderRuns (the "runs" subtab job, /api/job-runs +
    // /api/diagnostics/friction) is only invoked from the guarded branch: one
    // guarded call site plus the function definition itself.
    assert_eq!(
        app.matches("fetchAndRenderRuns()").count(),
        2,
        "fetchAndRenderRuns must have no unguarded call site"
    );

    // The placeholder helper covers both subtab bodies and the side card, and
    // neutralizes the count.
    assert!(
        app.contains("function renderDiagnosticsPlaceholders()"),
        "aggregate mode must render diagnostics placeholders"
    );
    for body in ["diag-body", "runs-body", "diag-implement-one-body"] {
        assert!(
            app.contains(&format!(r#"renderPanelPlaceholder("{body}")"#)),
            "diagnostics {body} must show a placeholder in aggregate mode"
        );
    }

    // A diagnostics subtab switch re-renders from the stale last* caches in
    // router.js, so it guards on the same live predicate instead of repainting
    // the previous workspace's rows over the placeholder.
    assert!(
        router.contains("isAggregateView"),
        "router.js must import the shared aggregate guard"
    );
    for body in ["diag-body", "runs-body"] {
        assert!(
            router.contains(&format!(r#"renderPanelPlaceholder("{body}")"#)),
            "router subtab switch must render the {body} placeholder in aggregate mode"
        );
    }
    assert_eq!(
        router.matches("if (isAggregateView())").count(),
        2,
        "both subtab re-render sites in router.js must be guarded"
    );

    // Knowledge detail panels: each list guard also replaces its detail panel
    // (which carries the per-workspace action buttons) with the placeholder.
    assert!(
        app.contains("function renderKnowledgeDetailPlaceholder(prefix)"),
        "the detail-panel placeholder helper must exist"
    );
    assert!(
        app.contains("renderPanelPlaceholder(`${prefix}-detail`)"),
        "the helper must target the <prefix>-detail panels"
    );
    for prefix in ["learning", "adr", "friction"] {
        assert!(
            app.contains(&format!(r#"renderKnowledgeDetailPlaceholder("{prefix}")"#)),
            "the {prefix} list guard must also clear the stale {prefix}-detail panel"
        );
    }
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
