use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd
}

fn write_skill(dir: &Path, id: &str, skill_md: &str, meta_json: Option<&str>) {
    let skill_dir = dir.join(".orbit").join("skills").join(id);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(skill_dir.join("SKILL.md"), skill_md).expect("write skill");
    if let Some(meta) = meta_json {
        std::fs::write(skill_dir.join("meta.json"), meta).expect("write meta");
    }
}

#[test]
fn skill_list_and_show_read_file_based_skills() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_skill(
        dir.path(),
        "orbit-assess-codebase",
        r#"---
name: orbit-assess-codebase
description: Perform architectural boundary and invariant analysis.
---

# Assess Codebase

## Purpose
Perform architectural boundary and invariant analysis.

## Behavioral Constraints
- Must return JSON only.

## Output Requirements
- severity_summary
"#,
        Some(
            r#"{
  "name": "Assess Codebase",
  "summary": "Architectural review",
  "tags": ["architecture"],
  "version": "1.0.0",
  "type": "object",
  "required": ["severity_summary"],
  "properties": {
    "severity_summary": { "type": "string" }
  }
}"#,
        ),
    );

    orbit_in(dir.path())
        .args(["skill", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("orbit-assess-codebase"));

    orbit_in(dir.path())
        .args(["skill", "show", "orbit-assess-codebase"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Behavioral Contract"))
        .stdout(predicate::str::contains("Structured Metadata"))
        .stdout(predicate::str::contains("orbit-assess-codebase"));
}

#[test]
fn skill_list_and_show_json_are_valid() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_skill(
        dir.path(),
        "lint-review",
        r#"---
name: lint-review
description: Review lint trends.
---

# Lint Review

## Purpose
Review lint trends.

## Behavioral Constraints
- Deterministic output.

## Output Requirements
- findings
"#,
        None,
    );

    orbit_in(dir.path())
        .args(["skill", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": \"lint-review\""));

    orbit_in(dir.path())
        .args(["skill", "show", "lint-review", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"sections\""))
        .stdout(predicate::str::contains("\"purpose\""));
}

#[test]
fn skill_doctor_reports_invalid_skill() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_skill(
        dir.path(),
        "broken",
        r#"---
name: broken
---

# Broken

## Purpose

"#,
        None,
    );

    orbit_in(dir.path())
        .args(["skill", "doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("broken"))
        .stdout(predicate::str::contains("ERROR"));

    orbit_in(dir.path())
        .args(["skill", "doctor", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\": \"error\""));
}

#[test]
fn legacy_mutation_subcommands_are_not_exposed() {
    let dir = tempfile::tempdir().expect("tempdir");
    orbit_in(dir.path())
        .args(["skill", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("show"))
        .stdout(predicate::str::contains("doctor"))
        .stdout(predicate::str::contains("add").not())
        .stdout(predicate::str::contains("update").not())
        .stdout(predicate::str::contains("delete").not())
        .stdout(predicate::str::contains("attach").not())
        .stdout(predicate::str::contains("detach").not());
}
