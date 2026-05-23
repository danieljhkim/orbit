//! Shallow markdown extractor (T20260422-1540).
//!
//! Emits one `LeafKind::Section { depth }` per ATX heading (`#`–`######`).
//! The `qualified_name` is a kebab-slug of the heading text, disambiguated by
//! line number when the same slug appears more than once in a file. A
//! heading's `source` span covers the heading line through (exclusive of) the
//! next same-or-higher heading — matching how markdown renderers treat section
//! boundaries.
//!
//! Deliberately out of scope: frontmatter parsing, setext (underlined)
//! headings, fenced code blocks as their own leaves, links, tables.

use super::FileExtractor;
use super::common::{
    ExtractedLeaf, ExtractionResult, compute_source_hash, finalize_unique_qualified_names,
};
use super::language::{DocFormat, FileKind};

pub struct MarkdownExtractor;

impl FileExtractor for MarkdownExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Doc(DocFormat::Markdown)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let headings = collect_headings(source);
        if headings.is_empty() {
            return ExtractionResult::default();
        }

        let lines: Vec<&str> = source.lines().collect();
        let total_lines = lines.len();
        let mut leaves = Vec::with_capacity(headings.len());

        for (i, heading) in headings.iter().enumerate() {
            let end_line = headings[i + 1..]
                .iter()
                .find(|next| next.depth <= heading.depth)
                .map(|next| next.line.saturating_sub(1))
                .unwrap_or(total_lines);
            let body = slice_lines(&lines, heading.line, end_line);
            let source_hash = compute_source_hash(&body);

            leaves.push(ExtractedLeaf {
                qualified_name: heading.qualified_name.clone(),
                name: heading.text.clone(),
                kind: "section".to_string(),
                start_line: heading.line,
                end_line,
                source: body,
                source_hash,
                parent_qualified_name: None,
                children_qualified_names: Vec::new(),
                depth: Some(heading.depth),
            });
        }

        finalize_unique_qualified_names(&mut leaves);
        ExtractionResult {
            leaves,
            ..Default::default()
        }
    }
}

struct Heading {
    depth: u8,
    text: String,
    qualified_name: String,
    line: usize,
}

fn collect_headings(source: &str) -> Vec<Heading> {
    let mut seen_slugs: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut headings = Vec::new();
    let mut in_fence = false;
    let mut fence_marker: Option<&str> = None;

    for (idx, raw_line) in source.lines().enumerate() {
        let line_num = idx + 1;
        let trimmed = raw_line.trim_start();

        // Track fenced code blocks; headings inside fences are not headings.
        if let Some(marker) = fence_marker {
            if trimmed.starts_with(marker) {
                in_fence = false;
                fence_marker = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            in_fence = true;
            fence_marker = Some("```");
            continue;
        }
        if trimmed.starts_with("~~~") {
            in_fence = true;
            fence_marker = Some("~~~");
            continue;
        }
        if in_fence {
            continue;
        }

        let Some(parsed) = parse_atx_heading(trimmed) else {
            continue;
        };
        let (depth, text) = parsed;
        let base_slug = slugify(&text);
        let suffix = seen_slugs.entry(base_slug.clone()).or_insert(0);
        let qualified_name = if *suffix == 0 {
            base_slug.clone()
        } else {
            format!("{base_slug}-{}", line_num)
        };
        *suffix += 1;

        headings.push(Heading {
            depth,
            text,
            qualified_name,
            line: line_num,
        });
    }

    headings
}

fn parse_atx_heading(trimmed: &str) -> Option<(u8, String)> {
    let bytes = trimmed.as_bytes();
    let mut depth: u8 = 0;
    while depth < 6 && bytes.get(depth as usize).copied() == Some(b'#') {
        depth += 1;
    }
    if depth == 0 {
        return None;
    }
    // Allow one more `#` to drop through only if it's followed by a non-#
    // (i.e. `####### foo` is NOT a heading — spec requires ≤ 6 leading `#`).
    if bytes.get(depth as usize).copied() == Some(b'#') {
        return None;
    }
    // Require at least one space separating `#`s from the text.
    let rest = &trimmed[depth as usize..];
    if !rest.starts_with(' ') && !rest.starts_with('\t') && !rest.is_empty() {
        return None;
    }
    let text = rest.trim_start().trim_end_matches('#').trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some((depth, text))
}

fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_hyphen = true;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            for low in ch.to_lowercase() {
                out.push(low);
            }
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            out.push('-');
            last_was_hyphen = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("section");
    }
    out
}

/// Slice source lines [start_line, end_line] inclusive (1-based) and join
/// with `\n`. `end_line == 0` yields an empty string.
fn slice_lines(lines: &[&str], start_line: usize, end_line: usize) -> String {
    if start_line == 0 || end_line < start_line {
        return String::new();
    }
    let lo = start_line - 1;
    let hi = end_line.min(lines.len());
    lines[lo..hi].join("\n")
}
