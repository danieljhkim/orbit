use orbit_common::utility::selector::canonical_selector;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkippedEntry {
    pub raw_entry: String,
    pub reason: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct PlanContextFilesExtraction {
    /// Canonical, deduped selectors in first-seen order. Empty when the section
    /// was recognized but yielded zero usable entries.
    pub canonical_entries: Vec<String>,
    pub skipped: Vec<SkippedEntry>,
}

/// Extract a canonical `context_files` list from a winning planning-duel plan.
///
/// Section recognition is deliberately strict: the plan must contain a heading
/// line at level `##` or `###` whose trimmed, case-insensitive text equals
/// `context files` or `context_files` (a single trailing `:` is permitted).
/// The section body extends to the next heading of equal-or-higher level, or
/// end of input. Anything outside this strict shape returns `None`, which the
/// writeback treats as "preserve the existing field".
///
/// Bullets within the section body (`- ` or `* ` prefix on an unindented line)
/// contribute one entry each: the first inline-code span if present, otherwise
/// the first whitespace-bounded token after the bullet marker. Each entry is
/// canonicalized via [`canonical_selector`]; entries that fail canonicalization
/// are dropped and reported in `skipped`. Duplicates are collapsed in
/// first-seen order.
///
/// Returns `None` for: section absent, section recognized but with zero
/// canonicalized entries (e.g. placeholder section or every bullet
/// unparseable). Returns `Some(extraction)` only when at least one canonical
/// entry was produced; `extraction.skipped` carries dropped entries even in
/// the success case so callers can record observability events.
pub(super) fn extract_context_files_from_plan(plan: &str) -> Option<PlanContextFilesExtraction> {
    let lines: Vec<&str> = plan.lines().collect();
    let section_range = find_context_files_section(&lines)?;
    let section = &lines[section_range];

    let mut canonical_entries: Vec<String> = Vec::new();
    let mut skipped: Vec<SkippedEntry> = Vec::new();

    for line in section {
        let Some(raw_entry) = bullet_entry(line) else {
            continue;
        };
        match canonical_selector(&raw_entry) {
            Ok(canonical) => {
                if !canonical_entries.contains(&canonical) {
                    canonical_entries.push(canonical);
                }
            }
            Err(err) => {
                skipped.push(SkippedEntry {
                    raw_entry,
                    reason: err.reason,
                });
            }
        }
    }

    if canonical_entries.is_empty() {
        return None;
    }

    Some(PlanContextFilesExtraction {
        canonical_entries,
        skipped,
    })
}

fn find_context_files_section(lines: &[&str]) -> Option<std::ops::Range<usize>> {
    let mut start: Option<usize> = None;
    let mut start_level: usize = 0;
    let mut end = lines.len();

    for (idx, line) in lines.iter().enumerate() {
        let Some((level, heading_text)) = parse_heading(line) else {
            continue;
        };

        if start.is_none() {
            if (level == 2 || level == 3) && heading_matches_context_files(heading_text) {
                start = Some(idx + 1);
                start_level = level;
            }
            continue;
        }

        if level <= start_level {
            end = idx;
            break;
        }
    }

    let body_start = start?;
    if body_start > end {
        return None;
    }
    Some(body_start..end)
}

fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let hash_count = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if hash_count == 0 || hash_count > 6 {
        return None;
    }
    let after_hashes = &trimmed[hash_count..];
    if !after_hashes.starts_with(' ') && !after_hashes.is_empty() {
        return None;
    }
    Some((hash_count, after_hashes.trim()))
}

fn heading_matches_context_files(heading_text: &str) -> bool {
    let normalized = heading_text.trim_end_matches(':').trim().to_lowercase();
    normalized == "context files" || normalized == "context_files"
}

fn bullet_entry(line: &str) -> Option<String> {
    let leading_spaces = line.bytes().take_while(|byte| *byte == b' ').count();
    if leading_spaces >= 2 {
        return None;
    }
    let trimmed = line.trim_start();
    let after_marker = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))?;
    let after_marker = after_marker.trim_start();
    if after_marker.is_empty() {
        return None;
    }

    if let Some(after_open) = after_marker.strip_prefix('`')
        && let Some(close) = after_open.find('`')
    {
        let entry = after_open[..close].trim();
        if !entry.is_empty() {
            return Some(entry.to_string());
        }
    }

    let token_end = after_marker
        .find(|c: char| c.is_whitespace())
        .unwrap_or(after_marker.len());
    let raw_token = &after_marker[..token_end];
    let token = raw_token.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ')' | '('));
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}
