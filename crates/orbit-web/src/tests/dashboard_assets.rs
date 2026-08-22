use axum::body::to_bytes;
use axum::http::{HeaderValue, header};
use axum::response::Response;

use crate::{
    DASHBOARD_CSP, serve_app_js, serve_audit_js, serve_common_js, serve_diagnostics_js,
    serve_index, serve_log_tail_js, serve_markdown_js, serve_marked_js, serve_operations_js,
    serve_purify_js, serve_reliability_js, serve_router_js, serve_run_detail_js, serve_runs_js,
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
        ("reliability", serve_reliability_js().await),
        ("log_tail", serve_log_tail_js().await),
        ("diagnostics", serve_diagnostics_js().await),
        ("router", serve_router_js().await),
        ("runs", serve_runs_js().await),
        ("run_detail", serve_run_detail_js().await),
        ("operations", serve_operations_js().await),
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
fn dashboard_omits_retired_duel_surfaces() {
    let index = include_str!("../../assets/dashboard/index.html");
    let scoreboard = include_str!("../../assets/dashboard/scoreboard.js");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    for asset in [index, scoreboard, css] {
        assert!(!asset.to_ascii_lowercase().contains("duel"));
    }
}

#[test]
fn dashboard_renders_normalized_managed_token_usage_without_provider_ranking() {
    let scoreboard = include_str!("../../assets/dashboard/scoreboard.js");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    assert!(scoreboard.contains("Normalized token usage"));
    assert!(scoreboard.contains("unknown model/input basis (excluded, never guessed)"));
    assert!(scoreboard.contains("vs preceding equal window"));
    assert!(scoreboard.contains("lifetime window · no comparison baseline"));
    assert!(scoreboard.contains("Model attribution (not a cross-provider ranking)"));
    assert!(scoreboard.contains(
        "Direct interactive Codex or Claude orchestration-session overhead is excluded."
    ));
    assert!(css.contains(".scoreboard-token-usage"));
    assert!(css.contains("@media (max-width: 620px)"));
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
fn dashboard_run_resume_matches_runtime_guard_and_surfaces_lineage_and_errors() {
    let runs = include_str!("../../assets/dashboard/runs.js");

    assert!(
        runs.contains(
            r#"const RESUMABLE_RUN_STATES = new Set(["failed", "interrupted", "timeout"])"#
        ),
        "Resume must only be offered for states accepted by resume_job_run"
    );
    assert!(
        runs.contains("/api/job-runs/${encodeURIComponent(runId)}/resume"),
        "Resume must POST to the job-run action route"
    );
    assert!(
        runs.contains("re-runs the failed step and all subsequent steps")
            && runs.contains("underlying cause is resolved"),
        "the confirmation must explain checkpoint resume semantics honestly"
    );
    assert!(
        runs.contains("text: `resumed as ${resumedAsId}`")
            && runs.contains("text: `from ${sourceId}`"),
        "the runs table must expose both directions of resumed-run lineage"
    );
    assert!(
        runs.contains(r#"class: "action-error", text: e.message || "resume failed""#),
        "Resume failures must display the server-provided error text"
    );
}

#[test]
fn dashboard_guards_per_workspace_panels_in_aggregate_view() {
    // ORB-00039: in the aggregate "All workspaces" view there is no concrete
    // workspace, so the per-workspace endpoints (/api/crews, /api/tasks/locks,
    // /api/audit/summary, /api/scoreboard) must not be
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
    // summary panel.
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

    // The per-workspace summary job is pushed only in the non-aggregate branch
    // (previously an unconditional array literal with a trailing comma) — the
    // old unconditional form must be gone.
    assert!(
        app.contains("jobs.push(fetchAndRenderSummary());"),
        "fetchAndRenderSummary must be pushed conditionally, not unconditionally"
    );
    assert!(
        !app.contains("fetchAndRenderSummary(),"),
        "fetchAndRenderSummary must no longer be an unconditional job"
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
    // (/api/frictions) and the scoreboard window
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

    // Knowledge tab: the friction fetch is guarded at the chokepoint, so the
    // auto-refresh, tab-activation and search-box entry points are all covered.
    assert!(
        app.contains(r#"renderPanelPlaceholder("frictions-body")"#),
        "knowledge frictions must show a placeholder in aggregate mode"
    );

    // Scoreboard: the tab body shows a placeholder. Window clicks write shared
    // dashboard state; the per-workspace fetch stays on the refresh path, which
    // is already aggregate-guarded above.
    assert!(
        app.contains(r#"renderPanelPlaceholder("scoreboard-body")"#),
        "scoreboard panel must show a placeholder in aggregate mode"
    );
    assert!(
        !scoreboard.contains("fetchJson(`/api/scoreboard"),
        "the scoreboard window selector must not fetch on its own"
    );

    // The guards read the live predicate before fetching (not a stale captured
    // flag), so switching to "All workspaces" after a concrete selection is safe.
    assert!(
        app.matches("if (isAggregateView())").count() >= 1,
        "the remaining knowledge fetch must guard on the live aggregate predicate"
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
    // friction detail panel previously kept stale content with live resolve/patch
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
        // ORB-10871: the incidents subtab is per-workspace too (`Ws` extractor).
        "/api/audit/incidents",
        "/api/diagnostics/implement_one",
        "/api/tasks/completion-by-complexity",
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
    // ORB-10444 added the scoreboard subtab, so there are three re-render sites.
    // ORB-10588's reliability subtab is deliberately not a fourth: it is served
    // by a cross-workspace endpoint (`/api/metrics/reliability` takes the whole
    // DashboardState, not the `Ws` extractor), so it holds no per-workspace
    // state to placehold and stays live in the aggregate view.
    assert_eq!(
        router.matches("isAggregateView()").count(),
        3,
        "every per-workspace subtab re-render site in router.js must be guarded"
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
    assert!(
        app.contains(r#"renderKnowledgeDetailPlaceholder("friction")"#),
        "the friction list guard must also clear the stale detail panel"
    );
}

#[test]
fn dashboard_renders_complexity_as_its_own_dimension() {
    let diagnostics = include_str!("../../assets/dashboard/diagnostics.js");
    assert!(
        diagnostics.contains("Task completion by complexity"),
        "completion-by-complexity panel must exist"
    );
    assert!(
        diagnostics.contains("unset (unlabeled)"),
        "unset complexity must be a named bucket"
    );
    assert!(
        diagnostics.contains("Average implement_one duration by actor (30d) · ${label} · n="),
        "duration-by-actor must be faceted by complexity"
    );
    let app = include_str!("../../assets/dashboard/app.js");
    assert!(
        app.contains("/api/tasks/completion-by-complexity"),
        "completion aggregate must be fetched from the generated index"
    );
}

#[test]
fn dashboard_recent_history_filters_before_limiting() {
    // ORB-10311: the recent-history panel must exclude legacy bare `commented`
    // stubs *before* applying the five-row limit, so meaningful status/workflow
    // events cannot be displaced by comment noise. Asserted against the embedded
    // asset source (the dashboard has no JS test runner).
    let tasks = include_str!("../../assets/dashboard/tasks.js");

    let filter_at = tasks
        .find(r#".filter((h) => h && h.event !== "commented")"#)
        .expect("recent history must drop legacy `commented` stubs");
    let slice_at = tasks
        .find("meaningful.slice(-5).reverse()")
        .expect("recent history must apply the five-row limit to the filtered list");
    assert!(
        filter_at < slice_at,
        "the `commented`-stub filter must run before the recent-history slice"
    );
}

#[test]
fn dashboard_task_detail_shows_orchestrator_as_attribution_not_execution_crew() {
    let tasks = include_str!("../../assets/dashboard/tasks.js");

    assert!(
        tasks.contains(r#"["orchestrator", "orchestrator"]"#),
        "task detail metadata must expose orchestration attribution"
    );
    assert!(
        tasks.contains("for (const [key, label] of TASK_META_FIELDS)"),
        "task detail must render the orchestrator metadata entry"
    );
    assert!(
        tasks.contains(r#"class: "task-crew-select mono""#),
        "execution crew must remain a distinct task-row control"
    );
}

/// ORB-10444/ORB-10875: the top-level nav includes the bounded Operations view.
/// A deprecated tab was retired outright — nav entry, route and pane — and
/// Scoreboard, being a diagnostics-shaped view, moved under Diagnostics. A route
/// left behind in `TABS` would resolve to a pane that no longer exists, so the
/// router's tab list is asserted alongside the markup.
#[tokio::test]
async fn dashboard_top_level_nav_matches_the_operator_tabs() {
    let body = response_body(serve_index().await).await;
    let router = include_str!("../../assets/dashboard/router.js");

    let nav: Vec<&str> = body
        .match_indices(r#"<button class="tab" data-tab=""#)
        .map(|(index, needle)| {
            let rest = &body[index + needle.len()..];
            match rest.find('"') {
                Some(end) => &rest[..end],
                None => panic!("unterminated data-tab attribute in the nav"),
            }
        })
        .collect();
    assert_eq!(
        nav,
        vec!["tasks", "audit", "diagnostics", "operations", "knowledge"]
    );

    assert!(
        router.contains(
            r#"const TABS = ["tasks", "audit", "diagnostics", "operations", "knowledge", "run-detail"];"#
        ),
        "the router's tab list must match the nav (plus the hash-only run-detail route)"
    );
    // Every routable tab must still have a pane to render into.
    for tab in [
        "tasks",
        "audit",
        "diagnostics",
        "operations",
        "knowledge",
        "run-detail",
    ] {
        assert!(
            body.contains(&format!(r#"<section class="tab-pane" data-tab="{tab}">"#)),
            "routable tab `{tab}` must have a pane"
        );
    }
    assert!(
        !body.contains(r#"data-tab="scoreboard""#),
        "Scoreboard must no longer be a top-level tab or pane"
    );
}

#[test]
fn dashboard_operations_are_typed_guarded_and_responsive() {
    let index = include_str!("../../assets/dashboard/index.html");
    let operations = include_str!("../../assets/dashboard/operations.js");
    let router = include_str!("../../assets/dashboard/router.js");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    for id in [
        "routines-body",
        "clock-body",
        "routine-operation-feedback",
        "auto-tasks-body",
        "auto-task-operation-feedback",
        "operations-subtabs",
    ] {
        assert!(index.contains(&format!(r#"id="{id}""#)), "{id}");
    }
    assert!(operations.contains(r#"postJson("/api/routines/toggle""#));
    assert!(operations.contains(r#"postJson("/api/routines/clock""#));
    assert!(operations.contains(r#"postJson("/api/auto-tasks/toggle""#));
    assert!(operations.contains(r#"postJson("/api/auto-tasks/mint""#));
    assert!(operations.contains("pendingOperations.has(key)"));
    assert!(operations.contains("window.confirm("));
    assert!(operations.contains("All-workspace mode is read-only"));
    assert!(operations.contains("routine.target"));
    assert!(operations.contains("last_evaluated_slot"));
    assert!(operations.contains("next_tick_at"));
    assert!(operations.contains("acknowledge_unconditional: true"));
    assert!(operations.contains("UNCONDITIONAL_MINT_WARNING"));
    assert!(operations.contains(
        "Manual mint ignores this definition's schedule, enabled flag, and scheduler dedupe policy."
    ));
    assert!(
        operations.contains("An open instance already exists; this will create another open task.")
    );
    assert!(operations.contains("Minted") || operations.contains("result.message"));
    assert!(operations.contains("Auto-task change failed"));
    assert!(operations.contains("Manual mint failed"));
    assert!(operations.contains("fetchJson(\"/api/auto-tasks\")"));
    assert!(
        !operations.contains("postJson(\"/api/auto-tasks")
            || operations.contains("addEventListener(\"click\"")
    );
    assert!(
        !operations.contains("hashchange") && !operations.contains("location.reload"),
        "refresh/back must not replay a toggle or mint POST"
    );
    assert!(router.contains(r#"const OPERATIONS_SUBTABS = ["routines", "auto-tasks"];"#));
    assert!(router.contains(r#"hash = `#operations/${sub}`;"#));
    assert!(css.contains("@media (max-width: 720px)"));
    assert!(css.contains("@media (max-width: 600px)"));
    assert!(css.contains(".operation-grid { grid-template-columns: 1fr; }"));
    assert!(css.contains("body.operations-active"));
    assert!(css.contains(".operation-mint-warning"));
    assert!(router.contains(r#"classList.toggle("operations-active", top === "operations")"#));
}

/// ORB-10444: Scoreboard content stays reachable after the move — as a
/// Diagnostics subtab whose markup (and therefore every id `scoreboard.js`
/// renders into, so the scoreboard API contract is untouched) lives inside the
/// diagnostics pane.
#[tokio::test]
async fn dashboard_scoreboard_is_reachable_under_diagnostics() {
    let body = response_body(serve_index().await).await;
    let router = include_str!("../../assets/dashboard/router.js");
    let app = include_str!("../../assets/dashboard/app.js");

    let diagnostics_at = body
        .find(r#"<section class="tab-pane" data-tab="diagnostics">"#)
        .expect("diagnostics pane");
    let scoreboard_at = body
        .find(r#"id="diagnostics-scoreboard-main""#)
        .expect("scoreboard host inside diagnostics");
    assert!(
        diagnostics_at < scoreboard_at,
        "the scoreboard markup must live inside the diagnostics pane"
    );
    assert!(
        body.contains(r#"<button class="subtab" data-subtab="scoreboard" type="button">"#),
        "Scoreboard must be offered as a diagnostics subtab"
    );
    // The panels scoreboard.js renders into came across unchanged.
    for id in [
        "scoreboard-body",
        "scoreboard-count",
        "scoreboard-window-selector",
        "scoreboard-narrative",
        "scoreboard-agent-strip",
        "scoreboard-insights",
        "scoreboard-orchestration",
        "scoreboard-orchestration-count",
        "scoreboard-highlights",
    ] {
        assert!(body.contains(&format!(r#"id="{id}""#)), "{id} must survive");
    }
    // ORB-10588 appended `reliability` to the same list.
    assert!(
        router.contains(
            r#"const DIAG_SUBTABS = ["runs", "metrics", "errors", "incidents", "reliability", "scoreboard"];"#
        ),
        "the scoreboard must route as a diagnostics subtab"
    );
    assert!(
        app.contains(r#"if (activeDiagSubtab === "scoreboard")"#)
            && app.contains(
                r#"fetchJson(`/api/scoreboard?window=${encodeURIComponent(selectedWindow)}`)"#
            ),
        "the scoreboard fetch must hang off the diagnostics subtab branch and honor the shared window"
    );
}

/// ORB-10588: the reliability view routes as a diagnostics subtab and owns the
/// ids `reliability.js` renders into.
#[tokio::test]
async fn dashboard_reliability_is_reachable_under_diagnostics() {
    let body = response_body(serve_index().await).await;
    let router = include_str!("../../assets/dashboard/router.js");
    let app = include_str!("../../assets/dashboard/app.js");

    let diagnostics_at = body
        .find(r#"<section class="tab-pane" data-tab="diagnostics">"#)
        .expect("diagnostics pane");
    let reliability_at = body
        .find(r#"id="diagnostics-reliability-main""#)
        .expect("reliability host inside diagnostics");
    assert!(
        diagnostics_at < reliability_at,
        "the reliability markup must live inside the diagnostics pane"
    );
    assert!(
        body.contains(r#"<button class="subtab" data-subtab="reliability" type="button">"#),
        "Reliability must be offered as a diagnostics subtab"
    );
    for id in [
        "reliability-count",
        "reliability-window-selector",
        "reliability-meta",
        "reliability-summary",
        "reliability-denominator-note",
        "reliability-truncation-note",
        "reliability-over-time",
        "reliability-breakdown",
        "reliability-activities",
    ] {
        assert!(body.contains(&format!(r#"id="{id}""#)), "{id} must exist");
    }
    assert!(
        app.contains(r#"if (activeDiagSubtab === "reliability")"#)
            && app.contains("fetchAndRenderReliability()"),
        "the reliability fetch must hang off the diagnostics subtab branch"
    );
    assert!(
        router.contains(r#"reliability: "diagnostics-reliability-main""#),
        "the reliability subtab must claim its own full-width main"
    );
}

/// ORB-10588: a rate is only actionable with its `n` and its window, and a
/// denominator too thin to trust must be withheld rather than rounded. Both
/// rules live in `reliability.js`; this pins them so a later edit cannot
/// quietly turn a withheld cell back into a confident percentage.
#[test]
fn dashboard_reliability_never_renders_a_rate_without_its_denominator() {
    let reliability = include_str!("../../assets/dashboard/reliability.js");
    let index = include_str!("../../assets/dashboard/index.html");

    assert!(
        reliability.contains("rate.low_sample"),
        "the low-sample flag from the API must be honored"
    );
    assert!(
        reliability.contains("n too small"),
        "a withheld rate must say why it is withheld"
    );
    assert!(
        reliability.contains("rel-rate-low"),
        "a withheld rate must be visually distinct from a real one"
    );
    assert!(
        reliability.contains("(n=${n})"),
        "a rendered percentage must carry its denominator"
    );
    assert!(
        reliability.contains("denominator_label"),
        "the denominator's meaning must be rendered, not left in the backend"
    );
    // `all` would be a rate with no stated range; the endpoint refuses it and
    // the selector must not offer it.
    assert!(
        !reliability.contains(r#""all""#),
        "an unbounded window must not be offered"
    );
    assert!(
        !index.contains(
            r#"id="reliability-window-selector" title="window scope">
                <span class="scoreboard-window-seg" data-window="all">"#
        ),
        "the reliability window selector must not offer `all`"
    );
}

/// ORB-10588: the recovery rate must be computed from durable run state only.
/// Friction F-token-disagreement (recorded in the task) makes any token- or
/// cost-derived input untrustworthy, so the reliability path must not read one.
#[test]
fn dashboard_reliability_reads_no_token_or_cost_field() {
    let reliability = include_str!("../../assets/dashboard/reliability.js");
    // Field identifiers, not the words: the module's own header explains *why*
    // it avoids these inputs, so a bare "token" match would flag the rationale.
    for banned in [
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "cache_read_tokens",
        "cache_create_tokens",
        "provider_cost_usd",
        "derived_cost_usd",
        "total_tool_calls",
    ] {
        assert!(
            !reliability.contains(banned),
            "reliability.js must not read `{banned}` — the token/cost inputs disagree across stores"
        );
    }
}

#[test]
fn dashboard_scoreboard_keeps_managed_cost_ownership_out_of_executor_rankings() {
    let scoreboard = include_str!("../../assets/dashboard/scoreboard.js");
    let index = include_str!("../../assets/dashboard/index.html");

    assert!(index.contains("Managed Execution Cost"));
    assert!(scoreboard.contains("renderOrchestrationSummary(summary?.orchestration)"));
    assert!(scoreboard.contains("named orchestrator"));
    assert!(scoreboard.contains("shared task ownership"));
    assert!(scoreboard.contains("unattributed task ownership"));
    assert!(scoreboard.contains("missing linked task"));
    assert!(scoreboard.contains("provider-reported"));
    assert!(scoreboard.contains("Provider-first estimate policy"));
    assert!(scoreboard.contains("derived estimate"));
    assert!(scoreboard.contains("if (known === 0)"));
    assert!(scoreboard.contains("formatUsd(total)"));
    assert!(scoreboard.contains("comparable same-invocation population"));
    assert!(scoreboard.contains("do not reconcile partial sums"));
    assert!(
        scoreboard.contains(
            "Direct interactive Codex or Claude orchestration-session overhead is excluded"
        )
    );
    assert!(scoreboard.contains("invocation < ${until} (exclusive cutoff"));
}

#[test]
fn dashboard_managed_execution_cost_panel_has_responsive_presentation_hooks() {
    let scoreboard = include_str!("../../assets/dashboard/scoreboard.js");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    assert!(
        scoreboard.contains("scoreboard-orchestration-context")
            && scoreboard.contains("scoreboard-orchestration-buckets")
            && scoreboard.contains("scoreboard-orchestration-bucket-head"),
        "scope metadata and ownership buckets need separate presentation groups"
    );
    assert!(
        scoreboard.contains("cost-value")
            && scoreboard.contains("cost-coverage")
            && scoreboard.contains("cost-comparison"),
        "cost amount, coverage, and comparison text need independent styling hooks"
    );
    assert!(
        scoreboard.contains("scoreboard-orchestration-cost primary")
            && scoreboard.contains("\"reported\",\n      true,"),
        "provider-reported cost must receive the primary visual treatment"
    );
    for kind in ["orchestrator", "shared", "unattributed", "missing"] {
        assert!(
            css.contains(&format!(".scoreboard-orchestration-bucket.kind-{kind}")),
            "the {kind} ownership bucket needs a distinct theme-variable accent"
        );
    }
    assert!(
        css.contains("#scoreboard-orchestration-panel {\n        align-self: start;")
            && css.contains("grid-template-columns: repeat(2, minmax(0, 1fr));"),
        "the desktop panel must stay content-height and use a compact bucket grid"
    );
    assert!(
        css.contains("@media (max-width: 900px)")
            && css.contains("@media (max-width: 620px)")
            && css.contains("overflow-wrap: anywhere;"),
        "the managed-cost layout must collapse and wrap safely at narrow widths"
    );
}

/// ORB-10444: the friction list is long enough to scroll the detail
/// pane out of view mid-read. The pane is pinned below the fixed chrome and
/// bounded to the remaining viewport so its body scrolls internally rather than
/// being clipped when the detail is taller than the screen.
#[test]
fn dashboard_knowledge_detail_pane_is_sticky_and_internally_scrollable() {
    let css = include_str!("../../assets/dashboard/dashboard.css");

    let sticky_at = css
        .find("#friction-detail-panel {\n        position: sticky;")
        .expect("the friction detail panel must be sticky");
    assert!(
        css[sticky_at..].starts_with(
            "#friction-detail-panel {\n        position: sticky;\n        top: 170px;\n        align-self: start;\n        max-height: calc(100vh - 194px);",
        ),
        "the pane must pin below the chrome and stay inside the viewport"
    );
    assert!(
        css.contains("#friction-detail-panel > .body {\n        overflow-y: auto;",),
        "detail content taller than the pane must scroll inside it, not be clipped"
    );
    assert!(
        !css.contains("min-height: calc(100vh - 360px)"),
        "the old fixed min-height fought the bounded sticky pane and must be gone"
    );
    // The single-column breakpoint stacks the pane under the list, where
    // pinning would only shrink it — the override must come after the sticky
    // rule so it wins the equal-specificity tie.
    let unpin_at = css
        .find("          position: static;\n          max-height: none;")
        .expect("the narrow-viewport override must exist");
    assert!(sticky_at < unpin_at);
}

/// ORB-10444: the Tasks tab's two write actions. Ship is one click — the
/// dispatch carries the task id alone, so the pipeline resolves the crew from
/// the task and the mode from the workspace — and comments post to the
/// task's review-thread endpoint rather than patching the task record.
#[test]
fn dashboard_task_write_actions_are_configuration_free() {
    let tasks = include_str!("../../assets/dashboard/tasks.js");

    assert!(
        tasks.contains(r#"const SHIP_STATUSES = new Set(["backlog"]);"#),
        "Ship must be offered only on backlog tasks"
    );
    assert!(
        tasks.contains(r#"postJson("/api/workflows/ship", { task_ids: [task.id] })"#),
        "Ship must dispatch the task id with no crew or mode override"
    );
    assert!(
        tasks.contains("taskActionNotice = `${task.id}: ship run ${runId} ${state}`"),
        "the resulting run must be surfaced to the operator"
    );
    assert!(
        tasks.contains(r#"text: `ship failed: ${error.message || String(error)}`"#),
        "a failed dispatch must surface the server error, not silently no-op"
    );
    // A second click must not launch a duplicate run: the guard is taken before
    // the request and released only when the dispatch failed.
    assert!(
        tasks.contains("if (shipInFlightTaskIds.has(task.id)) return;")
            && tasks.contains("shipInFlightTaskIds.add(task.id);"),
        "Ship must guard against a duplicate dispatch from the UI side"
    );
    assert_eq!(
        tasks
            .matches("shipInFlightTaskIds.delete(task.id);")
            .count(),
        1,
        "the in-flight guard may be released on the failure path only"
    );

    assert!(
        tasks.contains(
            r#"postJson(`/api/tasks/${encodeURIComponent(task.id)}/comments`, { message })"#
        ),
        "comments must post to the task's review-thread endpoint"
    );
    assert!(
        !tasks.contains("author:"),
        "the dashboard must not name the comment author; the server records the human identity"
    );
}

/// ORB-10874: the Tasks count previously read an ambiguous `N/50` with no way
/// to tell a total from a page size from a hard cap. It must now state which
/// number means what, using the `/api/tasks` paging envelope
/// (`{ items, total, limit, truncated }`, ORB-10400) when it is available.
#[test]
fn dashboard_task_count_states_shown_total_and_server_limit_explicitly() {
    let tasks = include_str!("../../assets/dashboard/tasks.js");

    assert!(
        tasks.contains("export function formatTaskCount("),
        "the count formatter must be a standalone, testable function"
    );
    assert!(
        tasks.contains("shown") && tasks.contains("total") && tasks.contains("server limit"),
        "the formatter must use explicit shown/total/server-limit language"
    );
    assert!(
        !tasks.contains("filtered.length}/${tasks.length}"),
        "the old ambiguous `N/M` shorthand must be gone"
    );
    assert!(
        tasks.contains("$(\"tasks-count\").textContent = formatTaskCount("),
        "the rendered count must go through the explicit formatter"
    );
}

/// ORB-10874: the status chips and search box are represented in the tasks
/// hash so a reload or the browser's back/forward button restores the same
/// filtered view, mirroring the audit tab's existing buildAuditHash /
/// applyAuditHashQuery pair. A visible summary line states the active filter
/// in words, not just via chip color.
#[test]
fn dashboard_task_filters_are_represented_in_the_url_and_summarized() {
    let tasks = include_str!("../../assets/dashboard/tasks.js");
    let router = include_str!("../../assets/dashboard/router.js");
    let index = include_str!("../../assets/dashboard/index.html");

    assert!(
        tasks.contains("export function buildTasksHash(")
            && tasks.contains("export function applyTasksHashQuery(")
            && tasks.contains("export function syncTaskControls("),
        "tasks.js must expose a hash build/apply/sync trio like audit.js does"
    );
    assert!(
        router.contains("ctx.applyTasksHashQuery(query)")
            && router.contains("ctx.buildTasksHash()"),
        "the router must apply and rebuild the tasks hash on every tasks-tab route"
    );
    assert!(
        tasks.contains("function renderFilterSummary("),
        "the active filter must be restated as text, not only via chip color"
    );
    assert!(
        index.contains(r#"id="task-filter-summary""#) && index.contains(r#"aria-live="polite""#),
        "the filter summary element must exist and announce updates to assistive tech"
    );
}

/// ORB-10942: an explicit all-status selection must survive the hash round
/// trip instead of becoming plain `#tasks`, whose omitted status query means
/// the default set without `someday`. The four cases below are asserted as a
/// deterministic source contract because the dashboard has no JS test runner.
#[test]
fn dashboard_task_filter_hash_round_trips_default_all_someday_and_none() {
    let tasks = include_str!("../../assets/dashboard/tasks.js");
    let app = include_str!("../../assets/dashboard/app.js");

    assert!(
        tasks.contains(r#"sp.set("status", "all")"#) && tasks.contains(r#"statusParam === "all""#),
        "all statuses need a distinct hash representation and matching parser branch"
    );
    assert!(
        tasks.contains(r#"sp.set("status", selected.length > 0 ? selected.join(",") : "none")"#)
            && tasks.contains(r#"statusParam === "none""#),
        "partial and empty selections need stable hash representations"
    );
    assert!(
        tasks.contains("setActiveStatuses(context, new Set(defaultActiveStatuses(context)))"),
        "an omitted status query must retain the documented default set"
    );

    for (label, hash_query, parser_marker) in [
        ("default", "#tasks", "statusParam == null"),
        ("all", "#tasks?status=all", "statusParam === \"all\""),
        (
            "someday-only",
            "#tasks?status=someday",
            "statusParam === \"none\"",
        ),
        (
            "none-selected",
            "#tasks?status=none",
            "statusParam === \"none\"",
        ),
    ] {
        assert!(!hash_query.is_empty(), "{label} hash must be deterministic");
        assert!(
            tasks.contains(parser_marker),
            "{label} parser branch must remain present"
        );
    }
    assert!(
        app.contains("activeStatuses.size > 0 && activeStatuses.size < STATUS_ORDER.length"),
        "single-workspace requests must send only partial active status sets"
    );
}

/// ORB-10874: switching the workspace selector only updated in-memory state,
/// so a reload silently fell back to the server's default workspace instead
/// of the one the operator had selected.
#[test]
fn dashboard_workspace_selection_persists_to_the_url() {
    let app = include_str!("../../assets/dashboard/app.js");

    assert!(
        app.contains("function persistWorkspaceToUrl(") && app.contains("persistScopeToUrl()"),
        "the workspace selector must persist its choice to the URL on every change"
    );
}

/// ORB-10874: the live `orbit.log` panel can now be collapsed and resized,
/// and the presentation choice is remembered locally (not shared/synced
/// state, so localStorage rather than the URL). The task list keeps an
/// explicit minimum height so it can never be squeezed toward zero.
#[test]
fn dashboard_log_panel_is_collapsible_resizable_and_remembers_presentation() {
    let log_tail = include_str!("../../assets/dashboard/log-tail.js");
    let index = include_str!("../../assets/dashboard/index.html");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    assert!(
        log_tail.contains("orbit.dashboard.logPanel"),
        "the log panel's collapsed/height preference must persist to localStorage"
    );
    assert!(
        log_tail.contains("function wireLogPanelToggle(")
            && log_tail.contains("function wireLogPanelResizeHandle("),
        "the log panel must offer both a collapse toggle and a resize handle"
    );
    assert!(
        index.contains(r#"id="log-panel-toggle""#) && index.contains(r#"id="log-panel-resize""#),
        "the toggle and resize handle must exist in the markup"
    );
    assert!(
        css.contains("#log-panel.collapsed") && css.contains(".log-resize-handle"),
        "the collapsed state and the resize handle must be styled"
    );
    assert!(
        css.contains("#tasks-panel > .body") && css.contains("min-height: 240px;"),
        "the task list must keep a guaranteed minimum usable height regardless of log panel state"
    );
}

/// ORB-10874: inline status/crew edits must show a pending state, refuse a
/// second submission while one is in flight, report durable success/failure
/// text (not just console.error), and offer a bounded undo while the prior
/// value can still be safely restored.
#[test]
fn dashboard_inline_task_edits_report_pending_success_failure_and_offer_undo() {
    let tasks = include_str!("../../assets/dashboard/tasks.js");

    assert!(
        tasks.contains(r#"{ kind: "pending", text: "saving…" }"#),
        "a status/crew change must show a pending state"
    );
    assert!(
        tasks.contains(r#"kind: "success""#) && tasks.contains(r#"kind: "error""#),
        "a status/crew change must report durable success or failure feedback"
    );
    assert!(
        tasks.contains("class: \"mutation-undo\", text: \"undo\""),
        "a successful change must offer an undo control"
    );
    assert!(
        tasks.contains("MUTATION_UNDO_WINDOW_MS") && tasks.contains("scheduleFeedbackExpiry("),
        "undo must be bounded to a window, not offered indefinitely"
    );
    assert!(
        tasks.contains("(feedback && feedback.kind === \"pending\")"),
        "the control must disable itself while its own change is pending"
    );
}

/// ORB-10874: in the aggregate ("All workspaces") view there is no ambient
/// workspace to scope a status/crew mutation to. A task fetched through
/// /api/tasks/all carries its own workspace_id (ORB-00037); mutation is
/// refused unless that explicit, workspace-qualified target is available.
#[test]
fn dashboard_aggregate_view_guards_inline_task_mutations() {
    let tasks = include_str!("../../assets/dashboard/tasks.js");

    assert!(
        tasks.contains("function canMutateTask(task) {")
            && tasks.contains("!isAggregateView() || Boolean(task && task.workspace_id)"),
        "mutation must be refused in aggregate mode unless the task names its own workspace"
    );
    assert!(
        tasks.contains("function taskMutationPath(task"),
        "an aggregate-mode mutation must target the task's own workspace explicitly, not the ambient one"
    );
    assert_eq!(
        tasks.matches("!mutable").count(),
        2,
        "both the status and crew controls must be disabled when the task cannot be safely mutated"
    );
}

/// ORB-10444: dashboard assets are a shipped, project-agnostic surface. A
/// personal name, an Orbit/knowledge id, or a checkout path baked into them
/// would ship to every install, so the served assets carry none.
#[test]
fn dashboard_assets_carry_no_project_specific_identifiers() {
    let assets = [
        (
            "index.html",
            include_str!("../../assets/dashboard/index.html"),
        ),
        (
            "dashboard.css",
            include_str!("../../assets/dashboard/dashboard.css"),
        ),
        ("app.js", include_str!("../../assets/dashboard/app.js")),
        (
            "common.js",
            include_str!("../../assets/dashboard/common.js"),
        ),
        (
            "markdown.js",
            include_str!("../../assets/dashboard/markdown.js"),
        ),
        ("tasks.js", include_str!("../../assets/dashboard/tasks.js")),
        ("audit.js", include_str!("../../assets/dashboard/audit.js")),
        (
            "scoreboard.js",
            include_str!("../../assets/dashboard/scoreboard.js"),
        ),
        (
            "log-tail.js",
            include_str!("../../assets/dashboard/log-tail.js"),
        ),
        (
            "diagnostics.js",
            include_str!("../../assets/dashboard/diagnostics.js"),
        ),
        (
            "router.js",
            include_str!("../../assets/dashboard/router.js"),
        ),
        ("runs.js", include_str!("../../assets/dashboard/runs.js")),
        (
            "run-detail.js",
            include_str!("../../assets/dashboard/run-detail.js"),
        ),
        (
            "reliability.js",
            include_str!("../../assets/dashboard/reliability.js"),
        ),
        (
            "operations.js",
            include_str!("../../assets/dashboard/operations.js"),
        ),
    ];
    // Personal names and layout paths of the machine Orbit is developed on, plus
    // the workspace names it registers. `orbit`/`ORB-` themselves are the
    // product's own vocabulary and are not project-specific.
    let banned = [
        "daniel",
        "/home/",
        "constellation",
        "knowledgebase",
        "polaris",
        "almanac",
        "dk-server",
        "sextant",
        "agentbase",
    ];

    for (name, source) in assets {
        let lowered = source.to_lowercase();
        for needle in banned {
            assert!(
                !lowered.contains(needle),
                "{name} must not name `{needle}` — dashboard assets ship to every install"
            );
        }
        for (line_index, line) in source.lines().enumerate() {
            // Knowledge-artifact ids (L-0021, ADR-0001, F2026-07-015) name
            // records that exist only in the authoring workspace. Task ids are
            // the exception: they are the repo's own change provenance and are
            // cited in comments across the codebase.
            for prefix in ["L-", "ADR-", "F20"] {
                assert!(
                    !line.contains(prefix),
                    "{name}:{} references a knowledge id (`{prefix}…`): {line}",
                    line_index + 1
                );
            }
        }
    }
}

/// ORB-10872: workspace + window are one dashboard scope. Scoreboard and
/// Managed Execution honor the same window; a mismatched 24h payload is
/// refused under a 7d selection; Reliability labels Fleet-wide; Audit
/// drill-downs expose removable chips; the URL restores the scope.
#[test]
fn dashboard_scope_is_shared_labeled_and_url_backed() {
    let common = include_str!("../../assets/dashboard/common.js");
    let app = include_str!("../../assets/dashboard/app.js");
    let router = include_str!("../../assets/dashboard/router.js");
    let scoreboard = include_str!("../../assets/dashboard/scoreboard.js");
    let reliability = include_str!("../../assets/dashboard/reliability.js");
    let audit = include_str!("../../assets/dashboard/audit.js");
    let index = include_str!("../../assets/dashboard/index.html");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    assert!(
        common.contains("export function getWindow(")
            && common.contains("export function setWindow(")
            && common.contains("export function payloadHonorsWindow(")
            && common.contains("export function persistScopeToUrl(")
            && common.contains("export function reliabilityWindowFor("),
        "common.js must own the shared dashboard window and payload-window guard"
    );
    assert!(
        common.contains("if (typeof reported === \"string\") return reported === selected;")
            && common.contains("reported.label === selected"),
        "payloadHonorsWindow must reject a 24h body under an active 7d selection"
    );
    assert!(
        app.contains("if (!payloadHonorsWindow(summary, selectedWindow))")
            && scoreboard.contains("if (summary && !payloadHonorsWindow(summary, getWindow()))"),
        "the scoreboard fetch and renderer must refuse a mismatched window payload"
    );
    assert!(
        reliability.contains("Fleet-wide")
            && index.contains(r#"id="reliability-scope-badge""#)
            && index.contains("Fleet-wide")
            && reliability.contains(r#"payload.scope === "workspace""#),
        "Reliability must label Fleet-wide when it ignores the selected workspace"
    );
    assert!(
        router.contains("markWorkspaceSelectorScope")
            && router.contains("Reliability is Fleet-wide; workspace does not apply")
            && css.contains(".workspace-select.scope-ignored")
            && css.contains(".scope-badge.independent"),
        "the workspace selector must not imply a scope Reliability does not use"
    );
    assert!(
        audit.contains("function navigateToDrilldown(")
            && audit.contains("function renderScopeChips(")
            && audit.contains(r#"removableChip("actor""#)
            && audit.contains(r#"removableChip("workspace""#)
            && audit.contains(r#"removableChip("window""#)
            && audit.contains(r#"removableChip("status""#)
            && audit.contains(r#"removableChip("metric""#)
            && index.contains(r#"id="audit-scope-chips""#),
        "actor/metric drill-down must show removable actor/workspace/window/status/metric chips"
    );
    assert!(
        router
            .contains(r#"hash = `#diagnostics/${sub}?window=${encodeURIComponent(getWindow())}`"#)
            && common.contains(r#"url.searchParams.set("window", currentWindow)"#)
            && audit.contains("sp.set(\"metric\", auditFilter.metric)"),
        "workspace, diagnostics subview, window, and drill-down filters must live in the URL"
    );
    assert!(
        css.contains("@media (max-width: 720px)")
            && css.contains("@media (max-width: 520px)")
            && css.contains(".scope-chip-v")
            && css.contains("max-width: 10ch"),
        "scope badges and filter chips must stay legible at 480–720px"
    );
}

/// ORB-10871: a repeated failure burst is one incident, not hundreds of
/// independent quality failures. The dashboard must therefore (a) show the
/// grouped count and the raw failed-event count side by side, each with its
/// denominator and the selected window, (b) let an operator expand an incident
/// down to the exact audit rows, actor, surfaces, run/task ids, first/last
/// timestamps, and grouping signature, and (c) never imply that a propagated
/// pipeline failure is its own root cause. All three live in the assets; this
/// pins them so a later edit cannot quietly go back to counting raw rows.
#[test]
fn dashboard_failure_metrics_are_incident_aware_and_state_their_denominators() {
    let index = include_str!("../../assets/dashboard/index.html");
    let app = include_str!("../../assets/dashboard/app.js");
    let router = include_str!("../../assets/dashboard/router.js");
    let diagnostics = include_str!("../../assets/dashboard/diagnostics.js");
    let scoreboard = include_str!("../../assets/dashboard/scoreboard.js");
    let audit = include_str!("../../assets/dashboard/audit.js");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    // Routed as a diagnostics subtab, fetched against the shared window.
    assert!(
        index.contains(r#"<button class="subtab" data-subtab="incidents" type="button">"#),
        "Incidents must be offered as a diagnostics subtab"
    );
    assert!(
        router.contains(r#""incidents""#),
        "the incidents subtab must be routable"
    );
    assert!(
        app.contains(r#"if (activeDiagSubtab === "incidents")"#)
            && app.contains("/api/audit/incidents?since=${encodeURIComponent(selectedWindow)}"),
        "the incidents fetch must hang off the diagnostics subtab branch and honor the shared window"
    );

    // Both counts, both denominators, and the window are rendered — never one
    // number standing in for the other.
    assert!(
        diagnostics.contains("${asCount(payload.incident_count)} incidents / ${asCount(payload.raw_failed_events)} failed events"),
        "the panel count must show grouped incidents and raw failed events together"
    );
    assert!(
        diagnostics.contains("grouped from ${failed} failed events of ${total} audited events"),
        "the incident count must state what it is out of"
    );
    assert!(
        diagnostics.contains("`window ${window}`"),
        "the incident summary must name the window it was measured over"
    );
    assert!(
        diagnostics.contains("incident-class-chip") && diagnostics.contains("INCIDENT_CLASS_ORDER"),
        "denials, expected negative paths, and unexpected failures must stay distinguishable"
    );

    // Expansion exposes the underlying evidence.
    for needle in [
        "grouping signature",
        "first seen",
        "last seen",
        "\"actor\"",
        "\"runs\"",
        "\"tasks\"",
        "Underlying audit events",
    ] {
        assert!(
            diagnostics.contains(needle),
            "incident expansion must reveal `{needle}`"
        );
    }
    assert!(
        diagnostics.contains("downstream failures, not independent root causes"),
        "a propagation chain must be labeled as a chain, not as separate root causes"
    );
    assert!(
        diagnostics.contains("navigateToDrilldown(")
            && diagnostics.contains("Open raw audit events"),
        "an incident must link out to the raw audit rows it collapsed"
    );
    assert!(
        audit.contains("auditFilter.tool = opts.tool || null;"),
        "the drill-down must carry the incident's surface into the raw Audit filter"
    );

    // Scoreboard keeps the raw failure column and gains the grouped one.
    assert!(
        scoreboard.contains(r#"left: "failed_tool_calls""#)
            && scoreboard.contains(r#"right: "tool_calls""#),
        "the raw failed/total tool-call pair must survive"
    );
    assert!(
        scoreboard.contains(r#"key: "failure_incidents""#)
            && scoreboard.contains(r#"left: "failure_incidents""#)
            && scoreboard.contains(r#"right: "failure_incident_events""#),
        "the scoreboard must show grouped incidents against the raw events they collapsed"
    );
    assert!(
        scoreboard.contains("function allScoreboardSections()")
            && scoreboard.contains("window ${window}"),
        "every scoreboard section badge must name the selected window"
    );

    // Narrow-viewport presentation hooks (480–720px).
    assert!(
        css.contains(".incident-summary")
            && css.contains(".incident-facts")
            && css.contains(".incident-evidence"),
        "the incident summary and its expansion need their own presentation hooks"
    );
    let responsive_at = css
        .rfind("@media (max-width: 720px)")
        .expect("a 720px breakpoint must exist");
    assert!(
        css[responsive_at..].contains(".incident-facts { grid-template-columns: minmax(0, 1fr);")
            && css[responsive_at..].contains(".incident-evidence"),
        "the incident expansion must reflow rather than clip below 720px"
    );
}

/// ORB-10873: Scoreboard delivery highlights, honest empty-section coverage,
/// accessible window tabs, and labeled abbreviations. Assets stay
/// project-agnostic.
#[test]
fn dashboard_scoreboard_highlights_are_accessible_and_honest() {
    let index = include_str!("../../assets/dashboard/index.html");
    let scoreboard = include_str!("../../assets/dashboard/scoreboard.js");
    let common = include_str!("../../assets/dashboard/common.js");
    let css = include_str!("../../assets/dashboard/dashboard.css");

    assert!(
        index.contains(r#"id="scoreboard-window-selector" role="tablist" aria-label="Scoreboard time window""#)
            && index.contains(r#"id="reliability-window-selector" role="tablist" aria-label="Reliability time window""#),
        "window controls must be semantic tablists"
    );
    assert!(
        index.contains(
            r#"role="tab" class="scoreboard-window-seg on" data-window="24h" aria-selected="true""#
        ) && common.contains("setAttribute(\"aria-selected\"")
            && common.contains("ArrowRight")
            && common.contains("ArrowLeft")
            && common.contains("Home")
            && common.contains("End"),
        "window tabs must expose selected state and keyboard navigation"
    );
    assert!(
        css.contains(".scoreboard-window-seg:focus-visible"),
        "window tabs must have a visible focus ring"
    );

    assert!(
        scoreboard.contains("Notable completions")
            && scoreboard.contains("not a quality score")
            && scoreboard.contains("No completion summary recorded.")
            && scoreboard.contains("function renderNotableCompletions("),
        "highlights must name the reading order and missing summaries"
    );
    assert!(
        !scoreboard.contains("quality score") || scoreboard.contains("not a quality score"),
        "the UI must not claim an objective quality score"
    );
    assert!(
        scoreboard.contains("no observed review comments in this source")
            && scoreboard.contains("coverage?.review?.availability === \"unavailable\"")
            && scoreboard.contains("missing coverage, not zero activity"),
        "empty Review must distinguish no events from incomplete coverage"
    );
    assert!(
        scoreboard.contains("orbit.task.* tool-call count")
            && scoreboard.contains("raw failed tool calls over total tool calls")
            && scoreboard.contains("append-only friction reports filed by this agent")
            && scoreboard.contains("Highest count in this row. Not a quality score."),
        "abbreviated metrics and the leader mark need plain-language definitions"
    );
    assert!(
        !scoreboard.contains("frict r"),
        "the unexplained frict r abbreviation must be gone"
    );

    assert!(
        css.contains(".scoreboard-highlights")
            && css.contains(".scoreboard-highlight-excerpt")
            && css.contains("overflow-wrap: anywhere;"),
        "highlights must wrap instead of clipping"
    );
    let scoreboard_720 = css
        .find("table.sb2-matrix col.metric { width: 132px; }")
        .expect("narrow scoreboard metric column");
    assert!(
        css[..scoreboard_720].contains("@media (max-width: 720px)"),
        "matrix labels must wrap at 480–720px"
    );

    for banned in ["constellation", "dk-server", "polaris", "SpaceX"] {
        assert!(
            !scoreboard
                .to_ascii_lowercase()
                .contains(&banned.to_ascii_lowercase()),
            "scoreboard assets must stay project-agnostic; found {banned}"
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
