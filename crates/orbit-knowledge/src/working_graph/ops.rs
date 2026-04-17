use std::fs;
use std::path::Path;

use crate::extract::{self, Language};
use crate::selector::Selector;

use super::model::{MoveResult, WorkingGraph, WriteError, WriteResult};
use super::rewrite::{
    insert_source_lines, inserted_line_range, remove_leaf_lines, render_content,
    validate_leaf_range,
};

impl WorkingGraph {
    // -----------------------------------------------------------------
    // Locking
    // -----------------------------------------------------------------

    /// Lock a node for exclusive editing.
    ///
    /// Returns `Err` if already locked by a different owner.
    /// Execute an edit on an existing leaf.
    ///
    /// Locking is handled externally via the shared `LockStore` —
    /// callers must acquire the lock before calling this method.
    pub fn edit_leaf(
        &mut self,
        selector: &Selector,
        new_source: &str,
        reason: Option<&str>,
        workspace_root: &Path,
    ) -> Result<WriteResult, WriteError> {
        let selector_str = selector.to_string();
        let leaf = self
            .leaves
            .get(&selector_str)
            .ok_or_else(|| WriteError {
                kind: "leaf_not_found".to_string(),
                reason: format!("selector `{selector_str}` does not resolve to a leaf"),
                expected_source_hash: None,
                actual_source: None,
                leaf_id: None,
            })?
            .clone();

        let file_path = workspace_root.join(&leaf.file_path);
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let language =
            Language::from_extension(ext).ok_or_else(|| WriteError::unsupported_language(ext))?;

        // Read file from disk
        let file_content = fs::read_to_string(&file_path)
            .map_err(|e| WriteError::io_error(format!("read {}: {e}", file_path.display())))?;
        self.ensure_file_snapshot(&leaf.file_path, &file_content);
        let file_lines: Vec<&str> = file_content.lines().collect();

        // Validate source hash at start_line..end_line
        validate_leaf_range(&leaf, file_lines.len(), &selector_str)?;

        let actual_source = file_lines[leaf.start_line - 1..leaf.end_line]
            .join("\n")
            .trim()
            .to_string();
        let actual_hash = extract::compute_source_hash(&actual_source);

        if actual_hash != leaf.source_hash {
            return Err(WriteError::source_conflict(
                &leaf.source_hash,
                &actual_source,
                &selector_str,
            ));
        }

        // Replace lines start_line..end_line with new_source
        let mut new_lines: Vec<String> = Vec::new();
        for line in &file_lines[..leaf.start_line - 1] {
            new_lines.push(line.to_string());
        }
        for line in new_source.lines() {
            new_lines.push(line.to_string());
        }
        for line in &file_lines[leaf.end_line..] {
            new_lines.push(line.to_string());
        }

        let new_content = new_lines.join("\n");
        // Preserve trailing newline if original had one
        let new_content = if file_content.ends_with('\n') && !new_content.ends_with('\n') {
            new_content + "\n"
        } else {
            new_content
        };

        // Write file back to disk
        fs::write(&file_path, &new_content)
            .map_err(|e| WriteError::io_error(format!("write {}: {e}", file_path.display())))?;

        // Re-extract and update working graph
        let affected = self.re_extract_file(&leaf.file_path, &new_content, language);

        // Append to version chain
        let new_hash = extract::compute_source_hash(new_source.trim());
        let edit_seq = self.append_version_chain(
            &selector_str,
            &leaf.source_hash,
            new_source.trim(),
            &new_hash,
            reason,
        );

        Ok(WriteResult {
            status: "ok".to_string(),
            selector: selector_str,
            edit_sequence: edit_seq,
            new_source_hash: new_hash,
            affected_leaves: affected,
            new_leaf_id: None,
        })
    }

