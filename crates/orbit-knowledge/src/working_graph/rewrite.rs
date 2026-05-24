use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::extract::{self, Language};
use crate::io::{
    LineEnding, StagedTextFile, render_content as render_text_content, write_text_atomic_durable,
};
use orbit_graph_extract::Selector;

use super::model::{WorkingGraph, WorkingLeaf, WriteError, WriteResult};

pub(super) fn find_test_module_line(lines: &[&str]) -> Option<usize> {
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == "#[cfg(test)]" {
            // Check next non-blank line is `mod tests`
            for next_line in &lines[i + 1..] {
                let next = next_line.trim();
                if next.is_empty() {
                    continue;
                }
                if next.starts_with("mod tests") {
                    return Some(i);
                }
                break;
            }
        }
    }
    None
}

pub(super) fn selector_string_for_extracted(
    rel_path: &str,
    leaf: &extract::ExtractedLeaf,
) -> String {
    format!("symbol:{rel_path}#{}:{}", leaf.qualified_name, leaf.kind)
}

pub(super) fn validate_leaf_range(
    leaf: &WorkingLeaf,
    total_lines: usize,
    selector: &str,
) -> Result<(), WriteError> {
    if leaf.start_line == 0
        || leaf.end_line == 0
        || leaf.start_line > leaf.end_line
        || leaf.end_line > total_lines
    {
        return Err(WriteError::source_conflict(
            &leaf.source_hash,
            &format!(
                "(line range {}..{} out of bounds for {}-line file)",
                leaf.start_line, leaf.end_line, total_lines
            ),
            selector,
        ));
    }
    Ok(())
}

pub(super) fn working_leaf_from_extracted(
    rel_path: &str,
    leaf: &extract::ExtractedLeaf,
) -> WorkingLeaf {
    WorkingLeaf {
        selector: selector_string_for_extracted(rel_path, leaf),
        file_path: rel_path.to_string(),
        name: leaf.name.clone(),
        qualified_name: leaf.qualified_name.clone(),
        kind: leaf.kind.clone(),
        start_line: leaf.start_line,
        end_line: leaf.end_line,
        source: leaf.source.clone(),
        source_hash: leaf.source_hash.clone(),
        parent_qualified_name: leaf.parent_qualified_name.clone(),
        children_qualified_names: leaf.children_qualified_names.clone(),
    }
}

pub(super) fn render_content(
    lines: Vec<String>,
    line_ending: LineEnding,
    preserve_trailing_newline: bool,
) -> String {
    render_text_content(lines, line_ending, preserve_trailing_newline)
}

pub(super) fn insert_source_lines(
    file_lines: &[&str],
    insert_after_line: usize,
    source: &str,
) -> Vec<String> {
    let mut new_lines: Vec<String> = file_lines[..insert_after_line]
        .iter()
        .map(|line| (*line).to_string())
        .collect();
    new_lines.push(String::new());
    new_lines.extend(source.lines().map(|line| line.to_string()));
    new_lines.extend(
        file_lines[insert_after_line..]
            .iter()
            .map(|line| (*line).to_string()),
    );
    new_lines
}

pub(super) fn inserted_line_range(insert_after_line: usize, source: &str) -> (usize, usize) {
    let start_line = insert_after_line + 2;
    let line_count = source.lines().count().max(1);
    (start_line, start_line + line_count - 1)
}

pub(super) fn remove_leaf_lines(
    file_lines: &[&str],
    start_line: usize,
    end_line: usize,
) -> Vec<String> {
    let mut new_lines: Vec<String> = file_lines[..start_line - 1]
        .iter()
        .map(|line| (*line).to_string())
        .collect();
    let after_start = end_line;
    let skip_blank = after_start < file_lines.len()
        && file_lines[after_start].trim().is_empty()
        && !new_lines.is_empty()
        && new_lines.last().is_some_and(|line| line.trim().is_empty());
    let skip_count = if skip_blank { 1 } else { 0 };
    new_lines.extend(
        file_lines[after_start + skip_count..]
            .iter()
            .map(|line| (*line).to_string()),
    );
    new_lines
}

pub(super) fn detect_line_ending(content: &str) -> LineEnding {
    LineEnding::detect(content)
}

pub(super) fn stage_text_file(path: &Path, content: &str) -> Result<StagedTextFile, WriteError> {
    StagedTextFile::new(path, content)
        .map_err(|error| WriteError::io_error(format!("stage {}: {error}", path.display())))
}

