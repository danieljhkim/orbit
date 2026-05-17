pub(super) fn sbpl_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn sbpl_filter_for_deny_rule(rule: &str) -> String {
    if rule_can_use_subpath(rule) {
        let path = subpath_root(rule);
        format!("(subpath \"{}\")", sbpl_escape(&path))
    } else {
        let regex = glob_rule_to_regex(rule);
        format!("(regex \"{}\")", sbpl_escape(&regex))
    }
}

pub(super) fn sbpl_filter_for_allow_rule(rule: &str) -> String {
    if rule_can_use_subpath(rule) {
        let path = subpath_root(rule);
        format!("(subpath \"{}\")", sbpl_escape(&path))
    } else {
        let regex = glob_rule_to_regex(rule);
        format!("(regex \"{}\")", sbpl_escape(&regex))
    }
}

fn rule_can_use_subpath(rule: &str) -> bool {
    let trimmed = rule.trim_end_matches('/');
    if !contains_glob(trimmed) {
        return true;
    }
    let Some(prefix) = trimmed.strip_suffix("/**") else {
        return false;
    };
    !contains_glob(prefix)
}

fn contains_glob(value: &str) -> bool {
    value.contains('*') || value.contains('?')
}

fn glob_rule_to_regex(rule: &str) -> String {
    let mut out = String::from("^");
    let chars: Vec<char> = rule.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                if chars.get(i + 2) == Some(&'/') {
                    out.push_str("(?:.*/)?");
                    i += 3;
                } else {
                    out.push_str(".*");
                    i += 2;
                }
            }
            '*' => {
                out.push_str("[^/]*");
                i += 1;
            }
            '?' => {
                out.push_str("[^/]");
                i += 1;
            }
            c => {
                push_regex_escaped(&mut out, c);
                i += 1;
            }
        }
    }
    out.push('$');
    out
}

pub(super) fn push_regex_escaped(out: &mut String, c: char) {
    if matches!(
        c,
        '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\'
    ) {
        out.push('\\');
    }
    out.push(c);
}

pub(super) fn push_regex_escaped_str(out: &mut String, value: &str) {
    for c in value.chars() {
        push_regex_escaped(out, c);
    }
}

/// Strip glob suffixes from a rule so it can be used as a `subpath` root.
/// `subpath` matches a directory and everything beneath, so `**` wildcards
/// are redundant and `*` segments cannot be expressed in SBPL — we collapse
/// them to the longest non-glob prefix.
fn subpath_root(rule: &str) -> String {
    let trimmed = rule.trim_end_matches('/');
    let trimmed = trimmed.trim_end_matches("/**");
    if let Some(idx) = trimmed.find(['*', '?']) {
        let prefix = &trimmed[..idx];
        let prefix = prefix.trim_end_matches('/');
        if prefix.is_empty() {
            "/".to_string()
        } else {
            prefix.to_string()
        }
    } else if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
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
                "(allow file-write* (regex \"^/Users/test/\\\\.orbit/orbit\\\\.db[^/]*$\"))"
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
            text.contains("(deny file-read* (regex \"^/Users/test/repo/(?:.*/)?[^/]*\\\\.env$\"))"),
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
                "(deny file-write* (regex \"^/Users/test/repo/(?:.*/)?[^/]*\\\\.env$\"))"
            ),
            "missing regex deny for env glob: {text}"
        );
        assert!(
            !text.contains("(deny file-write* (subpath \"/Users/test/repo\"))"),
            "env glob must not collapse to a repo-wide deny: {text}"
        );
    }
}