    /// Insert a new leaf into a file.
    ///
    /// When position is provided, inserts after the anchor leaf's end_line.
    /// When position is None, appends before #[cfg(test)] mod tests or at EOF.
    /// Insert a new leaf into a file.
    ///
    /// Locking is handled externally via the shared `LockStore`.
    pub fn insert_leaf(
        &mut self,
        selector: &Selector,
        new_source: &str,
        position: Option<&Selector>,
        reason: Option<&str>,
        workspace_root: &Path,
    ) -> Result<WriteResult, WriteError> {
        let file_path = match selector {
            Selector::Symbol { path, .. } => path.clone(),
            _ => {
                return Err(WriteError {
                    kind: "invalid_selector".to_string(),
                    reason: "insert mode requires a leaf selector".to_string(),
                    expected_source_hash: None,
                    actual_source: None,
                    leaf_id: None,
                });
            }
        };

        let abs_path = workspace_root.join(&file_path);
        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let language =
            Language::from_extension(ext).ok_or_else(|| WriteError::unsupported_language(ext))?;

        let file_content = fs::read_to_string(&abs_path)
            .map_err(|e| WriteError::io_error(format!("read {}: {e}", abs_path.display())))?;
        self.ensure_file_snapshot(&file_path, &file_content);
        let file_lines: Vec<&str> = file_content.lines().collect();

        let insert_after_line =
            self.resolve_insert_after_line(position, &file_path, &file_lines)?;
        let new_content = render_content(
            insert_source_lines(&file_lines, insert_after_line, new_source),
            file_content.ends_with('\n'),
        );
        let extraction = extract::extract_file(&new_content, language);
        self.validate_unique_extracted_selectors(&file_path, &extraction)?;

        let new_hash = extract::compute_source_hash(new_source.trim());
        let (inserted_start_line, inserted_end_line) =
            inserted_line_range(insert_after_line, new_source);
        let canonical_selector = self.resolve_canonical_new_leaf_selector(
            &file_path,
            selector,
            &extraction,
            &new_hash,
            inserted_start_line,
            inserted_end_line,
        )?;

        // Write file
        fs::write(&abs_path, &new_content)
            .map_err(|e| WriteError::io_error(format!("write {}: {e}", abs_path.display())))?;

        // Re-extract
        let affected = self.re_extract_file(&file_path, &new_content, language);

        let edit_seq = self.record_created_leaf(
            &canonical_selector,
            new_source.trim(),
            &new_hash,
            Some(reason.unwrap_or("created")),
        );

        Ok(WriteResult {
            status: "created".to_string(),
            selector: canonical_selector.clone(),
            edit_sequence: edit_seq,
            new_source_hash: new_hash,
            affected_leaves: affected,
            new_leaf_id: Some(canonical_selector),
        })
    }

    /// Delete a leaf from a file.
    ///
    /// Removes the leaf's source lines from the file on disk, re-extracts,
    /// and records a deletion marker in the version chain.
    /// Locking is handled externally via the shared `LockStore`.
    pub fn delete_leaf(
        &mut self,
        selector: &Selector,
        reason: Option<&str>,
        workspace_root: &Path,
    ) -> Result<WriteResult, WriteError> {
        let selector_str = selector.to_string();
        let leaf = self
            .leaves
            .get(&selector_str)
            .ok_or_else(|| WriteError {
                kind: "leaf_not_found".to_string(),
                reason: format!("selector `{selector_str}` does not resolve to a leaf"),
                expected_source_hash: None,
                actual_source: None,
                leaf_id: None,
            })?
            .clone();

        let file_path = workspace_root.join(&leaf.file_path);
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let language =
            Language::from_extension(ext).ok_or_else(|| WriteError::unsupported_language(ext))?;

        let file_content = fs::read_to_string(&file_path)
            .map_err(|e| WriteError::io_error(format!("read {}: {e}", file_path.display())))?;
        self.ensure_file_snapshot(&leaf.file_path, &file_content);
        let file_lines: Vec<&str> = file_content.lines().collect();

        // Validate source hash
        validate_leaf_range(&leaf, file_lines.len(), &selector_str)?;

        let actual_source = file_lines[leaf.start_line - 1..leaf.end_line]
            .join("\n")
            .trim()
            .to_string();
        let actual_hash = extract::compute_source_hash(&actual_source);

        if actual_hash != leaf.source_hash {
            return Err(WriteError::source_conflict(
                &leaf.source_hash,
                &actual_source,
                &selector_str,
            ));
        }

        // Remove lines and collapse any resulting double blank lines
        let mut new_lines: Vec<String> = Vec::new();
        for line in &file_lines[..leaf.start_line - 1] {
            new_lines.push(line.to_string());
        }
        // Skip a trailing blank line to avoid double gaps
        let after_start = leaf.end_line;
        let skip_blank = after_start < file_lines.len()
            && file_lines[after_start].trim().is_empty()
            && !new_lines.is_empty()
            && new_lines.last().is_some_and(|l| l.trim().is_empty());
        let skip_count = if skip_blank { 1 } else { 0 };
        for line in &file_lines[after_start + skip_count..] {
            new_lines.push(line.to_string());
        }

        let new_content = new_lines.join("\n");
        let new_content = if file_content.ends_with('\n') && !new_content.ends_with('\n') {
            new_content + "\n"
        } else {
            new_content
        };

        fs::write(&file_path, &new_content)
            .map_err(|e| WriteError::io_error(format!("write {}: {e}", file_path.display())))?;

        let affected = self.re_extract_file(&leaf.file_path, &new_content, language);

        // Record deletion in version chain
        let edit_seq = self.append_version_chain(
            &selector_str,
            &leaf.source_hash,
            "",
            &extract::compute_source_hash(""),
            Some(reason.unwrap_or("deleted")),
        );

        // Remove from leaves and file_leaves (re_extract_file already handles this
        // because the deleted symbol no longer appears in extraction results)

        Ok(WriteResult {
            status: "deleted".to_string(),
            selector: selector_str,
            edit_sequence: edit_seq,
            new_source_hash: String::new(),
            affected_leaves: affected,
            new_leaf_id: None,
        })
    }

