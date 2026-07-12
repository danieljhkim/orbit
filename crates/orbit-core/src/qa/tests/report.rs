//! QA findings-report parsing tests [ORB-10146]: agents wrap their JSON in
//! prose or fences, so parsing is lenient about the surroundings but strict
//! about the contract — a terminal run with no parseable `findings` object is a
//! bad report the sweep must not treat as a clean pass.

use orbit_common::types::TaskPriority;

use crate::qa::report::{Severity, parse_report, resolve_priority};

#[test]
fn parses_bare_json_object() {
    let report = parse_report(
        r#"{"findings":[{"name":"broken login","severity":"high","summary":"loops","evidence":"repro","commits":["abc feat"]}]}"#,
    )
    .expect("parse");
    assert_eq!(report.findings.len(), 1);
    let finding = &report.findings[0];
    assert_eq!(finding.name, "broken login");
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.summary, "loops");
    assert_eq!(finding.evidence, "repro");
    assert_eq!(finding.commits, vec!["abc feat".to_string()]);
}

#[test]
fn empty_findings_is_a_clean_pass() {
    let report = parse_report(r#"{"findings": []}"#).expect("parse");
    assert!(report.findings.is_empty());
}

#[test]
fn extracts_json_from_a_fenced_block_amid_prose() {
    let raw = "I validated the changes hands-on. Here is my report:\n\n\
               ```json\n\
               {\"findings\": [{\"name\": \"cli flag ignored\", \"severity\": \"medium\"}]}\n\
               ```\n\n\
               Let me know if you need more detail.";
    let report = parse_report(raw).expect("parse");
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].name, "cli flag ignored");
    assert_eq!(report.findings[0].severity, Severity::Medium);
}

#[test]
fn extracts_embedded_object_without_a_fence() {
    let raw = "Final report: {\"findings\": [{\"name\": \"x\", \"severity\": \"low\"}]} done.";
    let report = parse_report(raw).expect("parse");
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].severity, Severity::Low);
}

#[test]
fn missing_findings_key_is_an_error() {
    // Valid JSON, but not the contract: must not be treated as a clean pass.
    assert!(parse_report(r#"{"result": "all good"}"#).is_err());
}

#[test]
fn non_json_output_is_an_error() {
    assert!(parse_report("The build passed and everything looks fine.").is_err());
}

#[test]
fn empty_output_is_an_error() {
    assert!(parse_report("   \n  ").is_err());
}

#[test]
fn unknown_severity_falls_back_to_unknown() {
    let report = parse_report(r#"{"findings":[{"name":"n","severity":"spicy"}]}"#).expect("parse");
    assert_eq!(report.findings[0].severity, Severity::Unknown);
}

#[test]
fn commits_accepts_non_string_values() {
    let report =
        parse_report(r#"{"findings":[{"name":"n","commits":["a subj", 42, ""]}]}"#).expect("parse");
    // Blank entries dropped, non-strings stringified.
    assert_eq!(
        report.findings[0].commits,
        vec!["a subj".to_string(), "42".to_string()]
    );
}

// ---- severity -> priority mapping + clamping -------------------------------

#[test]
fn severity_maps_and_clamps_to_ceiling() {
    // High severity clamps down to a medium ceiling.
    assert_eq!(
        resolve_priority(Severity::High, TaskPriority::Medium),
        TaskPriority::Medium
    );
    // Low severity stays low even under a high ceiling.
    assert_eq!(
        resolve_priority(Severity::Low, TaskPriority::High),
        TaskPriority::Low
    );
    // Critical under a critical ceiling stays critical.
    assert_eq!(
        resolve_priority(Severity::Critical, TaskPriority::Critical),
        TaskPriority::Critical
    );
}

#[test]
fn unknown_severity_uses_the_ceiling_as_default() {
    assert_eq!(
        resolve_priority(Severity::Unknown, TaskPriority::High),
        TaskPriority::High
    );
}
