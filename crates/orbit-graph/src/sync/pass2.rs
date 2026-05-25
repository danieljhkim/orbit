//! Pass 2 resolves raw refs after Pass 1 has written files, symbols, and imports.
//!
//! Resolution deliberately follows the documented confidence ladder in strict
//! order: same-file exact matches, explicit imports, same-module matches, then
//! fuzzy name-only refs. All refs for the files refreshed by the current sync
//! are rewritten in one SQLite transaction; unchanged files' refs are not
//! touched during incremental syncs.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use orbit_graph_extract::RawRef;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use super::pass1::ExtractedFileRefs;
use super::scanner::DbLockGuard;
use crate::{GraphError, SyncMode};

const CONFIDENCE_EXACT: &str = "exact";
const CONFIDENCE_IMPORT_RESOLVED: &str = "import_resolved";
const CONFIDENCE_SAME_MODULE: &str = "same_module";
const CONFIDENCE_FUZZY_NAME: &str = "fuzzy_name";

pub(crate) fn run(
    db_path: &Path,
    mode: SyncMode,
    refs_by_file: Vec<ExtractedFileRefs>,
) -> Result<(), GraphError> {
    let _lock = DbLockGuard::acquire(db_path)?;
    let mut conn = open_writer_connection(db_path)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|source| GraphError::sqlite("begin pass2 refs transaction", source))?;

    for file_refs in refs_by_file {
        delete_refs_for_file(&tx, &file_refs.file_path)?;
        for raw_ref in &file_refs.refs {
            let resolved = resolve_ref(&tx, &file_refs.file_path, raw_ref)?;
            insert_ref(&tx, &file_refs.file_path, raw_ref, &resolved)?;
        }
    }

    update_sync_meta(&tx, mode)?;
    tx.commit()
        .map_err(|source| GraphError::sqlite("commit pass2 refs transaction", source))?;
    Ok(())
}

fn open_writer_connection(db_path: &Path) -> Result<Connection, GraphError> {
    let conn = Connection::open(db_path)
        .map_err(|source| GraphError::sqlite("open graph database for pass2 writes", source))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|source| GraphError::sqlite("enable foreign keys for pass2 writes", source))?;
    Ok(conn)
}

fn delete_refs_for_file(tx: &Transaction<'_>, from_file: &str) -> Result<(), GraphError> {
    tx.prepare_cached("DELETE FROM refs WHERE from_file = ?1")
        .map_err(|source| GraphError::sqlite("prepare pass2 ref delete", source))?
        .execute(params![from_file])
        .map_err(|source| GraphError::sqlite("delete prior refs for file", source))?;
    Ok(())
}

