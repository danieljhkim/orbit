//! The friction record title — one short handle per record.
//!
//! A friction record's handle is an author-supplied `title` on the record
//! ([`normalize_title`]). Derivation from the body ([`derive_title`]) is the
//! fallback for a record whose author supplied none, and the only source for
//! records written before the field existed.
//!
//! Derivation reads the body's *structure*, never its vocabulary. A markdown
//! document that opens with a heading is doing one of two things, and the
//! difference is structural rather than lexical:
//!
//! - the heading is the document's own title — it is the only heading at its
//!   level or shallower, so nothing else in the body claims to be a peer;
//! - the heading labels the first *section* of a structured report — sibling
//!   headings at the same level follow it, so the label describes a part of
//!   the record rather than the record.
//!
//! In the second case the subject of the record is the prose the label
//! introduces, so derivation skips the label. The same reasoning applies to a
//! bold run that opens a prose line: an inline lead-in labels the sentence
//! that follows it on the same line, so the sentence is the candidate.
//!
//! No list of known section headings appears here. Such a list is a symptom
//! fix — it is unmaintainable across authors and untranslatable across
//! languages — and it cannot help the opposite failure, an opening paragraph
//! long enough to be unreadable in any list view. [`FRICTION_TITLE_MAX_CHARS`]
//! bounds that one.

use crate::types::OrbitError;

/// Maximum length, in characters, of a friction record title.
///
/// Long enough for a specific one-line problem statement, short enough to sit
/// in a CLI table column or a dashboard row without wrapping.
pub const FRICTION_TITLE_MAX_CHARS: usize = 120;

/// The ellipsis appended to a title clamped at [`FRICTION_TITLE_MAX_CHARS`].
const ELLIPSIS: char = '…';

/// Shortest clamped prefix worth cutting back to a word boundary for. Below
/// this, honouring the boundary would discard more than it saves.
const MIN_WORD_BOUNDARY_PREFIX: usize = FRICTION_TITLE_MAX_CHARS / 2;

/// Normalize and validate an author-supplied title.
///
/// A title is a single line: interior newlines and whitespace runs collapse to
/// single spaces so the stored value renders the same in every consumer.
pub fn normalize_title(raw: &str) -> Result<String, OrbitError> {
    let title = collapse_whitespace(raw);
    if title.is_empty() {
        return Err(OrbitError::InvalidInput(
            "friction `title` must not be blank".to_string(),
        ));
    }
    let length = title.chars().count();
    if length > FRICTION_TITLE_MAX_CHARS {
        return Err(OrbitError::InvalidInput(format!(
            "friction `title` must be at most {FRICTION_TITLE_MAX_CHARS} characters, got {length}; \
             it is the record's handle in lists and search — keep the full report in `body`"
        )));
    }
    Ok(title)
}

/// Derive a title from a friction body.
///
/// Returns `None` only for a body with no textual content at all.
pub fn derive_title(body: &str) -> Option<String> {
    let lines = body.lines().collect::<Vec<_>>();
    let first_index = lines.iter().position(|line| !line.trim().is_empty())?;
    let candidate = candidate_text(&lines, first_index);
    let candidate = collapse_whitespace(&strip_line_decoration(&candidate));
    if candidate.is_empty() {
        return None;
    }
    Some(clamp_title(&candidate))
}

/// The handle to show for a record: the stored title, the derived one, or —
/// for a record with neither — its id, which at least resolves.
pub fn effective_title(stored: Option<&str>, body: &str, id: &str) -> String {
    stored
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToString::to_string)
        .or_else(|| derive_title(body))
        .unwrap_or_else(|| id.to_string())
}

/// Pick the text that carries the record's subject, skipping a leading label.
fn candidate_text(lines: &[&str], first_index: usize) -> String {
    let first = lines[first_index].trim();

    if let Some(level) = heading_level(first) {
        let text = heading_text(first);
        // A heading with a peer or shallower sibling labels a section of a
        // structured report; the record's subject is the prose it introduces.
        if has_further_heading(lines, first_index, level) {
            return first_prose_paragraph(lines, first_index).unwrap_or(text);
        }
        return text;
    }

    strip_bold_lead(&paragraph_from(lines, first_index))
}

