//! Documentation index, search, and maintenance commands.
//!
//! This module was split from a monolithic `docs.rs` (see ORB-00250).
//! Submodules own one concern each; the `impl OrbitRuntime` surface and
//! public API re-exports live here.

mod add_root;
mod artifact_ref;
mod config;
mod frontmatter;
mod migrate;
mod path_util;
mod search;
mod types;
mod walk;

#[cfg(test)]
mod tests;

use orbit_common::types::{OrbitError, Task};
pub use orbit_search::{
    AdrIndexParams, AdrIndexResult, AdrSearchResult, DocIndexParams, DocIndexResult,
    DocSearchResult, SearchResult,
};
use orbit_search::{score_adr_record, score_doc_record, sort_search_results};

use crate::OrbitRuntime;

pub use config::{AdrSearchConfig, DocsSearchConfig};
pub use frontmatter::parse_doc_frontmatter_strict;
pub use types::{
    ArtifactRef, DocAddOutcome, DocFrontmatter, DocMigrationChange, DocMigrationReport, DocRecord,
    DocShow, DocType, TaskRelatedDoc,
};
pub use walk::walk_docs_roots;

// Re-export the toml parsers at the docs root so `command::docs::parse_*_from_config_toml`
// surface is unchanged.
pub use config::{
    parse_adr_search_config_from_config_toml, parse_docs_roots_from_config_toml,
    parse_docs_search_config_from_config_toml,
};

// Bring helper fns into scope so the pasted impl block (lines 291-408 of original)
// continues to compile with bare calls.
use self::add_root::add_docs_root;
use self::config::{
    read_adr_search_config_from_config_path, read_docs_roots_from_config_path,
    read_docs_search_config_from_config_path, read_task_context_docs_roots_from_config_path,
};
use self::migrate::migrate_docs;
use self::search::{
    adr_embedding_sources, adr_search_source, adr_status_in_docs_search, doc_embedding_sources,
    doc_search_source, related_docs_for_context, show_doc,
};

// The original impl block (291-408) is preserved verbatim except for path adjustments
// that are mechanical (use of super:: paths). All bodies delegate to submodules.
impl OrbitRuntime {
    pub fn docs_roots(&self) -> Result<Vec<String>, OrbitError> {
        read_docs_roots_from_config_path(&self.config_path())
    }

    pub fn docs_search_config(&self) -> Result<DocsSearchConfig, OrbitError> {
        read_docs_search_config_from_config_path(&self.config_path())
    }

    pub fn adr_search_config(&self) -> Result<AdrSearchConfig, OrbitError> {
        read_adr_search_config_from_config_path(&self.config_path())
    }

    pub fn list_docs(
        &self,
        doc_type: Option<DocType>,
        tag: Option<&str>,
    ) -> Result<Vec<DocRecord>, OrbitError> {
        let mut records = walk_docs_roots(&self.paths().repo_root, &self.docs_roots()?)?;
        if let Some(doc_type) = doc_type {
            records.retain(|record| record.frontmatter.doc_type == doc_type);
        }
        if let Some(tag) = tag.map(|value| value.trim().to_ascii_lowercase())
            && !tag.is_empty()
        {
            records.retain(|record| {
                record
                    .frontmatter
                    .tags
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&tag))
            });
        }
        Ok(records)
    }

    pub fn show_doc(&self, path: &str) -> Result<DocShow, OrbitError> {
        show_doc(&self.paths().repo_root, &self.docs_roots()?, path)
    }

    pub fn search_docs(
        &self,
        query: &str,
        limit: Option<usize>,
        include_superseded: bool,
    ) -> Result<Vec<SearchResult>, OrbitError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(OrbitError::InvalidInput(
                "docs search query must not be empty".to_string(),
            ));
        }
        let limit = limit.unwrap_or(20);
        let query_lower = query.to_ascii_lowercase();
        let mut scored = self
            .list_docs(None, None)?
            .into_iter()
            .map(doc_search_source)
            .filter_map(|record| score_doc_record(record, &query_lower))
            .map(SearchResult::Doc)
            .collect::<Vec<_>>();
        scored.extend(
            self.stores()
                .adrs()
                .list()?
                .into_iter()
                .filter(|adr| adr_status_in_docs_search(adr.status, include_superseded))
                .map(adr_search_source)
                .filter_map(|adr| score_adr_record(adr, &query_lower))
                .map(SearchResult::Adr),
        );
        sort_search_results(&mut scored);
        scored.truncate(limit);
        Ok(scored)
    }

    pub fn related_docs_for_task(
        &self,
        task: &Task,
        limit: Option<usize>,
    ) -> Result<Vec<TaskRelatedDoc>, OrbitError> {
        let roots = read_task_context_docs_roots_from_config_path(&self.config_path())?;
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        // Tasks do not yet have a first-class `related_features` field, so the
        // agent-facing feature join uses normalized task tags as the feature
        // selectors until that storage field exists.
        related_docs_for_context(
            &self.paths().repo_root,
            &roots,
            &task.context_files,
            &task.tags,
            limit,
        )
    }

    pub fn add_docs_root(&self, path: &str) -> Result<DocAddOutcome, OrbitError> {
        add_docs_root(&self.paths().repo_root, &self.config_path(), path)
    }

    pub fn index_docs(&self, params: DocIndexParams) -> Result<DocIndexResult, OrbitError> {
        let roots = self.docs_roots()?;
        let sources = doc_embedding_sources(&self.paths().repo_root, &roots)?;
        orbit_search::doc_index(&self.stores().semantic_vector, &sources, params)
    }

    pub fn index_adrs(&self, params: AdrIndexParams) -> Result<AdrIndexResult, OrbitError> {
        let sources = adr_embedding_sources(&self.paths().repo_root, self.stores().adrs().list()?)?;
        orbit_search::adr_index(&self.stores().semantic_vector, &sources, params)
    }

    pub fn migrate_docs(&self, dry_run: bool) -> Result<DocMigrationReport, OrbitError> {
        migrate_docs(&self.paths().repo_root, dry_run)
    }
}

// The three parse_*_from_config_toml live in config.rs and are re-exported above.
// No other top-level items remain in this file.
