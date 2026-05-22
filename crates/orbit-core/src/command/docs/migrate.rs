use std::path::{Path, PathBuf};

use serde_json::json;

use orbit_common::types::OrbitError;

use super::frontmatter::{infer_frontmatter, parse_doc_strict, split_frontmatter};
use super::path_util::{path_to_slash_string, repo_relative_path};
use super::types::{DocFrontmatter, DocMigrationChange, DocMigrationReport};
use super::walk::path_is_or_contains_dot_orbit; // note: will make pub(super) in walk if needed

pub(super) fn migrate_docs(
    repo_root: &Path,
    dry_run: bool,
) -> Result<DocMigrationReport, OrbitError> {
    let mut candidates = Vec::new();
    collect_migration_candidates(&repo_root.join("docs/design"), 2, &mut candidates)?;
    collect_migration_candidates(&repo_root.join("docs/design-patterns"), 1, &mut candidates)?;
    candidates.sort();
    let mut changed = Vec::new();
    for path in candidates {
        if path_is_or_contains_dot_orbit(repo_root, &path) {
            continue;
        }
        let relative = repo_relative_path(repo_root, &path)?;
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
        let Some(updated) = migrate_doc_content(&relative, &path, &raw)? else {
            continue;
        };
        let diff = migration_diff(&path_to_slash_string(&relative), &raw, &updated);
        if !dry_run {
            std::fs::write(&path, &updated)
                .map_err(|error| OrbitError::Io(format!("write {}: {error}", path.display())))?;
        }
        changed.push(DocMigrationChange {
            path: path_to_slash_string(&relative),
            diff,
        });
    }
    Ok(DocMigrationReport { dry_run, changed })
}

fn collect_migration_candidates(
    root: &Path,
    relative_depth: usize,
    out: &mut Vec<PathBuf>,
) -> Result<(), OrbitError> {
    if !root.exists() {
        return Ok(());
    }
    fn rec(
        root: &Path,
        dir: &Path,
        relative_depth: usize,
        out: &mut Vec<PathBuf>,
    ) -> Result<(), OrbitError> {
        let mut entries = std::fs::read_dir(dir)
            .map_err(|error| OrbitError::Io(format!("read {}: {error}", dir.display())))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| OrbitError::Io(error.to_string()))?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| OrbitError::Io(error.to_string()))?;
            if file_type.is_dir() {
                rec(root, &path, relative_depth, out)?;
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("md") {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            if relative.components().count() == relative_depth {
                out.push(path);
            }
        }
        Ok(())
    }
    rec(root, root, relative_depth, out)
}

fn migrate_doc_content(
    relative: &Path,
    path: &Path,
    raw: &str,
) -> Result<Option<String>, OrbitError> {
    if parse_doc_strict(path, raw).is_ok() {
        return Ok(None);
    }
    let block = split_frontmatter(raw).map_err(|message| {
        OrbitError::InvalidInput(format!(
            "invalid frontmatter in {}: {message}",
            path.display()
        ))
    })?;
    let body = block.as_ref().map(|block| block.body).unwrap_or(raw);
    let inferred = infer_frontmatter(relative, body);
    let updated = match block {
        Some(block) => update_existing_frontmatter(block.raw, body, &inferred)?,
        None => {
            let mut output = render_frontmatter_block(&inferred);
            output.push_str(raw);
            output
        }
    };
    if updated == raw {
        return Ok(None);
    }
    Ok(Some(updated))
}

fn update_existing_frontmatter(
    existing: &str,
    body: &str,
    inferred: &DocFrontmatter,
) -> Result<String, OrbitError> {
    let mut value = if existing.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str::<serde_yaml::Value>(existing).map_err(|error| {
            OrbitError::InvalidInput(format!("invalid frontmatter YAML while migrating: {error}"))
        })?
    };
    if matches!(value, serde_yaml::Value::Null) {
        value = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    let serde_yaml::Value::Mapping(mapping) = &mut value else {
        return Err(OrbitError::InvalidInput(
            "frontmatter YAML must be a mapping to migrate".to_string(),
        ));
    };
    mapping.insert(
        serde_yaml::Value::String("type".to_string()),
        serde_yaml::Value::String(inferred.doc_type.as_str().to_string()),
    );
    mapping.insert(
        serde_yaml::Value::String("summary".to_string()),
        serde_yaml::Value::String(inferred.summary.clone()),
    );
    let tags_key = serde_yaml::Value::String("tags".to_string());
    if !inferred.tags.is_empty() && !mapping.contains_key(&tags_key) {
        mapping.insert(
            tags_key,
            serde_yaml::Value::Sequence(
                inferred
                    .tags
                    .iter()
                    .cloned()
                    .map(serde_yaml::Value::String)
                    .collect(),
            ),
        );
    }
    let mut rendered = serde_yaml::to_string(&value)
        .map_err(|error| OrbitError::Execution(format!("serialize frontmatter YAML: {error}")))?;
    if let Some(stripped) = rendered.strip_prefix("---\n") {
        rendered = stripped.to_string();
    }
    if let Some(stripped) = rendered.strip_suffix("...\n") {
        rendered = stripped.to_string();
    }
    let mut output = String::from("---\n");
    output.push_str(&rendered);
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("---\n");
    output.push_str(body);
    Ok(output)
}

