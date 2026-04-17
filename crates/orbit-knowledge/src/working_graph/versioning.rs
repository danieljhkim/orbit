use std::collections::HashMap;
use std::fs;
use std::path::Path;

use chrono::Utc;

use crate::extract::{self, Language};

use super::model::{LeafEdit, LeafVersionChain, WorkingGraph, WorkingLeaf, WriteError};
use super::rewrite::{selector_string_for_extracted, working_leaf_from_extracted};

impl WorkingGraph {
    /// Seed full-file snapshots from the current workspace contents.
    pub fn seed_file_snapshots_from_workspace(&mut self, workspace_root: &Path) {
        let rel_paths: Vec<String> = self.file_leaves.keys().cloned().collect();
        for rel_path in rel_paths {
            let abs_path = workspace_root.join(&rel_path);
            if let Ok(content) = fs::read_to_string(&abs_path) {
                self.remember_file_snapshot(&rel_path, &content);
            }
        }
    }

    pub(super) fn snapshot_file_leaves(&self, rel_path: &str) -> HashMap<String, WorkingLeaf> {
        self.file_leaves
            .get(rel_path)
            .into_iter()
            .flat_map(|selectors| selectors.iter())
            .filter_map(|selector| {
                self.leaves
                    .get(selector)
                    .cloned()
                    .map(|leaf| (selector.clone(), leaf))
            })
            .collect()
    }

    pub(super) fn record_file_rewrite_history(
        &mut self,
        rel_path: &str,
        old_leaves: HashMap<String, WorkingLeaf>,
        extraction: &extract::ExtractionResult,
        reason: Option<&str>,
    ) {
        let mut new_leaves: HashMap<String, WorkingLeaf> = HashMap::new();
        for leaf in &extraction.leaves {
            let selector = selector_string_for_extracted(rel_path, leaf);
            new_leaves.insert(
                selector.clone(),
                working_leaf_from_extracted(rel_path, leaf),
            );
        }

        for (selector, new_leaf) in &new_leaves {
            match old_leaves.get(selector) {
                Some(old_leaf)
                    if old_leaf.start_line == new_leaf.start_line
                        && old_leaf.end_line == new_leaf.end_line
                        && old_leaf.source_hash == new_leaf.source_hash => {}
                Some(old_leaf) => {
                    let rewrite_reason = reason
                        .map(|r| format!("{r} (file rewrite)"))
                        .unwrap_or_else(|| "file rewrite".to_string());
                    self.append_version_chain(
                        selector,
                        &old_leaf.source_hash,
                        &new_leaf.source,
                        &new_leaf.source_hash,
                        Some(rewrite_reason.as_str()),
                    );
                }
                None => {
                    let create_reason = reason
                        .map(|r| format!("{r} (created via file rewrite)"))
                        .unwrap_or_else(|| "created via file rewrite".to_string());
                    self.record_created_leaf(
                        selector,
                        &new_leaf.source,
                        &new_leaf.source_hash,
                        Some(create_reason.as_str()),
                    );
                }
            }
        }

        for (selector, old_leaf) in old_leaves {
            if new_leaves.contains_key(&selector) {
                continue;
            }
            let delete_reason = reason
                .map(|r| format!("{r} (deleted via file rewrite)"))
                .unwrap_or_else(|| "deleted via file rewrite".to_string());
            self.append_version_chain(
                &selector,
                &old_leaf.source_hash,
                "",
                &extract::compute_source_hash(""),
                Some(delete_reason.as_str()),
            );
        }
    }

    /// Re-run the extractor on a modified file and update the working graph.
    ///
    /// Returns the list of leaf selectors whose positions changed.
    pub(super) fn re_extract_file(
        &mut self,
        rel_path: &str,
        content: &str,
        language: Language,
    ) -> Vec<String> {
        let result = extract::extract_file(content, language);

        // Collect old selectors for this file
        let old_selectors: Vec<String> =
            self.file_leaves.get(rel_path).cloned().unwrap_or_default();

        // Build new leaf set
        let mut new_selectors = Vec::new();
        let mut affected = Vec::new();

        for extracted in &result.leaves {
            let selector_str = selector_string_for_extracted(rel_path, extracted);
            new_selectors.push(selector_str.clone());

            // Check if position changed compared to old
            if let Some(old_leaf) = self.leaves.get(&selector_str) {
                if old_leaf.start_line != extracted.start_line
                    || old_leaf.end_line != extracted.end_line
                    || old_leaf.source_hash != extracted.source_hash
                {
                    affected.push(selector_str.clone());
                }
            } else {
                affected.push(selector_str.clone());
            }

            let leaf = working_leaf_from_extracted(rel_path, extracted);
            self.leaves.insert(selector_str, leaf);
        }

        // Remove leaves that no longer exist in the file
        for old_sel in &old_selectors {
            if !new_selectors.contains(old_sel) {
                self.leaves.remove(old_sel);
            }
        }

        // Update file_leaves index
        self.file_leaves.insert(rel_path.to_string(), new_selectors);
        self.remember_file_snapshot(rel_path, content);

        affected
    }

