use axum::body::to_bytes;
use axum::http::{HeaderValue, header};
use axum::response::Response;

use crate::{
    DASHBOARD_CSP, serve_app_js, serve_audit_js, serve_common_js, serve_diagnostics_js,
    serve_index, serve_log_tail_js, serve_markdown_js, serve_marked_js, serve_purify_js,
    serve_reliability_js, serve_router_js, serve_run_detail_js, serve_runs_js, serve_scoreboard_js,
    serve_tasks_js,
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
    // (/api/learnings, /api/frictions) and the scoreboard window
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
    for body in ["learnings-body", "frictions-body"] {
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
        app.matches("if (isAggregateView())").count() >= 2,
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
    // knowledge detail panels (learning-detail / friction-detail) previously
    // kept stale content with live supersede/resolve/patch
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
    for prefix in ["learning", "friction"] {
        assert!(
            app.contains(&format!(r#"renderKnowledgeDetailPlaceholder("{prefix}")"#)),
            "the {prefix} list guard must also clear the stale {prefix}-detail panel"
        );
    }
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

/// ORB-10444: the top-level nav is exactly Tasks, Audit, Diagnostics, Knowledge.
/// A deprecated tab was retired outright — nav entry, route and pane — and
/// Scoreboard, being a diagnostics-shaped view, moved under Diagnostics. A route
/// left behind in `TABS` would resolve to a pane that no longer exists, so the
/// router's tab list is asserted alongside the markup.
#[tokio::test]
async fn dashboard_top_level_nav_is_the_four_operator_tabs() {
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
    assert_eq!(nav, vec!["tasks", "audit", "diagnostics", "knowledge"]);

    assert!(
        router.contains(
            r#"const TABS = ["tasks", "audit", "diagnostics", "knowledge", "run-detail"];"#
        ),
        "the router's tab list must match the nav (plus the hash-only run-detail route)"
    );
    // Every routable tab must still have a pane to render into.
    for tab in ["tasks", "audit", "diagnostics", "knowledge", "run-detail"] {
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
    ] {
        assert!(body.contains(&format!(r#"id="{id}""#)), "{id} must survive");
    }
    // ORB-10588 appended `reliability` to the same list.
    assert!(
        router.contains(
            r#"const DIAG_SUBTABS = ["runs", "metrics", "errors", "reliability", "scoreboard"];"#
        ),
        "the scoreboard must route as a diagnostics subtab"
    );
    assert!(
        app.contains(r#"if (activeDiagSubtab === "scoreboard")"#)
            && app.contains(r#"fetchJson("/api/scoreboard?window=24h")"#),
        "the scoreboard fetch must hang off the diagnostics subtab branch"
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

/// ORB-10444: the Knowledge artifact list is long enough to scroll the detail
/// pane out of view mid-read. The pane is pinned below the fixed chrome and
/// bounded to the remaining viewport so its body scrolls internally rather than
/// being clipped when the detail is taller than the screen.
#[test]
fn dashboard_knowledge_detail_pane_is_sticky_and_internally_scrollable() {
    let css = include_str!("../../assets/dashboard/dashboard.css");

    let sticky_at = css
        .find("#learning-detail-panel,\n      #friction-detail-panel {\n        position: sticky;")
        .expect("the knowledge detail panels must be sticky");
    assert!(
        css[sticky_at..].starts_with(
            "#learning-detail-panel,\n      #friction-detail-panel {\n        position: sticky;\n        top: 170px;\n        align-self: start;\n        max-height: calc(100vh - 194px);",
        ),
        "the pane must pin below the chrome and stay inside the viewport"
    );
    assert!(
        css.contains(
            "#learning-detail-panel > .body,\n      #friction-detail-panel > .body {\n        overflow-y: auto;",
        ),
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