fn render_frontmatter_block(frontmatter: &DocFrontmatter) -> String {
    let mut output = String::from("---\n");
    output.push_str(&format!("type: {}\n", frontmatter.doc_type));
    output.push_str(&format!(
        "summary: {}\n",
        yaml_inline_string(&frontmatter.summary)
    ));
    if !frontmatter.tags.is_empty() {
        output.push_str(&format!("tags: {}\n", json!(frontmatter.tags)));
    }
    if !frontmatter.paths.is_empty() {
        output.push_str(&format!("paths: {}\n", json!(frontmatter.paths)));
    }
    if !frontmatter.related_features.is_empty() {
        output.push_str(&format!(
            "related_features: {}\n",
            json!(frontmatter.related_features)
        ));
    }
    if !frontmatter.related_artifacts.is_empty() {
        output.push_str(&format!(
            "related_artifacts: {}\n",
            json!(frontmatter.related_artifacts)
        ));
    }
    output.push_str("---\n");
    output
}

fn yaml_inline_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn migration_diff(path: &str, before: &str, after: &str) -> String {
    let before_lines = diff_lines(before);
    let after_lines = diff_lines(after);
    let old_start = if before_lines.is_empty() { 0 } else { 1 };
    let new_start = if after_lines.is_empty() { 0 } else { 1 };
    let mut output = format!(
        "--- {path}\n+++ {path}\n@@ -{old_start},{} +{new_start},{} @@\n",
        before_lines.len(),
        after_lines.len()
    );
    for op in line_diff(&before_lines, &after_lines) {
        match op {
            DiffOp::Equal(line) => push_diff_line(&mut output, ' ', line),
            DiffOp::Delete(line) => push_diff_line(&mut output, '-', line),
            DiffOp::Insert(line) => push_diff_line(&mut output, '+', line),
        }
    }
    output
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DiffLine<'a> {
    text: &'a str,
    has_newline: bool,
}

enum DiffOp<'a> {
    Equal(DiffLine<'a>),
    Delete(DiffLine<'a>),
    Insert(DiffLine<'a>),
}

fn diff_lines(raw: &str) -> Vec<DiffLine<'_>> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split_inclusive('\n')
        .map(|line| match line.strip_suffix('\n') {
            Some(without_newline) => DiffLine {
                text: without_newline
                    .strip_suffix('\r')
                    .unwrap_or(without_newline),
                has_newline: true,
            },
            None => DiffLine {
                text: line,
                has_newline: false,
            },
        })
        .collect()
}

fn line_diff<'a>(before: &[DiffLine<'a>], after: &[DiffLine<'a>]) -> Vec<DiffOp<'a>> {
    let mut lcs = vec![vec![0usize; after.len() + 1]; before.len() + 1];
    for before_index in (0..before.len()).rev() {
        for after_index in (0..after.len()).rev() {
            lcs[before_index][after_index] = if before[before_index] == after[after_index] {
                lcs[before_index + 1][after_index + 1] + 1
            } else {
                lcs[before_index + 1][after_index].max(lcs[before_index][after_index + 1])
            };
        }
    }
    let mut ops = Vec::new();
    let mut before_index = 0;
    let mut after_index = 0;
    while before_index < before.len() && after_index < after.len() {
        if before[before_index] == after[after_index] {
            ops.push(DiffOp::Equal(before[before_index]));
            before_index += 1;
            after_index += 1;
        } else if lcs[before_index + 1][after_index] >= lcs[before_index][after_index + 1] {
            ops.push(DiffOp::Delete(before[before_index]));
            before_index += 1;
        } else {
            ops.push(DiffOp::Insert(after[after_index]));
            after_index += 1;
        }
    }
    while before_index < before.len() {
        ops.push(DiffOp::Delete(before[before_index]));
        before_index += 1;
    }
    while after_index < after.len() {
        ops.push(DiffOp::Insert(after[after_index]));
        after_index += 1;
    }
    ops
}

fn push_diff_line(output: &mut String, prefix: char, line: DiffLine<'_>) {
    output.push(prefix);
    output.push_str(line.text);
    output.push('\n');
    if !line.has_newline {
        output.push_str("\\ No newline at end of file\n");
    }
}