pub(super) fn write_text_file(path: &Path, content: &str) -> Result<(), WriteError> {
    write_text_atomic_durable(path, content)
        .map_err(|error| WriteError::io_error(format!("write {}: {error}", path.display())))
}

impl WorkingGraph {
    /// Rewrite an entire file and re-extract all leaves.
    pub fn rewrite_file(
        &mut self,
        rel_path: &str,
        new_content: &str,
        reason: Option<&str>,
        workspace_root: &Path,
    ) -> Result<WriteResult, WriteError> {
        let abs_path = workspace_root.join(rel_path);
        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let language = Language::from_extension(ext).ok_or_else(|| WriteError {
            kind: "unsupported_language".to_string(),
            reason: format!("no extractor for extension `.{ext}`"),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: Some(format!("file:{rel_path}")),
        })?;

        let old_content = fs::read_to_string(&abs_path).map_err(|e| WriteError {
            kind: "io_error".to_string(),
            reason: format!("read {}: {e}", abs_path.display()),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: Some(format!("file:{rel_path}")),
        })?;
        self.validate_file_snapshot(rel_path, &old_content)?;
        let old_leaves = self.snapshot_file_leaves(rel_path);
        let extraction = extract::extract_file(new_content, language);
        self.validate_unique_extracted_selectors(rel_path, &extraction)?;

        write_text_file(&abs_path, new_content)?;

        let affected = self.re_extract_file(rel_path, new_content, language);
        self.record_file_rewrite_history(rel_path, old_leaves, &extraction, reason);

        Ok(WriteResult {
            status: "ok".to_string(),
            selector: format!("file:{rel_path}"),
            edit_sequence: 0,
            new_source_hash: String::new(),
            affected_leaves: affected,
            new_leaf_id: None,
        })
    }

    /// Rewrite a line range in a file and re-extract all leaves.
    pub fn rewrite_file_region(
        &mut self,
        rel_path: &str,
        start_line: usize,
        end_line: usize,
        new_content: &str,
        reason: Option<&str>,
        workspace_root: &Path,
    ) -> Result<WriteResult, WriteError> {
        let abs_path = workspace_root.join(rel_path);
        let old_content = fs::read_to_string(&abs_path).map_err(|e| WriteError {
            kind: "io_error".to_string(),
            reason: format!("read {}: {e}", abs_path.display()),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: Some(format!("file:{rel_path}")),
        })?;

        let old_lines: Vec<&str> = old_content.lines().collect();
        if start_line == 0 || start_line > old_lines.len() || end_line < start_line {
            return Err(WriteError {
                kind: "invalid_range".to_string(),
                reason: format!(
                    "line range {start_line}..{end_line} out of bounds (file has {} lines)",
                    old_lines.len()
                ),
                expected_source_hash: None,
                actual_source: None,
                leaf_id: Some(format!("file:{rel_path}")),
            });
        }
        let end_clamped = end_line.min(old_lines.len());

        let mut new_lines: Vec<String> = old_lines[..start_line - 1]
            .iter()
            .map(|line| (*line).to_string())
            .collect();
        for line in new_content.lines() {
            new_lines.push(line.to_string());
        }
        if end_clamped < old_lines.len() {
            new_lines.extend(
                old_lines[end_clamped..]
                    .iter()
                    .map(|line| (*line).to_string()),
            );
        }

        let merged = render_content(
            new_lines,
            detect_line_ending(&old_content),
            old_content.ends_with('\n'),
        );
        self.rewrite_file(rel_path, &merged, reason, workspace_root)
    }

