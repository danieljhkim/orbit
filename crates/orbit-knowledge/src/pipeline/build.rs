use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use rayon::prelude::*;

use crate::error::KnowledgeError;
use crate::extract::{self, FileKind, identity_key, leaf_location, node_id};
use crate::graph::nodes::{BaseNodeFields, DirNode, FileNode, LeafKind, LeafNode, ReExport};
use crate::graph::object_store::GraphObjectStore;
use crate::pipeline::context::PipelineContext;
use tracing::debug;

// ---------------------------------------------------------------------------
// build_graph_dirs
// ---------------------------------------------------------------------------

/// Create DirNode entries for every directory containing scanned files.
pub fn build_graph_dirs(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    // Collect unique directory paths (relative, using "/" separators)
    let mut dir_set: BTreeSet<String> = BTreeSet::new();
    dir_set.insert(".".to_string()); // root always present

    for rel_path in &ctx.file_paths {
        let mut current = rel_path.as_path();
        while let Some(parent) = current.parent() {
            let dir_str = if parent.as_os_str().is_empty() {
                ".".to_string()
            } else {
                parent.to_string_lossy().into_owned()
            };
            if !dir_set.insert(dir_str) {
                break; // already present, ancestors are too
            }
            current = parent;
        }
    }

    // Build parent → children map
    let mut dir_children_map: HashMap<String, Vec<String>> = HashMap::new();
    for d in &dir_set {
        if d == "." {
            continue;
        }
        let parent_str = Path::new(d)
            .parent()
            .map(|p| {
                if p.as_os_str().is_empty() {
                    ".".to_string()
                } else {
                    p.to_string_lossy().into_owned()
                }
            })
            .unwrap_or_else(|| ".".to_string());
        dir_children_map
            .entry(parent_str)
            .or_default()
            .push(d.clone());
    }

    // Generate IDs and create nodes
    let mut id_map: HashMap<String, String> = HashMap::new();
    for d in &dir_set {
        let location = format!("{d}/");
        let id = node_id("dir", &location, "dir");
        id_map.insert(d.clone(), id);
    }

    let root_id = id_map["."].clone();
    ctx.graph.root_dir_id = root_id;

    for d in &dir_set {
        let location = format!("{d}/");
        let id = id_map[d].clone();
        let ikey = identity_key("dir", &location, "dir");

        let parent_id = if d == "." {
            None
        } else {
            let parent_str = Path::new(d.as_str())
                .parent()
                .map(|p| {
                    if p.as_os_str().is_empty() {
                        ".".to_string()
                    } else {
                        p.to_string_lossy().into_owned()
                    }
                })
                .unwrap_or_else(|| ".".to_string());
            Some(id_map[&parent_str].clone())
        };

        let child_dir_ids: Vec<String> = dir_children_map
            .get(d)
            .map(|kids| kids.iter().map(|k| id_map[k].clone()).collect())
            .unwrap_or_default();

        // file_children will be populated by build_graph_files
        ctx.graph.dirs.push(DirNode {
            base: BaseNodeFields {
                id,
                identity_key: ikey,
                object_hash: None,
                name: dir_name(d),
                location,
                language: String::new(),
                description: String::new(),
                parent_id,
                is_locked: false,
                lineage_locked: false,
                lock_owner: None,
                lock_reason: String::new(),
            },
            dir_children: child_dir_ids,
            file_children: Vec::new(),
        });
    }

    Ok(())
}

fn dir_name(dir_path: &str) -> String {
    if dir_path == "." {
        return ".".to_string();
    }
    Path::new(dir_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir_path.to_string())
}

// ---------------------------------------------------------------------------
// build_graph_files
// ---------------------------------------------------------------------------

