//! In-memory working graph that diverges from the persisted `.orbit/knowledge/`
//! as edits accumulate during an activity run.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::extractor::{self, Language};
use super::{KnowledgeError, KnowledgeStore, Selector};

/// A single edit in a leaf's version chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeafEdit {
    pub edit_sequence: u32,
    pub source_hash: String,
    pub source: String,
    pub timestamp: String,
    pub reason: Option<String>,
}

/// Full version history for a leaf across an activity run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeafVersionChain {
    pub leaf_id: String,
    pub selector: String,
    pub original_source_hash: String,
    pub edits: Vec<LeafEdit>,
}

/// In-memory leaf state tracked by the working graph.
#[derive(Debug, Clone)]
pub struct WorkingLeaf {
    pub selector: String,
    pub file_path: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub source: String,
    pub source_hash: String,
    pub parent_qualified_name: Option<String>,
    pub children_qualified_names: Vec<String>,
}

/// Result of a successful knowledge.write edit operation.
#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub status: String,
    pub selector: String,
    pub edit_sequence: u32,
    pub new_source_hash: String,
    pub affected_leaves: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_leaf_id: Option<String>,
}

/// Error from a knowledge.write operation.
#[derive(Debug, Clone, Serialize)]
pub struct WriteError {
    pub kind: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_source_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leaf_id: Option<String>,
}

impl WriteError {
    pub fn source_conflict(expected_hash: &str, actual_source: &str, leaf_selector: &str) -> Self {
        Self {
            kind: "source_conflict".to_string(),
            reason: format!("file on disk does not match working graph for `{leaf_selector}`"),
            expected_source_hash: Some(expected_hash.to_string()),
            actual_source: Some(actual_source.to_string()),
            leaf_id: Some(leaf_selector.to_string()),
        }
    }

    pub fn position_not_found(selector: &str) -> Self {
        Self {
            kind: "position_not_found".to_string(),
            reason: format!("position reference selector `{selector}` does not resolve"),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: None,
        }
    }

    pub fn unsupported_language(ext: &str) -> Self {
        Self {
            kind: "unsupported_language".to_string(),
            reason: format!("no extractor available for `.{ext}` files"),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: None,
        }
    }

    pub fn io_error(reason: impl Into<String>) -> Self {
        Self {
            kind: "io_error".to_string(),
            reason: reason.into(),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: None,
        }
    }
}

/// In-memory working graph that tracks leaf state during an activity run.
///
/// Initialized from the persisted knowledge store, then mutated in memory
/// as `knowledge.write` calls modify files and re-extract.
#[derive(Debug, Clone, Default)]
pub struct WorkingGraph {
    /// Leaves indexed by selector string (e.g. "leaf:path#symbol:kind").
    leaves: HashMap<String, WorkingLeaf>,
    /// Reverse index: file_path → list of leaf selector strings.
    file_leaves: HashMap<String, Vec<String>>,
    /// Version chains indexed by leaf selector string.
    version_chains: HashMap<String, LeafVersionChain>,
}

impl WorkingGraph {
    /// Create a new empty working graph.
    pub fn new() -> Self {
        Self {
            leaves: HashMap::new(),
            file_leaves: HashMap::new(),
            version_chains: HashMap::new(),
        }
    }

    /// Initialize a working graph from a persisted KnowledgeStore.
    ///
    /// Loads all leaf objects from the graph to populate initial state.
    pub fn from_store(store: &KnowledgeStore) -> Result<Self, KnowledgeError> {
        let mut graph = Self::new();

        for (selector_key, leaf_data) in store.leaf_data() {
            let selector_str = selector_key.to_selector_string();
            let leaf = WorkingLeaf {
                selector: selector_str.clone(),
                file_path: leaf_data.file_path.clone(),
                name: leaf_data.name.clone(),
                qualified_name: leaf_data.qualified_name.clone(),
                kind: leaf_data.kind.clone(),
                start_line: leaf_data.start_line,
                end_line: leaf_data.end_line,
                source: leaf_data.source.clone(),
                source_hash: leaf_data.source_hash.clone(),
                parent_qualified_name: leaf_data.parent_qualified_name.clone(),
                children_qualified_names: leaf_data.children_qualified_names.clone(),
            };
            graph
                .file_leaves
                .entry(leaf.file_path.clone())
                .or_default()
                .push(selector_str.clone());
            graph.leaves.insert(selector_str, leaf);
        }

        Ok(graph)
    }