    pub(super) fn resolve_insert_after_line(
        &self,
        position: Option<&Selector>,
        target_file: &str,
        file_lines: &[&str],
    ) -> Result<usize, WriteError> {
        let Some(pos_selector) = position else {
            return Ok(find_test_module_line(file_lines).unwrap_or(file_lines.len()));
        };

        let pos_str = pos_selector.to_string();
        let anchor = self
            .leaves
            .get(&pos_str)
            .ok_or_else(|| WriteError::position_not_found(&pos_str))?;
        if anchor.file_path != target_file {
            return Err(WriteError::invalid_position(
                &pos_str,
                format!(
                    "belongs to `{}` rather than target file `{target_file}`",
                    anchor.file_path
                ),
            ));
        }
        if anchor.end_line == 0 || anchor.end_line > file_lines.len() {
            return Err(WriteError::invalid_position(
                &pos_str,
                format!(
                    "ends at line {} but target file `{target_file}` has {} lines",
                    anchor.end_line,
                    file_lines.len()
                ),
            ));
        }
        if anchor.start_line == 0 || anchor.start_line > anchor.end_line {
            return Err(WriteError::invalid_position(
                &pos_str,
                format!(
                    "has invalid range {}..{} for target file `{target_file}`",
                    anchor.start_line, anchor.end_line
                ),
            ));
        }

        let actual_source = file_lines[anchor.start_line - 1..anchor.end_line]
            .join("\n")
            .trim()
            .to_string();
        let actual_hash = extract::compute_source_hash(&actual_source);
        if actual_hash != anchor.source_hash {
            return Err(WriteError::source_conflict(
                &anchor.source_hash,
                &actual_source,
                &pos_str,
            ));
        }

        Ok(anchor.end_line)
    }

    pub(super) fn resolve_insert_after_line_from_extraction(
        &self,
        position: Option<&Selector>,
        target_file: &str,
        file_lines: &[&str],
        extraction: &extract::ExtractionResult,
    ) -> Result<usize, WriteError> {
        let Some(pos_selector) = position else {
            return Ok(find_test_module_line(file_lines).unwrap_or(file_lines.len()));
        };

        let Selector::Symbol { path, .. } = pos_selector else {
            let pos_str = pos_selector.to_string();
            return Err(WriteError::position_not_found(&pos_str));
        };
        if path != target_file {
            return Err(WriteError::invalid_position(
                &pos_selector.to_string(),
                format!("belongs to `{path}` rather than target file `{target_file}`"),
            ));
        }

        for leaf in &extraction.leaves {
            let selector = selector_string_for_extracted(target_file, leaf);
            if selector == pos_selector.to_string() {
                return Ok(leaf.end_line);
            }
        }

        Err(WriteError::position_not_found(&pos_selector.to_string()))
    }

    pub(super) fn resolve_canonical_new_leaf_selector(
        &self,
        rel_path: &str,
        requested_selector: &Selector,
        extraction: &extract::ExtractionResult,
        expected_source_hash: &str,
        inserted_start_line: usize,
        inserted_end_line: usize,
    ) -> Result<String, WriteError> {
        self.validate_unique_extracted_selectors(rel_path, extraction)?;

        let requested_str = requested_selector.to_string();
        let requested_symbol = match requested_selector {
            Selector::Symbol { symbol, kind, .. } => (symbol.as_str(), kind.as_str()),
            _ => {
                return Err(WriteError {
                    kind: "invalid_selector".to_string(),
                    reason: "canonical selector lookup requires a leaf selector".to_string(),
                    expected_source_hash: None,
                    actual_source: None,
                    leaf_id: None,
                });
            }
        };

        let mut exact_matches = Vec::new();
        let mut range_matches = Vec::new();

        for leaf in &extraction.leaves {
            if leaf.kind != requested_symbol.1 || leaf.source_hash != expected_source_hash {
                continue;
            }
            if leaf.start_line < inserted_start_line || leaf.end_line > inserted_end_line {
                continue;
            }

            let selector = selector_string_for_extracted(rel_path, leaf);
            if leaf.qualified_name == requested_symbol.0 {
                exact_matches.push(selector.clone());
            }
            range_matches.push(selector);
        }

        match exact_matches.len() {
            1 => return Ok(exact_matches.remove(0)),
            n if n > 1 => return Err(WriteError::ambiguous_new_leaf(&requested_str, n)),
            _ => {}
        }

        match range_matches.len() {
            1 => Ok(range_matches.remove(0)),
            0 => Err(WriteError::expected_leaf_not_found(&requested_str)),
            n => Err(WriteError::ambiguous_new_leaf(&requested_str, n)),
        }
    }

    pub(super) fn validate_unique_extracted_selectors(
        &self,
        rel_path: &str,
        extraction: &extract::ExtractionResult,
    ) -> Result<(), WriteError> {
        let mut seen: HashMap<String, ()> = HashMap::new();
        for leaf in &extraction.leaves {
            let selector = selector_string_for_extracted(rel_path, leaf);
            if seen.insert(selector.clone(), ()).is_some() {
                return Err(WriteError::duplicate_selector(&selector));
            }
        }
        Ok(())
    }
}
