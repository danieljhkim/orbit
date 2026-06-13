pub(crate) fn source_mentions_symbol(source: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }

    if !needle.chars().all(is_identifier_char) {
        return source.contains(needle);
    }

    let mut search_start = 0usize;
    while let Some(relative_match) = source[search_start..].find(needle) {
        let match_start = search_start + relative_match;
        let match_end = match_start + needle.len();
        let before = source[..match_start].chars().next_back();
        let after = source[match_end..].chars().next();
        let before_ok = before.is_none_or(|ch| !is_identifier_char(ch));
        let after_ok = after.is_none_or(|ch| !is_identifier_char(ch));
        if before_ok && after_ok {
            return true;
        }
        search_start = match_end;
    }

    false
}

pub(crate) fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}