    pub(super) fn remember_file_snapshot(&mut self, rel_path: &str, content: &str) {
        self.file_snapshots
            .insert(rel_path.to_string(), extract::compute_source_hash(content));
    }

    pub(super) fn ensure_file_snapshot(&mut self, rel_path: &str, content: &str) {
        self.file_snapshots
            .entry(rel_path.to_string())
            .or_insert_with(|| extract::compute_source_hash(content));
    }

    pub(super) fn validate_file_snapshot(
        &mut self,
        rel_path: &str,
        content: &str,
    ) -> Result<(), WriteError> {
        let selector = format!("file:{rel_path}");
        match self.file_snapshots.get(rel_path) {
            Some(expected_hash) => {
                let actual_hash = extract::compute_source_hash(content);
                if actual_hash != *expected_hash {
                    return Err(WriteError::source_conflict(
                        expected_hash,
                        content,
                        &selector,
                    ));
                }
            }
            None => self.remember_file_snapshot(rel_path, content),
        }
        Ok(())
    }

    pub(super) fn record_created_leaf(
        &mut self,
        selector: &str,
        source: &str,
        source_hash: &str,
        reason: Option<&str>,
    ) -> u32 {
        if self.version_chains.contains_key(selector) {
            return self.append_version_chain(selector, source_hash, source, source_hash, reason);
        }

        self.version_chains.insert(
            selector.to_string(),
            LeafVersionChain {
                leaf_id: selector.to_string(),
                selector: selector.to_string(),
                original_source_hash: source_hash.to_string(),
                edits: vec![LeafEdit {
                    edit_sequence: 0,
                    source_hash: source_hash.to_string(),
                    source: source.to_string(),
                    timestamp: Utc::now().to_rfc3339(),
                    reason: reason.map(|r| r.to_string()),
                }],
            },
        );
        0
    }

    pub(super) fn transfer_version_chain(
        &mut self,
        from_selector: &str,
        to_selector: &str,
        original_hash: &str,
    ) {
        if from_selector == to_selector {
            self.version_chains
                .entry(to_selector.to_string())
                .or_insert_with(|| LeafVersionChain {
                    leaf_id: to_selector.to_string(),
                    selector: to_selector.to_string(),
                    original_source_hash: original_hash.to_string(),
                    edits: Vec::new(),
                });
            return;
        }

        let mut chain = self
            .version_chains
            .remove(from_selector)
            .unwrap_or_else(|| LeafVersionChain {
                leaf_id: to_selector.to_string(),
                selector: to_selector.to_string(),
                original_source_hash: original_hash.to_string(),
                edits: Vec::new(),
            });
        chain.leaf_id = to_selector.to_string();
        chain.selector = to_selector.to_string();

        if let Some(mut existing_target_chain) = self.version_chains.remove(to_selector) {
            let mut next_seq = existing_target_chain
                .edits
                .last()
                .map(|edit| edit.edit_sequence + 1)
                .unwrap_or(1);
            for edit in chain.edits {
                let mut merged = edit;
                merged.edit_sequence = next_seq;
                next_seq += 1;
                existing_target_chain.edits.push(merged);
            }
            chain = existing_target_chain;
            chain.leaf_id = to_selector.to_string();
            chain.selector = to_selector.to_string();
        }

        self.version_chains.insert(to_selector.to_string(), chain);
    }

    /// Append to the version chain for a leaf, returning the edit_sequence number.
    pub(super) fn append_version_chain(
        &mut self,
        selector: &str,
        original_hash: &str,
        new_source: &str,
        new_hash: &str,
        reason: Option<&str>,
    ) -> u32 {
        let chain = self
            .version_chains
            .entry(selector.to_string())
            .or_insert_with(|| LeafVersionChain {
                leaf_id: selector.to_string(),
                selector: selector.to_string(),
                original_source_hash: original_hash.to_string(),
                edits: Vec::new(),
            });

        let seq = if chain.edits.is_empty() {
            1 // 0 is reserved for original
        } else {
            chain.edits.last().unwrap().edit_sequence + 1
        };

        chain.edits.push(LeafEdit {
            edit_sequence: seq,
            source_hash: new_hash.to_string(),
            source: new_source.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            reason: reason.map(|s| s.to_string()),
        });

        seq
    }
}
