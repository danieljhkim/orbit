//! Markdown extractor (line-based port + extensions for ORB-00305).
//!
//! Headings -> symbols (kind "heading", byte spans computed from lines).
//! Notable strings: link URLs/text, code fence contents (filtered per spec §6.2).
//! No tree-sitter dep added (pure Rust, reuses old logic); satisfies tree-sitter-only for code paths.

use std::path::Path;

use super::common::{dedup_symbols, is_notable_string, normalize_path};
use crate::{ExtractedFile, Extractor, RawString, RawSymbol};

/// Markdown heading/string extractor.
pub struct MarkdownExtractor;

impl Extractor for MarkdownExtractor {
    fn lang(&self) -> &'static str {
        "markdown"
    }

    fn supports(&self, path: &Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md") | Some("markdown")
        )
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let headings = collect_headings(source);
        let lines: Vec<&str> = source.lines().collect();
        let total_lines = lines.len();

        let mut symbols = Vec::new();
        let mut strings = Vec::new();

        for (i, heading) in headings.iter().enumerate() {
            let end_line = headings
                .get(i + 1)
                .map(|next| {
                    if next.depth <= heading.depth {
                        next.line.saturating_sub(1)
                    } else {
                        total_lines
                    }
                })
                .unwrap_or(total_lines);
            let _body = slice_lines(&lines, heading.line, end_line);
            let (start_byte, end_byte) = line_range_to_bytes(source, heading.line, end_line);

            symbols.push(RawSymbol {
                file_path: normalize_path(path),
                name: heading.text.clone(),
                qualified: heading.qualified_name.clone(),
                kind: "heading".to_string(),
                span_start: start_byte,
                span_end: end_byte,
                signature: None,
                parent_symbol: None,
            });
        }

        // Collect notable strings: links and code blocks
        collect_notable_strings(source, &mut strings, path);

        dedup_symbols(&mut symbols);
        ExtractedFile {
            symbols,
            strings,
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

        let Some((depth, text)) = parse_atx_heading(trimmed) else {
            continue;
        };
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
    if bytes.get(depth as usize).copied() == Some(b'#') {
        return None;
    }
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

fn slice_lines(lines: &[&str], start_line: usize, end_line: usize) -> String {
    if start_line == 0 || end_line < start_line {
        return String::new();
    }
    let lo = start_line - 1;
    let hi = end_line.min(lines.len());
    lines[lo..hi].join("\n")
}

fn line_range_to_bytes(source: &str, start_line: usize, end_line: usize) -> (usize, usize) {
    let mut cur_line = 1;
    let mut start_byte = 0;
    let mut end_byte = source.len();
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            cur_line += 1;
            if cur_line == start_line {
                start_byte = i + 1;
            }
            if cur_line == end_line + 1 {
                end_byte = i;
                break;
            }
        }
    }
    if start_line == 1 {
        start_byte = 0;
    }
    (start_byte, end_byte.min(source.len()))
}

fn collect_notable_strings(source: &str, out: &mut Vec<RawString>, path: &Path) {
    let file_path = normalize_path(path);
    let lines: Vec<&str> = source.lines().collect();

    // Simple link extraction: [text](url) or [text][ref]
    for (i, line) in lines.iter().enumerate() {
        let mut pos = 0;
        while let Some(start) = line[pos..].find('[') {
            let abs = pos + start;
            if let Some(end_text) = line[abs..].find(']') {
                let text_part = &line[abs + 1..abs + end_text];
                if let Some(url_start) = line[abs + end_text..].find('(') {
                    let value_start = abs + end_text + url_start + 1;
                    let Some(url_end_rel) = line[value_start..].find(')') else {
                        pos = abs + 1;
                        continue;
                    };
                    let url = line[value_start..value_start + url_end_rel].trim();
                    if !url.is_empty() {
                        let val = format!("{} {}", text_part.trim(), url);
                        if is_notable_string(&val) {
                            out.push(RawString {
                                file_path: file_path.clone(),
                                line: i + 1,
                                value: val,
                                context_symbol: None,
                            });
                        }
                    }
                }
            }
            pos = abs + 1;
            if pos >= line.len() {
                break;
            }
        }
    }

    // Code fences: collect content between ```...```
    let mut in_fence = false;
    let mut fence_start_line = 0;
    let mut fence_lines: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            if in_fence {
                let content = fence_lines.join("\n");
                if is_notable_string(&content) {
                    out.push(RawString {
                        file_path: file_path.clone(),
                        line: fence_start_line,
                        value: content,
                        context_symbol: None,
                    });
                }
                fence_lines.clear();
                in_fence = false;
            } else {
                in_fence = true;
                fence_start_line = i + 1;
            }
            continue;
        }
        if in_fence {
            fence_lines.push(line.to_string());
        }
    }
}