/// Create FileNode entries for every scanned file, linking to parent dirs.
pub fn build_graph_files(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    // Pre-build dir location → index map for wiring file_children
    let dir_id_map: HashMap<String, usize> = ctx
        .graph
        .dirs
        .iter()
        .enumerate()
        .map(|(i, d)| (d.base.location.clone(), i))
        .collect();

    for rel_path in &ctx.file_paths {
        let rel_str = rel_path.to_string_lossy().into_owned();
        let id = node_id("file", &rel_str, "file");
        let ikey = identity_key("file", &rel_str, "file");

        let extension = rel_path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_string);

        let language = extension
            .as_deref()
            .map(FileKind::from_extension)
            .map(|k| k.as_str().to_string())
            .unwrap_or_default();

        let parent_dir_str = rel_path
            .parent()
            .map(|p| {
                if p.as_os_str().is_empty() {
                    ".".to_string()
                } else {
                    p.to_string_lossy().into_owned()
                }
            })
            .unwrap_or_else(|| ".".to_string());
        let parent_location = format!("{parent_dir_str}/");

        let parent_id = dir_id_map
            .get(&parent_location)
            .map(|&i| ctx.graph.dirs[i].base.id.clone());

        let name = rel_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| rel_str.clone());

        let file_node = FileNode {
            base: BaseNodeFields {
                id: id.clone(),
                identity_key: ikey,
                object_hash: None,
                name,
                location: rel_str,
                language,
                description: String::new(),
                parent_id: parent_id.clone(),
                is_locked: false,
                lineage_locked: false,
                lock_owner: None,
                lock_reason: String::new(),
            },
            extension,
            source_blob_hash: None,
            source: String::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            re_exports: Vec::new(),
            leaf_children: Vec::new(),
        };
        ctx.graph.files.push(file_node);

        // Wire into parent dir's file_children
        if let Some(&dir_idx) = dir_id_map.get(&parent_location) {
            ctx.graph.dirs[dir_idx].file_children.push(id);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// build_graph_leaves
// ---------------------------------------------------------------------------

/// Extract leaf nodes from source files using file-kind-dispatched extractors.
///
/// Covers code (tree-sitter), markdown, structured config (YAML/JSON/TOML),
/// and tabular data (CSV/TSV). Non-extractable files yield no leaves.
pub fn build_graph_leaves(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    let registry = extract::ExtractorRegistry::new();
    let prior_files = load_prior_file_snapshots(ctx);
    let changed_paths: HashSet<String> = ctx.changed_paths.iter().cloned().collect();

    let file_infos: Vec<LeafBuildInput> = ctx
        .graph
        .files
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let ext = f.extension.as_deref()?;
            let kind = FileKind::from_extension(ext);
            if !kind.is_extractable() {
                return None;
            }
            Some(LeafBuildInput {
                file_idx: i,
                location: f.base.location.clone(),
                file_id: f.base.id.clone(),
                file_kind: kind,
            })
        })
        .collect();

    let mut outputs: Vec<LeafBuildOutput> = file_infos
        .par_iter()
        .filter_map(|info| {
            build_file_leaves(ctx, &registry, prior_files.as_ref(), &changed_paths, info)
        })
        .collect();
    outputs.sort_by_key(LeafBuildOutput::file_idx);

    for output in outputs {
        match output {
            LeafBuildOutput::Reused { file_idx, snapshot } => {
                reuse_prior_file(ctx, file_idx, snapshot);
            }
            LeafBuildOutput::Extracted { file_idx, file } => {
                apply_extracted_file(ctx, file_idx, file);
            }
        }
    }

    Ok(())
}

struct LeafBuildInput {
    file_idx: usize,
    location: String,
    file_id: String,
    file_kind: FileKind,
}

struct ExtractedFile {
    source_blob_hash: String,
    source: String,
    exports: Vec<String>,
    re_exports: Vec<ReExport>,
    leaf_children: Vec<String>,
    leaves: Vec<LeafNode>,
}

enum LeafBuildOutput {
    Reused {
        file_idx: usize,
        snapshot: PriorFileSnapshot,
    },
    Extracted {
        file_idx: usize,
        file: ExtractedFile,
    },
}

impl LeafBuildOutput {
    fn file_idx(&self) -> usize {
        match self {
            Self::Reused { file_idx, .. } | Self::Extracted { file_idx, .. } => *file_idx,
        }
    }
}

