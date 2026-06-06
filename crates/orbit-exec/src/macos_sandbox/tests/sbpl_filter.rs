use super::super::test_support::*;

#[test]
fn compile_strips_glob_suffix_for_subpath_root() {
    let resolved = profile(
        "default",
        &["/Users/test/repo"],
        &["/Users/test/repo/src/**"],
    );
    let text = compile_with_env(&resolved, EnvOverrides::default());
    assert!(
        text.contains("(allow file-write* (subpath \"/Users/test/repo/src\"))"),
        "expected glob-stripped subpath: {text}"
    );
    assert!(
        !text.contains("/src/**"),
        "subpath should not contain glob marker: {text}"
    );
}

#[test]
fn compile_uses_regex_for_non_subpath_positive_modify_glob() {
    let resolved = profile(
        "default",
        &["/Users/test/repo"],
        &["/Users/test/.orbit/orbit.db*"],
    );
    let text = compile_with_env(&resolved, EnvOverrides::default());
    assert!(
        text.contains(
            "(allow file-write* (regex \"(?i)^/Users/test/\\\\.orbit/orbit\\\\.db[^/]*$\"))"
        ),
        "missing regex allow for SQLite sidecar glob: {text}"
    );
    assert!(
        !text.contains("(allow file-write* (subpath \"/Users/test/.orbit\"))"),
        "positive file glob must not collapse to the whole Orbit root: {text}"
    );
}

#[test]
fn compile_appends_explicit_deny_for_negated_modify_rule() {
    let mut resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo"]);
    resolved.modify.push("!/Users/test/repo/.env".to_string());
    let text = compile_with_env(&resolved, EnvOverrides::default());
    assert!(
        text.contains("(deny file-write* (subpath \"/Users/test/repo/.env\"))"),
        "missing deny clause: {text}"
    );
    let allow_pos = text
        .find("(allow file-write* (subpath \"/Users/test/repo\"))")
        .expect("allow clause present");
    let deny_pos = text
        .find("(deny file-write* (subpath \"/Users/test/repo/.env\"))")
        .expect("deny clause present");
    assert!(
        allow_pos < deny_pos,
        "deny clause must come after allow for last-match-wins: {text}"
    );
}

#[test]
fn compile_emits_explicit_read_deny_for_negated_read_rule() {
    // Invariant: `denyRead` rules (negated entries in `read`) must
    // translate to explicit `(deny file-read* ...)` clauses appended
    // after the broad `(allow file-read*)` so they win under
    // last-match-wins. This is the kernel-side complement to
    // `compile_appends_explicit_deny_for_negated_modify_rule`.
    let mut resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo"]);
    resolved.read.push("!/Users/test/repo/.env".to_string());
    let text = compile_with_env(&resolved, EnvOverrides::default());
    assert!(
        text.contains("(deny file-read* (subpath \"/Users/test/repo/.env\"))"),
        "missing deny file-read* clause: {text}"
    );
    let allow_pos = text.find("(allow file-read*)").expect("broad read allow");
    let deny_pos = text
        .find("(deny file-read* (subpath \"/Users/test/repo/.env\"))")
        .expect("read deny clause");
    assert!(
        allow_pos < deny_pos,
        "deny file-read* must come after broad allow for last-match-wins: {text}"
    );
}

#[test]
fn compile_uses_regex_for_non_subpath_negated_read_glob() {
    // Invariant: a `denyRead` rule with a non-trivial glob (e.g.
    // `!**/secrets/**`) must compile to a regex deny clause, not a
    // collapsed subpath that would over-match.
    let mut resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo"]);
    resolved.read.push("!/Users/test/repo/**/*.env".to_string());
    let text = compile_with_env(&resolved, EnvOverrides::default());
    assert!(
        text.contains("(deny file-read* (regex \"(?i)^/Users/test/repo/(?:.*/)?[^/]*\\\\.env$\"))"),
        "missing regex read deny: {text}"
    );
}

#[test]
fn compile_uses_regex_for_non_subpath_negated_modify_glob() {
    let mut resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo"]);
    resolved
        .modify
        .push("!/Users/test/repo/**/*.env".to_string());
    let text = compile_with_env(&resolved, EnvOverrides::default());
    assert!(
        text.contains(
            "(deny file-write* (regex \"(?i)^/Users/test/repo/(?:.*/)?[^/]*\\\\.env$\"))"
        ),
        "missing regex deny for env glob: {text}"
    );
    assert!(
        !text.contains("(deny file-write* (subpath \"/Users/test/repo\"))"),
        "env glob must not collapse to a repo-wide deny: {text}"
    );
}

#[test]
fn regex_deny_filters_match_case_variant_secret_paths() {
    let env_regex = regex_from_filter(&super::super::sbpl_filter::sbpl_filter_for_deny_rule(
        "/Users/test/repo/**/*.env",
    ));
    assert!(
        env_regex.is_match("/Users/test/repo/Secret.ENV"),
        "env deny regex should match case-varied root dotenv paths"
    );
    assert!(
        env_regex.is_match("/Users/test/repo/config/Secret.ENV"),
        "env deny regex should match case-varied nested dotenv paths"
    );

    let orbit_regex = regex_from_filter(&super::super::sbpl_filter::sbpl_filter_for_deny_rule(
        "/Users/test/repo/**/.orbit/**",
    ));
    assert!(
        orbit_regex.is_match("/Users/test/repo/.Orbit/state/task.json"),
        "orbit deny regex should match case-varied .orbit paths"
    );
}

fn regex_from_filter(filter: &str) -> regex::Regex {
    let prefix = "(regex \"";
    let suffix = "\")";
    let escaped_regex = filter
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(suffix))
        .expect("regex filter shape");
    let regex = escaped_regex.replace("\\\\", "\\").replace("\\\"", "\"");
    regex::Regex::new(&regex).expect("valid emitted regex")
}
