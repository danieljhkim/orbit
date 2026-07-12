//! Rendering tests for `orbit run qa-sweep` report rows [ORB-10039,
//! reworked ORB-10146].

use orbit_core::qa::{QaFindingReport, QaWorkspaceReport};

use crate::command::run::RunSubcommand;
use crate::command::run::qa_sweep::{report_json, report_line};

use super::parse_run;

#[test]
fn parses_qa_sweep_flags() {
    let command = parse_run(&["orbit", "run", "qa-sweep", "--dry-run", "--json"]);
    match command.command {
        RunSubcommand::QaSweep(args) => {
            assert!(args.dry_run);
            assert!(args.json);
        }
        _ => panic!("expected qa-sweep"),
    }
}

#[test]
fn qa_sweep_flags_default_off() {
    let command = parse_run(&["orbit", "run", "qa-sweep"]);
    match command.command {
        RunSubcommand::QaSweep(args) => {
            assert!(!args.dry_run);
            assert!(!args.json);
        }
        _ => panic!("expected qa-sweep"),
    }
}

fn report(action: &'static str) -> QaWorkspaceReport {
    QaWorkspaceReport {
        workspace: "polaris".to_string(),
        action,
        reason: None,
        branch: Some("agent-main".to_string()),
        crew: Some("opus".to_string()),
        head: Some("beefbeefbeefbeef".to_string()),
        baseline: Some("cafecafecafecafe".to_string()),
        new_commits: Some(vec!["beefbeef add thing".to_string()]),
        watermark_reset: false,
        run_id: Some("run-42".to_string()),
        agent_run_id: Some("wrk-7".to_string()),
        findings: vec![QaFindingReport {
            name: "login-redirect-loops".to_string(),
            severity: "high".to_string(),
            fingerprint: "abc123def456".to_string(),
            filed_task: Some("ORB-10101".to_string()),
            deduped_task: None,
        }],
    }
}

#[test]
fn line_carries_range_findings_and_run() {
    let line = report_line(&report("validated"));
    assert_eq!(
        line,
        "polaris: validated — cafecafeca..beefbeefbe — crew opus \
         [login-redirect-loops: high (filed ORB-10101)] — run run-42"
    );
}

#[test]
fn clean_validated_line_marks_clean() {
    let mut clean = report("validated");
    clean.findings.clear();
    assert!(report_line(&clean).contains("[clean]"));
}

#[test]
fn skipped_line_shows_reason_without_range() {
    let mut skipped = report("skipped");
    skipped.reason = Some("no_new_commits".to_string());
    skipped.crew = None;
    skipped.findings.clear();
    skipped.run_id = None;
    assert_eq!(report_line(&skipped), "polaris: skipped — no_new_commits");
}

#[test]
fn deduped_finding_names_the_open_task() {
    let mut deduped = report("validated");
    deduped.findings[0].filed_task = None;
    deduped.findings[0].deduped_task = Some("ORB-10100".to_string());
    assert!(report_line(&deduped).contains("login-redirect-loops: high (open ORB-10100)"));
}

#[test]
fn json_shape_is_stable() {
    let value = report_json(&report("validated"));
    assert_eq!(value["workspace"], "polaris");
    assert_eq!(value["action"], "validated");
    assert_eq!(value["baseline"], "cafecafecafecafe");
    assert_eq!(value["new_commits"], 1);
    assert_eq!(value["run_id"], "run-42");
    assert_eq!(value["agent_run_id"], "wrk-7");
    assert_eq!(value["crew"], "opus");
    assert_eq!(value["findings"][0]["fingerprint"], "abc123def456");
    assert_eq!(value["findings"][0]["filed_task"], "ORB-10101");
    assert_eq!(value["findings"][0]["severity"], "high");
}