fn build_file_leaves(
    ctx: &PipelineContext,
    registry: &extract::ExtractorRegistry,
    prior_files: Option<&HashMap<String, PriorFileSnapshot>>,
    changed_paths: &HashSet<String>,
    info: &LeafBuildInput,
) -> Option<LeafBuildOutput> {
    if let Some(snapshot) = reusable_prior_snapshot(
        ctx,
        prior_files,
        changed_paths,
        &info.location,
        &info.file_id,
    ) {
        return Some(LeafBuildOutput::Reused {
            file_idx: info.file_idx,
            snapshot,
        });
    }

    let abs = ctx.repo_path.join(&info.location);
    let content = fs::read_to_string(&abs).ok()?;
    let extractor = registry.get(info.file_kind)?;
    let result = extractor.extract(&content);
    let file_hash_at_capture = ctx.new_hashes.get(&info.location).cloned();
    let mut file = extracted_file_from_result(
        &info.location,
        &info.file_id,
        info.file_kind,
        content,
        result,
        file_hash_at_capture,
    );
    preserve_prior_leaf_identities(&mut file, prior_files, &ctx.new_hashes, &info.location);

    Some(LeafBuildOutput::Extracted {
        file_idx: info.file_idx,
        file,
    })
}

fn extracted_file_from_result(
    location: &str,
    file_id: &str,
    file_kind: FileKind,
    content: String,
    result: extract::ExtractionResult,
    file_hash_at_capture: Option<String>,
) -> ExtractedFile {
    let source_blob_hash = extract::compute_source_hash(&content);
    let (exports, re_exports) = file_exports(&result.exports);
    let mut leaf_children = Vec::new();
    let mut leaves = Vec::with_capacity(result.leaves.len());

    for extracted in &result.leaves {
        let loc = leaf_location(location, &extracted.qualified_name);
        let id = node_id("symbol", &loc, &extracted.kind);
        let ikey = identity_key("symbol", &loc, &extracted.kind);
        let kind = parse_leaf_kind(&extracted.kind, extracted.depth);

        leaf_children.push(id.clone());
        leaves.push(LeafNode {
            base: BaseNodeFields {
                id,
                identity_key: ikey,
                object_hash: None,
                name: extracted.name.clone(),
                location: loc,
                language: file_kind.as_str().to_string(),
                description: String::new(),
                parent_id: Some(file_id.to_string()),
                is_locked: false,
                lineage_locked: false,
                lock_owner: None,
                lock_reason: String::new(),
            },
            kind,
            source: extracted.source.clone(),
            source_blob_hash: None,
            source_hash: Some(extracted.source_hash.clone()),
            file_hash_at_capture: file_hash_at_capture.clone(),
            history: Vec::new(),
            input_signature: Vec::new(),
            output_signature: Vec::new(),
            start_line: Some(extracted.start_line as u32),
            end_line: Some(extracted.end_line as u32),
            children: extracted
                .children_qualified_names
                .iter()
                .map(|qn| {
                    let child_loc = leaf_location(location, qn);
                    node_id("symbol", &child_loc, "method")
                })
                .collect(),
        });
    }

    ExtractedFile {
        source_blob_hash,
        source: content,
        exports,
        re_exports,
        leaf_children,
        leaves,
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct LeafIdentityMatchKey {
    qualified_name: String,
    kind: String,
    source_hash: Option<String>,
}

#[derive(Clone, Debug)]
struct PriorLeafIdentity {
    id: String,
    identity_key: String,
}

fn preserve_prior_leaf_identities(
    file: &mut ExtractedFile,
    prior_files: Option<&HashMap<String, PriorFileSnapshot>>,
    current_hashes: &HashMap<String, String>,
    current_location: &str,
) {
    let Some(prior_files) = prior_files else {
        return;
    };

    if let Some(snapshot) = prior_files.get(current_location) {
        apply_prior_leaf_identities(file, &snapshot.leaves, false);
        return;
    }

    let mut candidates = prior_files
        .iter()
        .filter(|(location, _)| location.as_str() != current_location)
        .filter(|(location, _)| !current_hashes.contains_key(location.as_str()))
        .filter(|(_, snapshot)| {
            snapshot.source_blob_hash.as_deref() == Some(&file.source_blob_hash)
        })
        .map(|(_, snapshot)| snapshot);

    let Some(candidate) = candidates.next() else {
        return;
    };
    if candidates.next().is_some() {
        return;
    }

    apply_prior_leaf_identities(file, &candidate.leaves, true);
}

fn apply_prior_leaf_identities(
    file: &mut ExtractedFile,
    prior_leaves: &[LeafNode],
    include_source_hash: bool,
) {
    let prior_by_key = unique_prior_leaf_identities(prior_leaves, include_source_hash);
    let mut id_rewrites = HashMap::new();

    for leaf in &mut file.leaves {
        let Some(key) = leaf_identity_match_key(leaf, include_source_hash) else {
            continue;
        };
        let Some(prior) = prior_by_key.get(&key) else {
            continue;
        };

        let default_id = leaf.base.id.clone();
        leaf.base.id = prior.id.clone();
        leaf.base.identity_key = prior.identity_key.clone();
        id_rewrites.insert(default_id, leaf.base.id.clone());
    }

    if id_rewrites.is_empty() {
        return;
    }

    for child_id in &mut file.leaf_children {
        if let Some(rewritten) = id_rewrites.get(child_id) {
            *child_id = rewritten.clone();
        }
    }
    for leaf in &mut file.leaves {
        for child_id in &mut leaf.children {
            if let Some(rewritten) = id_rewrites.get(child_id) {
                *child_id = rewritten.clone();
            }
        }
    }
}

fn unique_prior_leaf_identities(
    leaves: &[LeafNode],
    include_source_hash: bool,
) -> HashMap<LeafIdentityMatchKey, PriorLeafIdentity> {
    let mut identities = HashMap::new();
    let mut duplicates = HashSet::new();

    for leaf in leaves {
        let Some(key) = leaf_identity_match_key(leaf, include_source_hash) else {
            continue;
        };
        if identities
            .insert(
                key.clone(),
                PriorLeafIdentity {
                    id: leaf.base.id.clone(),
                    identity_key: leaf.base.identity_key.clone(),
                },
            )
            .is_some()
        {
            duplicates.insert(key);
        }
    }

    for key in duplicates {
        identities.remove(&key);
    }

    identities
}

fn leaf_identity_match_key(
    leaf: &LeafNode,
    include_source_hash: bool,
) -> Option<LeafIdentityMatchKey> {
    let (_, qualified_name) = leaf.base.location.split_once('#')?;
    let source_hash = if include_source_hash {
        Some(leaf.source_hash.clone()?)
    } else {
        None
    };
    Some(LeafIdentityMatchKey {
        qualified_name: qualified_name.to_string(),
        kind: leaf.kind.to_string(),
        source_hash,
    })
}

#[derive(Clone)]
struct PriorFileSnapshot {
    source_blob_hash: Option<String>,
    source: String,
    imports: Vec<String>,
    exports: Vec<String>,
    re_exports: Vec<ReExport>,
    leaf_children: Vec<String>,
    leaves: Vec<LeafNode>,
}

fn load_prior_file_snapshots(ctx: &PipelineContext) -> Option<HashMap<String, PriorFileSnapshot>> {
    if !ctx.incremental {
        return None;
    }

    let store = GraphObjectStore::new(ctx.graph_dir());
    let prior_graph = match store.read_graph(&ctx.ref_name, None, ctx.default_ref_name.as_ref()) {
        Ok(graph) => graph,
        Err(error) => {
            debug!(
                ref_name = %ctx.ref_name,
                error = %error,
                "incremental graph rebuild falling back to full leaf extraction: prior graph unavailable"
            );
            return None;
        }
    };

    let leaves_by_id: HashMap<String, LeafNode> = prior_graph
        .leaves
        .into_iter()
        .map(|leaf| (leaf.base.id.clone(), leaf))
        .collect();

    let mut snapshots = HashMap::new();
    for file in prior_graph.files {
        let mut leaves = Vec::with_capacity(file.leaf_children.len());
        let mut missing_leaf = None;
        for leaf_id in &file.leaf_children {
            match leaves_by_id.get(leaf_id) {
                Some(leaf) => leaves.push(leaf.clone()),
                None => {
                    missing_leaf = Some(leaf_id.clone());
                    break;
                }
            }
        }

        if let Some(missing_leaf) = missing_leaf {
            debug!(
                file = %file.base.location,
                missing_leaf = %missing_leaf,
                "incremental graph rebuild cannot reuse prior file: leaf child missing from prior graph"
            );
            continue;
        }

        snapshots.insert(
            file.base.location.clone(),
            PriorFileSnapshot {
                source_blob_hash: file.source_blob_hash.clone(),
                source: file.source.clone(),
                imports: file.imports.clone(),
                exports: file.exports.clone(),
                re_exports: file.re_exports.clone(),
                leaf_children: file.leaf_children.clone(),
                leaves,
            },
        );
    }

    Some(snapshots)
}

fn reusable_prior_snapshot(
    ctx: &PipelineContext,
    prior_files: Option<&HashMap<String, PriorFileSnapshot>>,
    changed_paths: &HashSet<String>,
    location: &str,
    file_id: &str,
) -> Option<PriorFileSnapshot> {
    if changed_paths.contains(location) {
        return None;
    }

    let snapshot = prior_files?.get(location)?;
    let new_hash = ctx.new_hashes.get(location)?;

    if snapshot.source_blob_hash.as_ref() != Some(new_hash) {
        debug!(
            file = %location,
            "incremental graph rebuild treating unchanged-path candidate as changed: prior file source hash does not match new hash"
        );
        return None;
    }

    if snapshot
        .leaves
        .iter()
        .any(|leaf| leaf.base.parent_id.as_deref() != Some(file_id))
    {
        debug!(
            file = %location,
            "incremental graph rebuild cannot reuse prior file: leaf parent does not match current file id"
        );
        return None;
    }

    if snapshot
        .leaves
        .iter()
        .any(|leaf| leaf.file_hash_at_capture.as_ref() != Some(new_hash))
    {
        debug!(
            file = %location,
            "incremental graph rebuild treating unchanged-path candidate as changed: prior leaf file hash does not match new hash"
        );
        return None;
    }

    Some(snapshot.clone())
}

fn reuse_prior_file(ctx: &mut PipelineContext, file_idx: usize, snapshot: PriorFileSnapshot) {
    let file = &mut ctx.graph.files[file_idx];
    file.source_blob_hash = snapshot.source_blob_hash;
    file.source = snapshot.source;
    file.imports = snapshot.imports;
    file.exports = snapshot.exports;
    file.re_exports = snapshot.re_exports;
    file.leaf_children = snapshot.leaf_children;
    ctx.graph.leaves.extend(snapshot.leaves);
}

fn apply_extracted_file(ctx: &mut PipelineContext, file_idx: usize, extracted: ExtractedFile) {
    let file = &mut ctx.graph.files[file_idx];
    file.source_blob_hash = Some(extracted.source_blob_hash);
    file.source = extracted.source;
    file.exports = extracted.exports;
    file.re_exports = extracted.re_exports;
    file.leaf_children = extracted.leaf_children;
    ctx.graph.leaves.extend(extracted.leaves);
}

fn parse_leaf_kind(s: &str, depth: Option<u8>) -> LeafKind {
    match s {
        "function" => LeafKind::Function,
        "function_declaration" => LeafKind::FunctionDeclaration,
        "method" => LeafKind::Method,
        "singleton_method" => LeafKind::SingletonMethod,
        "class" => LeafKind::Class,
        "singleton_class" => LeafKind::SingletonClass,
        "package" => LeafKind::Package,
        "object" => LeafKind::Object,
        "companion_object" => LeafKind::CompanionObject,
        "namespace" => LeafKind::Namespace,
        "enum" => LeafKind::Enum,
        "struct" => LeafKind::Struct,
        "record" => LeafKind::Record,
        "interface" => LeafKind::Interface,
        "type_alias" => LeafKind::TypeAlias,
        "trait" => LeafKind::Trait,
        "impl" => LeafKind::Impl,
        "property" => LeafKind::Property,
        "field" => LeafKind::Field,
        "event" => LeafKind::Event,
        "delegate" => LeafKind::Delegate,
        "global" => LeafKind::Global,
        "macro" => LeafKind::Macro,
        "constant" => LeafKind::Constant,
        "module" => LeafKind::Module,
        "section" => LeafKind::Section {
            depth: depth.unwrap_or(1),
        },
        "config_key" => LeafKind::ConfigKey,
        "column" => LeafKind::Column,
        _ => LeafKind::Function,
    }
}

fn file_exports(exports: &[extract::ExtractedExport]) -> (Vec<String>, Vec<ReExport>) {
    let mut names = BTreeSet::new();
    let mut re_exports = Vec::new();

    for export in exports {
        if export.name.is_empty() {
            continue;
        }
        names.insert(export.name.clone());
        if let Some(source_path) = export.source_path.as_ref()
            && !source_path.is_empty()
        {
            re_exports.push(ReExport {
                name: export.name.clone(),
                source_path: source_path.clone(),
            });
        }
    }

    re_exports.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source_path.cmp(&right.source_path))
    });
    re_exports
        .dedup_by(|left, right| left.name == right.name && left.source_path == right.source_path);

    (names.into_iter().collect(), re_exports)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::*;
    use crate::graph::object_store::RefName;
    use crate::pipeline::context::BuildConfig;
    use crate::pipeline::hash;

    #[test]
    fn incremental_leaf_build_reuses_unchanged_files_and_extracts_changed_files() {
        let fixture = IncrementalFixture::new();
        fixture.write_file("unchanged.rs", "pub fn reused_symbol() -> u8 { 1 }\n");
        fixture.write_file("changed.rs", "pub fn fresh_symbol() -> u8 { 1 }\n");

        let mut prior = fixture.build_context(false, &[]);
        let prior_reused_hash = leaf_by_name(&prior, "reused_symbol").source_hash.clone();
        let prior_fresh_hash = leaf_by_name(&prior, "fresh_symbol").source_hash.clone();
        file_by_location_mut(&mut prior, "unchanged.rs")
            .exports
            .push("synthetic_prior_export".to_string());
        fixture.persist_ref(&prior);

        fixture.write_file("changed.rs", "pub fn fresh_symbol() -> u8 { 2 }\n");
        let incremental = fixture.build_context(true, &["changed.rs"]);

        let reused_leaf = leaf_by_name(&incremental, "reused_symbol");
        assert_eq!(reused_leaf.source_hash, prior_reused_hash);
        assert_eq!(
            reused_leaf.file_hash_at_capture.as_ref(),
            incremental.new_hashes.get("unchanged.rs")
        );
        assert!(
            file_by_location(&incremental, "unchanged.rs")
                .exports
                .contains(&"synthetic_prior_export".to_string())
        );

        let fresh_leaf = leaf_by_name(&incremental, "fresh_symbol");
        assert_ne!(fresh_leaf.source_hash, prior_fresh_hash);
        assert!(
            !file_by_location(&incremental, "changed.rs")
                .exports
                .contains(&"synthetic_prior_export".to_string())
        );
    }

    #[test]
    fn non_incremental_leaf_build_extracts_even_when_prior_graph_exists() {
        let fixture = IncrementalFixture::new();
        fixture.write_file("unchanged.rs", "pub fn reused_symbol() -> u8 { 1 }\n");
        fixture.write_file("changed.rs", "pub fn fresh_symbol() -> u8 { 1 }\n");

        let mut prior = fixture.build_context(false, &[]);
        file_by_location_mut(&mut prior, "unchanged.rs")
            .exports
            .push("synthetic_prior_export".to_string());
        fixture.persist_ref(&prior);

        let rebuilt = fixture.build_context(false, &[]);

        assert_eq!(
            leaf_by_name(&rebuilt, "reused_symbol").base.location,
            "unchanged.rs#reused_symbol"
        );
        assert!(
            !file_by_location(&rebuilt, "unchanged.rs")
                .exports
                .contains(&"synthetic_prior_export".to_string())
        );
    }

    #[test]
    fn incremental_leaf_build_falls_back_to_full_extract_when_prior_graph_is_absent() {
        let fixture = IncrementalFixture::new();
        fixture.write_file("only.rs", "pub fn extracted_without_prior() {}\n");

        let rebuilt = fixture.build_context(true, &[]);

        assert_eq!(
            leaf_by_name(&rebuilt, "extracted_without_prior")
                .base
                .location,
            "only.rs#extracted_without_prior"
        );
    }

    #[test]
    fn leaf_build_skips_missing_files_without_error() {
        let fixture = IncrementalFixture::new();
        fixture.write_file("readable.rs", "pub fn readable_symbol() {}\n");

        let rebuilt = fixture.build_context_with_paths(
            false,
            &[],
            vec![PathBuf::from("missing.rs"), PathBuf::from("readable.rs")],
        );

        assert_eq!(
            leaf_by_name(&rebuilt, "readable_symbol").base.location,
            "readable.rs#readable_symbol"
        );
        assert!(
            file_by_location(&rebuilt, "missing.rs")
                .leaf_children
                .is_empty()
        );
        assert!(
            !rebuilt
                .graph
                .leaves
                .iter()
                .any(|leaf| leaf.base.location.starts_with("missing.rs#"))
        );
    }

    struct IncrementalFixture {
        repo: TempDir,
        knowledge: TempDir,
        ref_name: RefName,
    }

    impl IncrementalFixture {
        fn new() -> Self {
            Self {
                repo: TempDir::new().expect("create repo tempdir"),
                knowledge: TempDir::new().expect("create knowledge tempdir"),
                ref_name: RefName::new("main").expect("valid ref name"),
            }
        }

        fn write_file(&self, rel: &str, content: &str) {
            let path = self.repo.path().join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent dir");
            }
            fs::write(path, content).expect("write fixture file");
        }

        fn build_context(&self, incremental: bool, changed_paths: &[&str]) -> PipelineContext {
            self.build_context_with_paths(
                incremental,
                changed_paths,
                rust_file_paths(self.repo.path()),
            )
        }

        fn build_context_with_paths(
            &self,
            incremental: bool,
            changed_paths: &[&str],
            file_paths: Vec<PathBuf>,
        ) -> PipelineContext {
            let config = BuildConfig {
                repo_path: self.repo.path().to_path_buf(),
                output_dir: self.knowledge.path().to_path_buf(),
                incremental,
                ref_name: Some(self.ref_name.clone()),
            };
            let mut ctx = PipelineContext::new(config, self.ref_name.clone(), None);
            ctx.file_paths = file_paths;
            hash::compute_hashes(&mut ctx).expect("compute hashes");
            ctx.changed_paths = changed_paths
                .iter()
                .map(|path| (*path).to_string())
                .collect();

            build_graph_dirs(&mut ctx).expect("build dirs");
            build_graph_files(&mut ctx).expect("build files");
            build_graph_leaves(&mut ctx).expect("build leaves");
            ctx
        }

        fn persist_ref(&self, ctx: &PipelineContext) {
            let store = GraphObjectStore::new(ctx.graph_dir());
            store
                .prepare_refs_layout(ctx.default_ref_name.as_ref())
                .expect("prepare refs");
            let current_ref = store.write_graph(&ctx.graph).expect("write graph");
            store
                .write_ref_atomic(&ctx.ref_name, &current_ref)
                .expect("write ref");
        }
    }

    fn rust_file_paths(repo: &Path) -> Vec<PathBuf> {
        let mut paths = fs::read_dir(repo)
            .expect("read fixture repo")
            .map(|entry| entry.expect("read dir entry").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
            .map(|path| {
                path.strip_prefix(repo)
                    .expect("fixture path under repo")
                    .to_path_buf()
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    fn leaf_by_name<'a>(ctx: &'a PipelineContext, name: &str) -> &'a LeafNode {
        ctx.graph
            .leaves
            .iter()
            .find(|leaf| leaf.base.name == name)
            .unwrap_or_else(|| panic!("missing leaf {name}"))
    }

    fn file_by_location<'a>(ctx: &'a PipelineContext, location: &str) -> &'a FileNode {
        ctx.graph
            .files
            .iter()
            .find(|file| file.base.location == location)
            .unwrap_or_else(|| panic!("missing file {location}"))
    }

    fn file_by_location_mut<'a>(ctx: &'a mut PipelineContext, location: &str) -> &'a mut FileNode {
        ctx.graph
            .files
            .iter_mut()
            .find(|file| file.base.location == location)
            .unwrap_or_else(|| panic!("missing file {location}"))
    }
}
