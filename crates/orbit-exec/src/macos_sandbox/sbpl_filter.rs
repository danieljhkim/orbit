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
