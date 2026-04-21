use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use crate::error::KnowledgeError;
use crate::extract::{self, Language, identity_key, leaf_location, node_id};
use crate::graph::nodes::{BaseNodeFields, DirNode, FileNode, LeafKind, LeafNode};
use crate::pipeline::context::PipelineContext;

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
                task_ids: Vec::new(),
                structural_conflict: false,
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
            .and_then(Language::from_extension)
            .map(|l| l.as_str().to_string())
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
                task_ids: Vec::new(),
                structural_conflict: false,
            },
            extension,
            source_blob_hash: None,
            imports: Vec::new(),
            exports: Vec::new(),
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

/// Extract leaf nodes from source files using tree-sitter extractors.
pub fn build_graph_leaves(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    let registry = extract::ExtractorRegistry::new();

    // Collect file indices to process (we need to mutate ctx.graph but iterate files)
    let file_infos: Vec<(usize, String, String)> = ctx
        .graph
        .files
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let ext = f.extension.as_deref()?;
            Language::from_extension(ext)?;
            Some((i, f.base.location.clone(), f.base.id.clone()))
        })
        .collect();

    for (file_idx, location, file_id) in file_infos {
        let abs = ctx.repo_path.join(&location);
        let content = match fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let ext = ctx.graph.files[file_idx].extension.as_deref().unwrap_or("");
        let language = match Language::from_extension(ext) {
            Some(l) => l,
            None => continue,
        };

        let extractor = match registry.get(language) {
            Some(e) => e,
            None => continue,
        };

        let result = extractor.extract(&content);
        let source_hash = extract::compute_source_hash(&content);

        // Set file source_blob_hash (will be written to blob by persist)
        ctx.graph.files[file_idx].source_blob_hash = Some(source_hash);

        let mut leaf_ids = Vec::new();

        for extracted in &result.leaves {
            let loc = leaf_location(&location, &extracted.qualified_name);
            let id = node_id("symbol", &loc, &extracted.kind);
            let ikey = identity_key("symbol", &loc, &extracted.kind);

            let kind = parse_leaf_kind(&extracted.kind);

            let leaf = LeafNode {
                base: BaseNodeFields {
                    id: id.clone(),
                    identity_key: ikey,
                    object_hash: None,
                    name: extracted.name.clone(),
                    location: loc,
                    language: language.as_str().to_string(),
                    description: String::new(),
                    parent_id: Some(file_id.clone()),
                    is_locked: false,
                    lineage_locked: false,
                    lock_owner: None,
                    lock_reason: String::new(),
                    task_ids: Vec::new(),
                    structural_conflict: false,
                },
                kind,
                source: extracted.source.clone(),
                source_blob_hash: None,
                source_hash: Some(extracted.source_hash.clone()),
                file_hash_at_capture: ctx.new_hashes.get(&location).cloned(),
                history: Vec::new(),
                input_signature: Vec::new(),
                output_signature: Vec::new(),
                start_line: Some(extracted.start_line as u32),
                end_line: Some(extracted.end_line as u32),
                children: extracted
                    .children_qualified_names
                    .iter()
                    .map(|qn| {
                        let child_loc = leaf_location(&location, qn);
                        node_id("symbol", &child_loc, "method")
                    })
                    .collect(),
            };

            leaf_ids.push(id);
            ctx.graph.leaves.push(leaf);
        }

        ctx.graph.files[file_idx].leaf_children = leaf_ids;
    }

    Ok(())
}

fn parse_leaf_kind(s: &str) -> LeafKind {
    match s {
        "function" => LeafKind::Function,
        "method" => LeafKind::Method,
        "class" => LeafKind::Class,
        "struct" => LeafKind::Struct,
        "interface" => LeafKind::Interface,
        "trait" => LeafKind::Trait,
        "impl" => LeafKind::Impl,
        "field" => LeafKind::Field,
        "module" => LeafKind::Module,
        _ => LeafKind::Function,
    }
}