fn insert_ref(
    tx: &Transaction<'_>,
    from_file: &str,
    raw_ref: &RawRef,
    resolved: &ResolvedRef,
) -> Result<(), GraphError> {
    let target_symbol_hint = match resolved.target_qualified.as_deref() {
        Some(qualified) => unique_symbol_id_for_qualified(tx, qualified)?,
        None => None,
    };
    tx.prepare_cached(
        "INSERT INTO refs (
            from_file, from_span_start, from_span_end, target_name, target_qualified,
            target_symbol_hint, kind, confidence
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .map_err(|source| GraphError::sqlite("prepare pass2 ref insert", source))?
    .execute(params![
        from_file,
        usize_to_i64("convert ref span start", raw_ref.from_span_start)?,
        usize_to_i64("convert ref span end", raw_ref.from_span_end)?,
        raw_ref.target_name,
        resolved.target_qualified,
        target_symbol_hint,
        raw_ref.kind,
        resolved.confidence,
    ])
    .map_err(|source| GraphError::sqlite("insert resolved ref row", source))?;
    Ok(())
}

fn resolve_ref(
    tx: &Transaction<'_>,
    from_file: &str,
    raw_ref: &RawRef,
) -> Result<ResolvedRef, GraphError> {
    if let Some(target_qualified) = resolve_exact(tx, from_file, raw_ref)? {
        return Ok(ResolvedRef::qualified(target_qualified, CONFIDENCE_EXACT));
    }
    if let Some(target_qualified) = resolve_import(tx, from_file, raw_ref)? {
        return Ok(ResolvedRef::qualified(
            target_qualified,
            CONFIDENCE_IMPORT_RESOLVED,
        ));
    }
    if let Some(target_qualified) = resolve_same_module(tx, from_file, raw_ref)? {
        return Ok(ResolvedRef::qualified(
            target_qualified,
            CONFIDENCE_SAME_MODULE,
        ));
    }
    Ok(ResolvedRef {
        target_qualified: None,
        confidence: CONFIDENCE_FUZZY_NAME,
    })
}

fn resolve_exact(
    tx: &Transaction<'_>,
    from_file: &str,
    raw_ref: &RawRef,
) -> Result<Option<String>, GraphError> {
    let candidates = symbols_in_file_by_name(tx, from_file, &raw_ref.target_name)?;
    if candidates.is_empty() {
        return Ok(None);
    }

    if let Some(target_qualified) = raw_ref.target_qualified.as_deref() {
        let qualified_matches = candidates
            .iter()
            .filter(|candidate| candidate.qualified == target_qualified)
            .cloned()
            .collect::<Vec<_>>();
        if let Some(qualified) = unique_candidate_qualified(&qualified_matches) {
            return Ok(Some(qualified));
        }
    }

    Ok(unique_candidate_qualified(&candidates))
}

fn resolve_import(
    tx: &Transaction<'_>,
    from_file: &str,
    raw_ref: &RawRef,
) -> Result<Option<String>, GraphError> {
    let imports = imports_for_file(tx, from_file)?;
    for explicit_only in [true, false] {
        for import in imports
            .iter()
            .filter(|import| import.target_symbol.is_some() == explicit_only)
        {
            let Some(imported_name) = import
                .target_symbol
                .as_deref()
                .or(Some(raw_ref.target_name.as_str()))
            else {
                continue;
            };
            if import.target_symbol.is_some() && imported_name != raw_ref.target_name {
                continue;
            }

            let candidates = symbols_by_name(tx, imported_name)?
                .into_iter()
                .filter(|candidate| {
                    qualified_matches_import(
                        &candidate.qualified,
                        &import.target_path,
                        imported_name,
                    )
                })
                .collect::<Vec<_>>();
            if let Some(qualified) = unique_distinct_qualified(&candidates) {
                return Ok(Some(qualified));
            }
        }
    }

    Ok(None)
}

fn resolve_same_module(
    tx: &Transaction<'_>,
    from_file: &str,
    raw_ref: &RawRef,
) -> Result<Option<String>, GraphError> {
    let prefixes = module_prefixes_for_file(tx, from_file)?;
    if prefixes.is_empty() {
        return Ok(None);
    }

    let candidates = symbols_by_name(tx, &raw_ref.target_name)?
        .into_iter()
        .filter(|candidate| candidate.file_path != from_file)
        .filter(|candidate| {
            qualified_prefix_before_name(&candidate.qualified, &candidate.name)
                .is_some_and(|prefix| prefixes.contains(prefix.as_str()))
        })
        .collect::<Vec<_>>();

    Ok(unique_candidate_qualified(&candidates))
}

fn symbols_in_file_by_name(
    tx: &Transaction<'_>,
    from_file: &str,
    name: &str,
) -> Result<Vec<SymbolCandidate>, GraphError> {
    let mut stmt = tx
        .prepare_cached(
            "SELECT id, file_path, name, qualified FROM symbols
             WHERE file_path = ?1 AND name = ?2
             ORDER BY qualified, id",
        )
        .map_err(|source| GraphError::sqlite("prepare exact symbol lookup", source))?;
    let rows = stmt
        .query_map(params![from_file, name], symbol_candidate_from_row)
        .map_err(|source| GraphError::sqlite("query symbols for ref resolution", source))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|source| GraphError::sqlite("collect symbols for ref resolution", source))
}

fn symbols_by_name(tx: &Transaction<'_>, name: &str) -> Result<Vec<SymbolCandidate>, GraphError> {
    let mut stmt = tx
        .prepare_cached(
            "SELECT id, file_path, name, qualified FROM symbols
             WHERE name = ?1
             ORDER BY qualified, id",
        )
        .map_err(|source| GraphError::sqlite("prepare name symbol lookup", source))?;
    let rows = stmt
        .query_map(params![name], symbol_candidate_from_row)
        .map_err(|source| GraphError::sqlite("query symbols for ref resolution", source))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|source| GraphError::sqlite("collect symbols for ref resolution", source))
}

fn module_prefixes_for_file(
    tx: &Transaction<'_>,
    from_file: &str,
) -> Result<BTreeSet<String>, GraphError> {
    let mut stmt = tx
        .prepare_cached(
            "SELECT id, file_path, name, qualified FROM symbols
             WHERE file_path = ?1
             ORDER BY qualified, id",
        )
        .map_err(|source| GraphError::sqlite("prepare module prefix symbol lookup", source))?;
    let rows = stmt
        .query_map(params![from_file], symbol_candidate_from_row)
        .map_err(|source| GraphError::sqlite("query symbols for ref resolution", source))?;
    let symbols = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| GraphError::sqlite("collect symbols for ref resolution", source))?;
    Ok(symbols
        .iter()
        .filter_map(|symbol| qualified_prefix_before_name(&symbol.qualified, &symbol.name))
        .collect())
}