    /// Move a leaf from its current file to a target file.
    ///
    /// Removes the leaf's source from the source file, inserts it into
    /// the target file at the given position, and re-extracts both files.
    /// Locking is handled externally via the shared `LockStore`.
    pub fn move_leaf(
        &mut self,
        selector: &Selector,
        target_file: &str,
        position: Option<&Selector>,
        reason: Option<&str>,
        workspace_root: &Path,
    ) -> Result<MoveResult, WriteError> {
        let selector_str = selector.to_string();
        let leaf = self
            .leaves
            .get(&selector_str)
            .ok_or_else(|| WriteError {
                kind: "leaf_not_found".to_string(),
                reason: format!("selector `{selector_str}` does not resolve to a leaf"),
                expected_source_hash: None,
                actual_source: None,
                leaf_id: None,
            })?
            .clone();

        let source_text = leaf.source.clone();
        let old_file_path = leaf.file_path.clone();

        let src_abs = workspace_root.join(&old_file_path);
        let src_ext = src_abs.extension().and_then(|e| e.to_str()).unwrap_or("");
        let src_language = Language::from_extension(src_ext)
            .ok_or_else(|| WriteError::unsupported_language(src_ext))?;

        let src_content = fs::read_to_string(&src_abs)
            .map_err(|e| WriteError::io_error(format!("read {}: {e}", src_abs.display())))?;
        self.ensure_file_snapshot(&old_file_path, &src_content);
        let src_lines: Vec<&str> = src_content.lines().collect();

        validate_leaf_range(&leaf, src_lines.len(), &selector_str)?;

        let actual_source = src_lines[leaf.start_line - 1..leaf.end_line]
            .join("\n")
            .trim()
            .to_string();
        let actual_hash = extract::compute_source_hash(&actual_source);
        if actual_hash != leaf.source_hash {
            return Err(WriteError::source_conflict(
                &leaf.source_hash,
                &actual_source,
                &selector_str,
            ));
        }

        let new_src_content = render_content(
            remove_leaf_lines(&src_lines, leaf.start_line, leaf.end_line),
            src_content.ends_with('\n'),
        );

        if target_file == old_file_path {
            let planned_lines: Vec<&str> = new_src_content.lines().collect();
            let planned_extraction = extract::extract_file(&new_src_content, src_language);
            let insert_after_line = self.resolve_insert_after_line_from_extraction(
                position,
                target_file,
                &planned_lines,
                &planned_extraction,
            )?;
            let final_content = render_content(
                insert_source_lines(&planned_lines, insert_after_line, &source_text),
                src_content.ends_with('\n'),
            );
            let final_extraction = extract::extract_file(&final_content, src_language);
            self.validate_unique_extracted_selectors(target_file, &final_extraction)?;
            let (inserted_start_line, inserted_end_line) =
                inserted_line_range(insert_after_line, &source_text);
            let canonical_selector = self.resolve_canonical_new_leaf_selector(
                target_file,
                selector,
                &final_extraction,
                &leaf.source_hash,
                inserted_start_line,
                inserted_end_line,
            )?;

            fs::write(&src_abs, &final_content)
                .map_err(|e| WriteError::io_error(format!("write {}: {e}", src_abs.display())))?;

            let affected = self.re_extract_file(&old_file_path, &final_content, src_language);
            self.transfer_version_chain(&selector_str, &canonical_selector, &leaf.source_hash);

            let move_reason = reason
                .map(|r| format!("{r} (moved from {selector_str} to {canonical_selector})"))
                .unwrap_or_else(|| format!("moved from {selector_str} to {canonical_selector}"));
            self.append_version_chain(
                &canonical_selector,
                &leaf.source_hash,
                source_text.trim(),
                &leaf.source_hash,
                Some(move_reason.as_str()),
            );

            let mut all_affected = affected;
            if !all_affected.contains(&selector_str) {
                all_affected.push(selector_str.clone());
            }
            if !all_affected.contains(&canonical_selector) {
                all_affected.push(canonical_selector.clone());
            }

            return Ok(MoveResult {
                status: "moved".to_string(),
                old_selector: selector_str,
                new_selector: canonical_selector,
                affected_leaves: all_affected,
            });
        }

        let tgt_abs = workspace_root.join(target_file);
        let tgt_ext = tgt_abs.extension().and_then(|e| e.to_str()).unwrap_or("");
        let tgt_language = Language::from_extension(tgt_ext)
            .ok_or_else(|| WriteError::unsupported_language(tgt_ext))?;

        let tgt_content = fs::read_to_string(&tgt_abs)
            .map_err(|e| WriteError::io_error(format!("read {}: {e}", tgt_abs.display())))?;
        self.ensure_file_snapshot(target_file, &tgt_content);
        let tgt_lines: Vec<&str> = tgt_content.lines().collect();
        let insert_after_line =
            self.resolve_insert_after_line(position, target_file, &tgt_lines)?;
        let new_tgt_content = render_content(
            insert_source_lines(&tgt_lines, insert_after_line, &source_text),
            tgt_content.ends_with('\n'),
        );
        let tgt_extraction = extract::extract_file(&new_tgt_content, tgt_language);
        self.validate_unique_extracted_selectors(target_file, &tgt_extraction)?;
        let (inserted_start_line, inserted_end_line) =
            inserted_line_range(insert_after_line, &source_text);
        let canonical_selector = self.resolve_canonical_new_leaf_selector(
            target_file,
            selector,
            &tgt_extraction,
            &leaf.source_hash,
            inserted_start_line,
            inserted_end_line,
        )?;

        fs::write(&tgt_abs, &new_tgt_content)
            .map_err(|e| WriteError::io_error(format!("write {}: {e}", tgt_abs.display())))?;
        if let Err(source_write_err) = fs::write(&src_abs, &new_src_content) {
            let rollback = fs::write(&tgt_abs, &tgt_content);
            return Err(match rollback {
                Ok(()) => {
                    WriteError::io_error(format!("write {}: {source_write_err}", src_abs.display()))
                }
                Err(rollback_err) => WriteError::io_error(format!(
                    "write {}: {source_write_err}; rollback {}: {rollback_err}",
                    src_abs.display(),
                    tgt_abs.display()
                )),
            });
        }

        let source_affected = self.re_extract_file(&old_file_path, &new_src_content, src_language);
        let target_affected = self.re_extract_file(target_file, &new_tgt_content, tgt_language);

        self.transfer_version_chain(&selector_str, &canonical_selector, &leaf.source_hash);

        let move_reason = reason
            .map(|r| format!("{r} (moved from {selector_str} to {canonical_selector})"))
            .unwrap_or_else(|| format!("moved from {selector_str} to {canonical_selector}"));
        self.append_version_chain(
            &canonical_selector,
            &leaf.source_hash,
            source_text.trim(),
            &leaf.source_hash,
            Some(move_reason.as_str()),
        );

        let mut all_affected = target_affected;
        for selector in source_affected {
            if !all_affected.contains(&selector) {
                all_affected.push(selector);
            }
        }
        if !all_affected.contains(&selector_str) {
            all_affected.push(selector_str.clone());
        }
        if !all_affected.contains(&canonical_selector) {
            all_affected.push(canonical_selector.clone());
        }

        Ok(MoveResult {
            status: "moved".to_string(),
            old_selector: selector_str,
            new_selector: canonical_selector,
            affected_leaves: all_affected,
        })
    }
}