    /// Insert a pre-built working leaf into the graph.
    ///
    /// Used by the tool to populate the graph from a file extraction when
    /// no persisted knowledge store is available.
    pub fn insert_working_leaf(&mut self, selector: String, leaf: WorkingLeaf) {
        self.file_leaves
            .entry(leaf.file_path.clone())
            .or_default()
            .push(selector.clone());
        self.leaves.insert(selector, leaf);
    }

    /// Resolve a selector against the working graph.
    pub fn resolve_leaf(&self, selector: &Selector) -> Option<&WorkingLeaf> {
        let key = selector.to_string();
        self.leaves.get(&key)
    }

    /// Check if a selector resolves to an existing leaf.
    pub fn has_leaf(&self, selector: &Selector) -> bool {
        self.leaves.contains_key(&selector.to_string())
    }

    /// Get all leaves for a given file path.
    pub fn leaves_in_file(&self, file_path: &str) -> Vec<&WorkingLeaf> {
        self.file_leaves
            .get(file_path)
            .map(|selectors| {
                selectors
                    .iter()
                    .filter_map(|s| self.leaves.get(s))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the version chains for serialization at activity completion.
    pub fn version_chains(&self) -> &HashMap<String, LeafVersionChain> {
        &self.version_chains
    }

    /// Execute an edit on an existing leaf.
    ///
    /// 1. Validate source hash against file on disk
    /// 2. Replace the leaf's lines in the file
    /// 3. Write the file back to disk
    /// 4. Re-extract all leaves in the file
    /// 5. Update the working graph
    /// 6. Append to version chain
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
        let file_lines: Vec<&str> = file_content.lines().collect();

        // Validate source hash at start_line..end_line
        if leaf.start_line == 0 || leaf.end_line == 0 || leaf.end_line > file_lines.len() {
            return Err(WriteError::source_conflict(
                &leaf.source_hash,
                &format!(
                    "(line range {}..{} out of bounds for {}-line file)",
                    leaf.start_line,
                    leaf.end_line,
                    file_lines.len()
                ),
                &selector_str,
            ));
        }

        let actual_source = file_lines[leaf.start_line - 1..leaf.end_line]
            .join("\n")
            .trim()
            .to_string();
        let actual_hash = extractor::compute_source_hash(&actual_source);

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
        let new_hash = extractor::compute_source_hash(new_source.trim());
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
    pub fn insert_leaf(
        &mut self,
        selector: &Selector,
        new_source: &str,
        position: Option<&Selector>,
        reason: Option<&str>,
        workspace_root: &Path,
    ) -> Result<WriteResult, WriteError> {
        let selector_str = selector.to_string();
        let file_path = match selector {
            Selector::Leaf { path, .. } => path.clone(),
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
        let file_lines: Vec<&str> = file_content.lines().collect();

        // Determine insertion point
        let insert_after_line = if let Some(pos_selector) = position {
            let pos_str = pos_selector.to_string();
            // Strip "after:" prefix if present in position processing
            let anchor = self
                .leaves
                .get(&pos_str)
                .ok_or_else(|| WriteError::position_not_found(&pos_str))?;
            anchor.end_line
        } else {
            // Append before #[cfg(test)] mod tests, or at end of file
            find_test_module_line(&file_lines).unwrap_or(file_lines.len())
        };

        // Build new file content
        let mut new_lines: Vec<String> = Vec::new();
        for line in &file_lines[..insert_after_line] {
            new_lines.push(line.to_string());
        }
        new_lines.push(String::new()); // blank line separator
        for line in new_source.lines() {
            new_lines.push(line.to_string());
        }
        for line in &file_lines[insert_after_line..] {
            new_lines.push(line.to_string());
        }

        let new_content = new_lines.join("\n");
        let new_content = if file_content.ends_with('\n') && !new_content.ends_with('\n') {
            new_content + "\n"
        } else {
            new_content
        };

        // Write file
        fs::write(&abs_path, &new_content)
            .map_err(|e| WriteError::io_error(format!("write {}: {e}", abs_path.display())))?;

        // Re-extract
        let affected = self.re_extract_file(&file_path, &new_content, language);

        // Create version chain for new leaf
        let new_hash = extractor::compute_source_hash(new_source.trim());
        let chain = LeafVersionChain {
            leaf_id: selector_str.clone(),
            selector: selector_str.clone(),
            original_source_hash: new_hash.clone(),
            edits: vec![LeafEdit {
                edit_sequence: 0,
                source_hash: new_hash.clone(),
                source: new_source.trim().to_string(),
                timestamp: Utc::now().to_rfc3339(),
                reason: Some(reason.unwrap_or("created").to_string()),
            }],
        };
        self.version_chains.insert(selector_str.clone(), chain);

        Ok(WriteResult {
            status: "created".to_string(),
            selector: selector_str.clone(),
            edit_sequence: 0,
            new_source_hash: new_hash,
            affected_leaves: affected,
            new_leaf_id: Some(selector_str),
        })
    }

    /// Re-run the extractor on a modified file and update the working graph.
    ///
    /// Returns the list of leaf selectors whose positions changed.
    fn re_extract_file(
        &mut self,
        rel_path: &str,
        content: &str,
        language: Language,
    ) -> Vec<String> {
        let result = extractor::extract_file(content, language);

        // Collect old selectors for this file
        let old_selectors: Vec<String> =
            self.file_leaves.get(rel_path).cloned().unwrap_or_default();

        // Build new leaf set
        let mut new_selectors = Vec::new();
        let mut affected = Vec::new();

        for extracted in &result.leaves {
            let selector_str = format!(
                "leaf:{rel_path}#{}:{}",
                extracted.qualified_name, extracted.kind
            );
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

            let leaf = WorkingLeaf {
                selector: selector_str.clone(),
                file_path: rel_path.to_string(),
                name: extracted.name.clone(),
                qualified_name: extracted.qualified_name.clone(),
                kind: extracted.kind.clone(),
                start_line: extracted.start_line,
                end_line: extracted.end_line,
                source: extracted.source.clone(),
                source_hash: extracted.source_hash.clone(),
                parent_qualified_name: extracted.parent_qualified_name.clone(),
                children_qualified_names: extracted.children_qualified_names.clone(),
            };
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

        affected
    }

    /// Append to the version chain for a leaf, returning the edit_sequence number.
    fn append_version_chain(
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

    /// Get the source for a leaf (used by knowledge.pack integration).
    pub fn get_leaf_source(&self, selector: &Selector) -> Option<String> {
        self.resolve_leaf(selector).map(|l| l.source.clone())
    }

    /// Get all leaf selectors in the working graph.
    #[cfg(test)]
    pub fn all_selectors(&self) -> Vec<String> {
        self.leaves.keys().cloned().collect()
    }
}

/// Find the line number where `#[cfg(test)]` `mod tests` begins.
/// Returns 0-indexed line to insert before (i.e., the `#[cfg(test)]` line).
fn find_test_module_line(lines: &[&str]) -> Option<usize> {
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

#[cfg(test)]
mod tests {
    use super::super::extractor::ExtractionResult;
    use super::*;
    use std::path::PathBuf;

    fn make_test_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = dir.path().to_path_buf();
        (dir, ws)
    }

    fn write_rust_file(ws: &Path, rel_path: &str, content: &str) {
        let full = ws.join(rel_path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, content).unwrap();
    }

    const SAMPLE_RS: &str = r#"use std::fmt;

pub fn alpha() -> i32 {
    1
}

pub fn beta() -> i32 {
    2
}

pub fn gamma() -> i32 {
    3
}
"#;

    #[test]
    fn edit_leaf_replaces_source_and_updates_positions() {
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/lib.rs", SAMPLE_RS);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(SAMPLE_RS, Language::Rust);
        populate_graph_from_extraction(&mut graph, "src/lib.rs", &extraction);

        let selector: Selector = "leaf:src/lib.rs#alpha:function".parse().unwrap();
        let old_beta = graph
            .resolve_leaf(&"leaf:src/lib.rs#beta:function".parse().unwrap())
            .unwrap();
        let old_beta_start = old_beta.start_line;

        // Edit alpha to add a line (makes it longer)
        let new_source = "pub fn alpha() -> i32 {\n    let x = 1;\n    x\n}";
        let result = graph
            .edit_leaf(&selector, new_source, Some("add variable"), &ws)
            .unwrap();

        assert_eq!(result.status, "ok");
        assert_eq!(result.edit_sequence, 1);

        // Beta's start_line should have shifted by 1 (added one line to alpha)
        let new_beta = graph
            .resolve_leaf(&"leaf:src/lib.rs#beta:function".parse().unwrap())
            .unwrap();
        assert_eq!(new_beta.start_line, old_beta_start + 1);
    }

    #[test]
    fn edit_detects_source_conflict() {
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/lib.rs", SAMPLE_RS);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(SAMPLE_RS, Language::Rust);
        populate_graph_from_extraction(&mut graph, "src/lib.rs", &extraction);

        // Modify file on disk externally
        let modified = SAMPLE_RS.replace(
            "pub fn alpha() -> i32 {\n    1\n}",
            "pub fn alpha() -> i64 {\n    42\n}",
        );
        write_rust_file(&ws, "src/lib.rs", &modified);

        let selector: Selector = "leaf:src/lib.rs#alpha:function".parse().unwrap();
        let err = graph
            .edit_leaf(&selector, "pub fn alpha() {}", None, &ws)
            .unwrap_err();
        assert_eq!(err.kind, "source_conflict");
        assert!(err.expected_source_hash.is_some());
        assert!(err.actual_source.is_some());
    }

    #[test]
    fn offset_drift_correctness_multi_edit() {
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/lib.rs", SAMPLE_RS);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(SAMPLE_RS, Language::Rust);
        populate_graph_from_extraction(&mut graph, "src/lib.rs", &extraction);

        // Edit alpha (near top) - add 2 extra lines
        let selector_alpha: Selector = "leaf:src/lib.rs#alpha:function".parse().unwrap();
        let new_alpha = "pub fn alpha() -> i32 {\n    let a = 1;\n    let b = 2;\n    a + b\n}";
        graph
            .edit_leaf(&selector_alpha, new_alpha, None, &ws)
            .unwrap();

        // Now edit gamma (near bottom) - should succeed without conflict
        let selector_gamma: Selector = "leaf:src/lib.rs#gamma:function".parse().unwrap();
        let new_gamma = "pub fn gamma() -> i32 {\n    99\n}";
        let result = graph
            .edit_leaf(&selector_gamma, new_gamma, None, &ws)
            .unwrap();
        assert_eq!(result.status, "ok");

        // Verify file content
        let final_content = fs::read_to_string(ws.join("src/lib.rs")).unwrap();
        assert!(final_content.contains("a + b"));
        assert!(final_content.contains("99"));
    }

    #[test]
    fn insert_leaf_with_position() {
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/lib.rs", SAMPLE_RS);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(SAMPLE_RS, Language::Rust);
        populate_graph_from_extraction(&mut graph, "src/lib.rs", &extraction);

        let new_selector: Selector = "leaf:src/lib.rs#delta:function".parse().unwrap();
        let position: Selector = "leaf:src/lib.rs#alpha:function".parse().unwrap();
        let result = graph
            .insert_leaf(
                &new_selector,
                "pub fn delta() -> i32 {\n    4\n}",
                Some(&position),
                Some("new function"),
                &ws,
            )
            .unwrap();

        assert_eq!(result.status, "created");
        assert_eq!(result.edit_sequence, 0);

        // Verify the new function exists in the file
        let content = fs::read_to_string(ws.join("src/lib.rs")).unwrap();
        assert!(content.contains("pub fn delta()"));

        // Verify it's in the working graph
        assert!(graph.has_leaf(&new_selector));
    }

    #[test]
    fn insert_leaf_appends_before_test_module() {
        let source_with_tests = format!(
            "{}\n#[cfg(test)]\nmod tests {{\n    #[test]\n    fn it_works() {{}}\n}}\n",
            SAMPLE_RS.trim()
        );
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/lib.rs", &source_with_tests);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(&source_with_tests, Language::Rust);
        populate_graph_from_extraction(&mut graph, "src/lib.rs", &extraction);

        let new_selector: Selector = "leaf:src/lib.rs#delta:function".parse().unwrap();
        graph
            .insert_leaf(
                &new_selector,
                "pub fn delta() -> i32 {\n    4\n}",
                None,
                None,
                &ws,
            )
            .unwrap();

        let content = fs::read_to_string(ws.join("src/lib.rs")).unwrap();
        let delta_pos = content.find("pub fn delta()").unwrap();
        let test_pos = content.find("#[cfg(test)]").unwrap();
        assert!(delta_pos < test_pos, "delta should be before test module");
    }

    #[test]
    fn version_chain_accumulates_edits() {
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/lib.rs", SAMPLE_RS);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(SAMPLE_RS, Language::Rust);
        populate_graph_from_extraction(&mut graph, "src/lib.rs", &extraction);

        let selector: Selector = "leaf:src/lib.rs#alpha:function".parse().unwrap();
        let sel_str = selector.to_string();

        // Edit 1
        graph
            .edit_leaf(
                &selector,
                "pub fn alpha() -> i32 {\n    10\n}",
                Some("first edit"),
                &ws,
            )
            .unwrap();
        // Edit 2
        graph
            .edit_leaf(
                &selector,
                "pub fn alpha() -> i32 {\n    20\n}",
                Some("second edit"),
                &ws,
            )
            .unwrap();
        // Edit 3
        graph
            .edit_leaf(
                &selector,
                "pub fn alpha() -> i32 {\n    30\n}",
                Some("third edit"),
                &ws,
            )
            .unwrap();

        let chain = graph.version_chains().get(&sel_str).unwrap();
        assert_eq!(chain.edits.len(), 3);
        assert_eq!(chain.edits[0].edit_sequence, 1);
        assert_eq!(chain.edits[1].edit_sequence, 2);
        assert_eq!(chain.edits[2].edit_sequence, 3);
        assert_eq!(chain.edits[0].reason.as_deref(), Some("first edit"));
        assert_eq!(chain.edits[2].reason.as_deref(), Some("third edit"));
    }

    #[test]
    fn version_chain_serializes_to_json() {
        let chain = LeafVersionChain {
            leaf_id: "leaf:src/lib.rs#alpha:function".to_string(),
            selector: "leaf:src/lib.rs#alpha:function".to_string(),
            original_source_hash: "abc123".to_string(),
            edits: vec![
                LeafEdit {
                    edit_sequence: 1,
                    source_hash: "def456".to_string(),
                    source: "pub fn alpha() { 1 }".to_string(),
                    timestamp: "2026-04-09T07:00:00Z".to_string(),
                    reason: Some("first".to_string()),
                },
                LeafEdit {
                    edit_sequence: 2,
                    source_hash: "ghi789".to_string(),
                    source: "pub fn alpha() { 2 }".to_string(),
                    timestamp: "2026-04-09T07:01:00Z".to_string(),
                    reason: Some("second".to_string()),
                },
                LeafEdit {
                    edit_sequence: 3,
                    source_hash: "jkl012".to_string(),
                    source: "pub fn alpha() { 3 }".to_string(),
                    timestamp: "2026-04-09T07:02:00Z".to_string(),
                    reason: None,
                },
            ],
        };

        let json = serde_json::to_string(&chain).unwrap();
        let deserialized: LeafVersionChain = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.edits.len(), 3);
        assert_eq!(deserialized.edits[0].edit_sequence, 1);
        assert_eq!(deserialized.edits[2].edit_sequence, 3);
        assert_eq!(deserialized.original_source_hash, "abc123");
    }

    #[test]
    fn unsupported_language_returns_error() {
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/main.go", "package main\n");

        let mut graph = WorkingGraph::new();
        let selector: Selector = "leaf:src/main.go#main:function".parse().unwrap();
        // Manually add a leaf
        graph.leaves.insert(
            selector.to_string(),
            WorkingLeaf {
                selector: selector.to_string(),
                file_path: "src/main.go".to_string(),
                name: "main".to_string(),
                qualified_name: "main".to_string(),
                kind: "function".to_string(),
                start_line: 1,
                end_line: 1,
                source: "package main".to_string(),
                source_hash: extractor::compute_source_hash("package main"),
                parent_qualified_name: None,
                children_qualified_names: vec![],
            },
        );

        let err = graph
            .edit_leaf(&selector, "func main() {}", None, &ws)
            .unwrap_err();
        assert_eq!(err.kind, "unsupported_language");
    }

    #[test]
    fn find_test_module_line_works() {
        let lines = vec![
            "use std::fmt;",
            "",
            "pub fn foo() {}",
            "",
            "#[cfg(test)]",
            "mod tests {",
            "    #[test]",
            "    fn it_works() {}",
            "}",
        ];
        assert_eq!(find_test_module_line(&lines), Some(4));
    }

    #[test]
    fn find_test_module_line_returns_none_without_tests() {
        let lines = vec!["pub fn foo() {}", "pub fn bar() {}"];
        assert_eq!(find_test_module_line(&lines), None);
    }

    #[test]
    fn fs_write_conflict_detection() {
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/lib.rs", SAMPLE_RS);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(SAMPLE_RS, Language::Rust);
        populate_graph_from_extraction(&mut graph, "src/lib.rs", &extraction);

        // Simulate fs.write modifying the file externally
        let tampered = SAMPLE_RS.replace(
            "pub fn beta() -> i32 {\n    2\n}",
            "pub fn beta() -> String {\n    \"two\".to_string()\n}",
        );
        write_rust_file(&ws, "src/lib.rs", &tampered);

        // Next knowledge.write to beta should detect the conflict
        let selector: Selector = "leaf:src/lib.rs#beta:function".parse().unwrap();
        let err = graph
            .edit_leaf(&selector, "pub fn beta() -> i32 {\n    22\n}", None, &ws)
            .unwrap_err();
        assert_eq!(err.kind, "source_conflict");
        assert!(err.expected_source_hash.is_some());
        assert!(err.actual_source.is_some());
    }

    #[test]
    fn working_graph_reflects_edits_for_subsequent_reads() {
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/lib.rs", SAMPLE_RS);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(SAMPLE_RS, Language::Rust);
        populate_graph_from_extraction(&mut graph, "src/lib.rs", &extraction);

        let selector: Selector = "leaf:src/lib.rs#alpha:function".parse().unwrap();

        // Verify original source
        let original = graph.get_leaf_source(&selector).unwrap();
        assert!(original.contains("1"));

        // Edit the leaf
        graph
            .edit_leaf(&selector, "pub fn alpha() -> i32 {\n    42\n}", None, &ws)
            .unwrap();

        // The working graph should now return the updated source
        let updated = graph.get_leaf_source(&selector).unwrap();
        assert!(updated.contains("42"));
        assert!(!updated.contains("\n    1\n"));
    }

    #[test]
    fn extractor_selection_by_rs_extension() {
        let (_dir, ws) = make_test_dir();
        write_rust_file(&ws, "src/lib.rs", SAMPLE_RS);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(SAMPLE_RS, Language::Rust);
        populate_graph_from_extraction(&mut graph, "src/lib.rs", &extraction);

        let selector: Selector = "leaf:src/lib.rs#alpha:function".parse().unwrap();
        let result = graph
            .edit_leaf(&selector, "pub fn alpha() -> i32 {\n    100\n}", None, &ws)
            .unwrap();
        assert_eq!(result.status, "ok");
    }

    #[test]
    fn extractor_selection_by_py_extension() {
        let (_dir, ws) = make_test_dir();
        let py_source = "def foo():\n    return 1\n\ndef bar():\n    return 2\n";
        write_rust_file(&ws, "src/main.py", py_source);

        let mut graph = WorkingGraph::new();
        let extraction = extractor::extract_file(py_source, Language::Python);
        populate_graph_from_extraction(&mut graph, "src/main.py", &extraction);

        let selector: Selector = "leaf:src/main.py#foo:function".parse().unwrap();
        let result = graph
            .edit_leaf(&selector, "def foo():\n    return 42", None, &ws)
            .unwrap();
        assert_eq!(result.status, "ok");
    }

    /// Helper: populate graph from extraction result (simulates loading from store).
    fn populate_graph_from_extraction(
        graph: &mut WorkingGraph,
        file_path: &str,
        extraction: &ExtractionResult,
    ) {
        let mut selectors = Vec::new();
        for leaf in &extraction.leaves {
            let sel = format!("leaf:{file_path}#{}:{}", leaf.qualified_name, leaf.kind);
            selectors.push(sel.clone());
            graph.leaves.insert(
                sel.clone(),
                WorkingLeaf {
                    selector: sel,
                    file_path: file_path.to_string(),
                    name: leaf.name.clone(),
                    qualified_name: leaf.qualified_name.clone(),
                    kind: leaf.kind.clone(),
                    start_line: leaf.start_line,
                    end_line: leaf.end_line,
                    source: leaf.source.clone(),
                    source_hash: leaf.source_hash.clone(),
                    parent_qualified_name: leaf.parent_qualified_name.clone(),
                    children_qualified_names: leaf.children_qualified_names.clone(),
                },
            );
        }
        graph.file_leaves.insert(file_path.to_string(), selectors);
    }
}