fn imports_for_file(
    tx: &Transaction<'_>,
    from_file: &str,
) -> Result<Vec<ImportCandidate>, GraphError> {
    let mut stmt = tx
        .prepare_cached(
            "SELECT target_path, target_symbol FROM imports
             WHERE from_file = ?1
             ORDER BY target_symbol IS NULL, target_path, target_symbol",
        )
        .map_err(|source| GraphError::sqlite("prepare import lookup", source))?;
    let rows = stmt
        .query_map(params![from_file], |row| {
            Ok(ImportCandidate {
                target_path: row.get(0)?,
                target_symbol: row.get(1)?,
            })
        })
        .map_err(|source| GraphError::sqlite("query imports for ref resolution", source))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|source| GraphError::sqlite("collect imports for ref resolution", source))
}

fn unique_symbol_id_for_qualified(
    tx: &Transaction<'_>,
    qualified: &str,
) -> Result<Option<i64>, GraphError> {
    let mut stmt = tx
        .prepare_cached("SELECT id FROM symbols WHERE qualified = ?1 ORDER BY id LIMIT 2")
        .map_err(|source| GraphError::sqlite("prepare target symbol hint lookup", source))?;
    let ids = stmt
        .query_map(params![qualified], |row| row.get::<_, i64>(0))
        .map_err(|source| GraphError::sqlite("query target symbol hint", source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| GraphError::sqlite("collect target symbol hint", source))?;
    if ids.len() == 1 {
        Ok(ids.first().copied())
    } else {
        Ok(None)
    }
}

fn symbol_candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolCandidate> {
    Ok(SymbolCandidate {
        file_path: row.get(1)?,
        name: row.get(2)?,
        qualified: row.get(3)?,
    })
}

fn unique_distinct_qualified(candidates: &[SymbolCandidate]) -> Option<String> {
    let qualified = candidates
        .iter()
        .map(|candidate| candidate.qualified.as_str())
        .collect::<BTreeSet<_>>();
    if qualified.len() == 1 {
        qualified.first().map(|value| (*value).to_string())
    } else {
        None
    }
}

fn unique_candidate_qualified(candidates: &[SymbolCandidate]) -> Option<String> {
    if candidates.len() == 1 {
        candidates
            .first()
            .map(|candidate| candidate.qualified.clone())
    } else {
        None
    }
}

fn qualified_matches_import(qualified: &str, target_path: &str, target_name: &str) -> bool {
    qualified_prefix_before_name(qualified, target_name)
        .is_some_and(|prefix| prefix == normalize_qualified_prefix(target_path))
}

fn qualified_prefix_before_name(qualified: &str, name: &str) -> Option<String> {
    if qualified == name {
        return Some(String::new());
    }
    let prefix = qualified.strip_suffix(name)?;
    let trimmed = normalize_qualified_prefix(prefix);
    if trimmed.len() == prefix.len() {
        None
    } else {
        Some(trimmed)
    }
}

fn normalize_qualified_prefix(prefix: &str) -> String {
    prefix
        .trim_end_matches(|ch: char| !is_qualified_word(ch))
        .to_string()
}

fn is_qualified_word(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-')
}

fn update_sync_meta(tx: &Transaction<'_>, mode: SyncMode) -> Result<(), GraphError> {
    let key = match mode {
        SyncMode::Auto => "last_incremental_at",
        SyncMode::Full => "last_full_build_at",
    };
    tx.prepare_cached("UPDATE meta SET value = ?1 WHERE key = ?2")
        .map_err(|source| GraphError::sqlite("prepare graph sync metadata update", source))?
        .execute(params![
            now_epoch_nanos("record sync timestamp")?.to_string(),
            key
        ])
        .map_err(|source| GraphError::sqlite("update graph sync metadata", source))?;
    Ok(())
}

fn now_epoch_nanos(operation: &'static str) -> Result<i64, GraphError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            GraphError::invalid_data(
                operation,
                format!("system time is before UNIX_EPOCH: {error}"),
            )
        })?;
    i64::try_from(duration.as_nanos())
        .map_err(|error| GraphError::invalid_data(operation, error.to_string()))
}

fn usize_to_i64(operation: &'static str, value: usize) -> Result<i64, GraphError> {
    i64::try_from(value).map_err(|error| GraphError::invalid_data(operation, error.to_string()))
}

#[derive(Debug, Clone)]
struct ResolvedRef {
    target_qualified: Option<String>,
    confidence: &'static str,
}

impl ResolvedRef {
    fn qualified(target_qualified: String, confidence: &'static str) -> Self {
        Self {
            target_qualified: Some(target_qualified),
            confidence,
        }
    }
}

#[derive(Debug, Clone)]
struct SymbolCandidate {
    file_path: String,
    name: String,
    qualified: String,
}

#[derive(Debug, Clone)]
struct ImportCandidate {
    target_path: String,
    target_symbol: Option<String>,
}

#[cfg(test)]
#[path = "tests/pass2.rs"]
mod tests;
