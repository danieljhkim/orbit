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
