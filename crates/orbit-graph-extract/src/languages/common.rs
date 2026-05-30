//! Shared helpers for language extractors (dedup, path normalize, string filter).
//! Moved from rust.rs to avoid intra-crate duplication >50 LOC across c/markdown/config.
//! See task ORB-00305 comments for rationale.

use std::path::Path;

use crate::{RawImport, RawRef, RawRelation, RawSymbol};

pub(crate) fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Filter for RawString / notable strings per spec §6.2: len>=6, not all ASCII punct, not pure format.
pub(crate) fn is_notable_string(value: &str) -> bool {
    if value.len() < 6 {
        return false;
    }
    let all_punct = value
        .chars()
        .all(|c| c.is_ascii_punctuation() || c.is_whitespace());
    if all_punct {
        return false;
    }
    // crude "pure format string" heuristic: contains {} or %s/%d etc but no letters outside
    let has_letters = value.chars().any(|c| c.is_ascii_alphabetic());
    if !has_letters && (value.contains("{}") || value.contains("%s") || value.contains("%d")) {
        return false;
    }
    true
}

pub(crate) fn dedup_symbols(symbols: &mut Vec<RawSymbol>) {
    symbols.sort_by(|left, right| {
        left.span_start
            .cmp(&right.span_start)
            .then_with(|| left.span_end.cmp(&right.span_end))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.qualified.cmp(&right.qualified))
    });
    symbols.dedup_by(|left, right| {
        left.file_path == right.file_path
            && left.qualified == right.qualified
            && left.kind == right.kind
            && left.span_start == right.span_start
            && left.span_end == right.span_end
    });
}

pub(crate) fn dedup_refs(refs: &mut Vec<RawRef>) {
    refs.sort_by(|left, right| {
        left.from_span_start
            .cmp(&right.from_span_start)
            .then_with(|| left.from_span_end.cmp(&right.from_span_end))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.target_name.cmp(&right.target_name))
    });
    refs.dedup_by(|left, right| {
        left.from_file == right.from_file
            && left.from_span_start == right.from_span_start
            && left.from_span_end == right.from_span_end
            && left.target_name == right.target_name
            && left.target_qualified == right.target_qualified
            && left.kind == right.kind
            && left.confidence == right.confidence
    });
}

pub(crate) fn dedup_relations(relations: &mut Vec<RawRelation>) {
    relations.sort_by(|left, right| {
        left.def_span_start
            .cmp(&right.def_span_start)
            .then_with(|| left.from_qualified.cmp(&right.from_qualified))
            .then_with(|| left.to_qualified.cmp(&right.to_qualified))
    });
    relations.dedup_by(|left, right| {
        left.from_qualified == right.from_qualified
            && left.to_qualified == right.to_qualified
            && left.kind == right.kind
            && left.def_file == right.def_file
            && left.def_span_start == right.def_span_start
            && left.def_span_end == right.def_span_end
    });
}

pub(crate) fn dedup_imports(imports: &mut Vec<RawImport>) {
    imports.sort_by(|left, right| {
        left.target_path
            .cmp(&right.target_path)
            .then_with(|| left.target_symbol.cmp(&right.target_symbol))
    });
    imports.dedup_by(|left, right| {
        left.from_file == right.from_file
            && left.target_path == right.target_path
            && left.target_symbol == right.target_symbol
    });
}