/// Join the paragraph starting at `start` into one string.
///
/// The unit is the paragraph, not the line: a hard-wrapped body states its
/// subject across a wrap, and reading a single line would cut the sentence at
/// whatever column the author's editor happened to use.
fn paragraph_from(lines: &[&str], start: usize) -> String {
    lines
        .iter()
        .skip(start)
        .map(|line| line.trim())
        .take_while(|line| !line.is_empty() && heading_level(line).is_none())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The first paragraph after `first_index` that is not itself a heading.
fn first_prose_paragraph(lines: &[&str], first_index: usize) -> Option<String> {
    let start = lines
        .iter()
        .enumerate()
        .skip(first_index + 1)
        .find(|(_, line)| {
            let line = line.trim();
            !line.is_empty() && heading_level(line).is_none()
        })
        .map(|(index, _)| index)?;
    Some(strip_bold_lead(&paragraph_from(lines, start)))
}

/// Drop a leading `**bold**` lead-in, keeping the sentence it introduces.
///
/// An inline lead-in labels the sentence beside it; a line that is nothing but
/// the bold run is itself the subject.
fn strip_bold_lead(text: &str) -> String {
    match split_bold_lead(text) {
        Some((label, rest)) if rest.is_empty() => label.to_string(),
        Some((_, rest)) => rest.to_string(),
        None => text.to_string(),
    }
}

/// The ATX heading level of `line`, if it is a heading.
fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (hashes > 0 && hashes <= 6).then_some(hashes)
}

/// A heading line's text, with its markers removed.
fn heading_text(line: &str) -> String {
    line.trim_start_matches('#')
        .trim_end_matches('#')
        .trim()
        .to_string()
}

/// Whether any later line is a heading at `level` or shallower — the signal
/// that the heading at `first_index` is one section among several.
fn has_further_heading(lines: &[&str], first_index: usize, level: usize) -> bool {
    lines
        .iter()
        .skip(first_index + 1)
        .filter_map(|line| heading_level(line.trim()))
        .any(|other| other <= level)
}

/// Split a leading `**bold**` run from the rest of its line.
fn split_bold_lead(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("**")?;
    let close = rest.find("**")?;
    let label = rest[..close].trim();
    if label.is_empty() {
        return None;
    }
    Some((label, rest[close + 2..].trim()))
}

/// Remove leading block markers (quote, list, heading) a title should not carry.
fn strip_line_decoration(line: &str) -> String {
    let mut value = line.trim();
    loop {
        let stripped = value
            .strip_prefix('>')
            .or_else(|| value.strip_prefix("- "))
            .or_else(|| value.strip_prefix("* "))
            .or_else(|| value.strip_prefix("+ "))
            .or_else(|| strip_ordered_marker(value))
            .map(str::trim_start);
        match stripped {
            Some(next) if next != value => value = next,
            _ => break,
        }
    }
    value.trim_start_matches('#').trim().to_string()
}

/// Strip an ordered-list marker (`1.` / `2)`), if the line opens with one.
fn strip_ordered_marker(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))
}

/// Collapse every whitespace run to a single space and trim the result.
fn collapse_whitespace(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bound a candidate at [`FRICTION_TITLE_MAX_CHARS`], preferring a word boundary.
fn clamp_title(candidate: &str) -> String {
    if candidate.chars().count() <= FRICTION_TITLE_MAX_CHARS {
        return candidate.to_string();
    }
    let budget = FRICTION_TITLE_MAX_CHARS - 1;
    let mut prefix = candidate.chars().take(budget).collect::<String>();
    if let Some(boundary) = prefix.rfind(char::is_whitespace)
        && prefix[..boundary].chars().count() >= MIN_WORD_BOUNDARY_PREFIX
    {
        prefix.truncate(boundary);
    }
    let mut title = prefix.trim_end().to_string();
    title.push(ELLIPSIS);
    title
}
