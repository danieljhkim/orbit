//! Pass 1 extracts files in parallel, then serializes SQLite writes.
//!
//! Extraction is CPU-bound and independent per file, so this pass uses rayon to
//! parse every new or modified file concurrently. SQLite remains a single-writer
//! boundary, so the extracted rows are collected in memory and written in a
//! deterministic serial loop. The collect-then-write strategy keeps transaction
//! ownership simple: each file has one immediate SQLite transaction that deletes
//! the prior row and inserts all Pass 1 rows, while `ExtractedFile::refs` stays
//! in memory for Pass 2 instead of being staged in the frozen schema.

use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orbit_graph_extract::{
    ExtractedFile, RawCommand, RawConfig, RawImport, RawRef, RawRelation, RawString, RawSymbol,
    languages,
};
use rayon::prelude::*;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use super::scanner::{Diff, mtime_ns, normalize_path};
use crate::{GraphError, SyncMode};

pub(crate) struct Pass1Output {
    pub(crate) refs: Vec<ExtractedFileRefs>,
    pub(crate) files_written: usize,
    pub(crate) files_removed: usize,
    pub(crate) files_indexed: usize,
}

pub(crate) struct ExtractedFileRefs {
    pub(crate) file_path: String,
    pub(crate) refs: Vec<RawRef>,
}

pub(crate) fn run(
    db_path: &Path,
    worktree_root: &Path,
    _mode: SyncMode,
    diff: &Diff,
) -> Result<Pass1Output, GraphError> {
    run_with_backend(
        db_path,
        worktree_root,
        _mode,
        diff,
        &DefaultExtractorBackend,
    )
}

fn run_with_backend(
    db_path: &Path,
    worktree_root: &Path,
    _mode: SyncMode,
    diff: &Diff,
    backend: &dyn ExtractorBackend,
) -> Result<Pass1Output, GraphError> {
    let changed = changed_files(diff);
    let mut extracted = extract_changed_files(worktree_root, &changed, backend);
    extracted.sort_by(|left, right| left.path.cmp(&right.path));

    let mut conn = open_writer_connection(db_path)?;

    let mut files_removed = 0;
    for rel_path in &diff.deleted {
        delete_file_transaction(&mut conn, rel_path)?;
        files_removed += 1;
    }

    let mut refs = Vec::new();
    let mut commands = Vec::new();
    let mut files_written = 0;
    for mut file in extracted {
        let file_refs = std::mem::take(&mut file.rows.refs);
        let file_commands = std::mem::take(&mut file.rows.commands);
        write_file_transaction(&mut conn, &file)?;
        refs.push(ExtractedFileRefs {
            file_path: file.file_path.clone(),
            refs: file_refs,
        });
        commands.extend(file_commands);
        files_written += 1;
    }
    insert_commands_transaction(&mut conn, &commands)?;

    let files_indexed = count_files(&conn)?;

    Ok(Pass1Output {
        refs,
        files_written,
        files_removed,
        files_indexed,
    })
}

fn changed_files(diff: &Diff) -> Vec<PathBuf> {
    let mut changed = Vec::with_capacity(diff.modified.len() + diff.new.len());
    changed.extend(diff.modified.iter().cloned());
    changed.extend(diff.new.iter().cloned());
    changed.sort();
    changed.dedup();
    changed
}

fn extract_changed_files(
    worktree_root: &Path,
    changed: &[PathBuf],
    backend: &dyn ExtractorBackend,
) -> Vec<ExtractedSourceFile> {
    let _panic_hook = SilentPanicHook::install();
    changed
        .par_iter()
        .filter_map(|rel_path| {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                backend.extract(worktree_root, rel_path)
            })) {
                Ok(Ok(file)) => Some(file),
                Ok(Err(error)) => {
                    warn_extraction_failure(rel_path, error.to_string());
                    None
                }
                Err(payload) => {
                    warn_extraction_failure(
                        rel_path,
                        format!(
                            "extractor panicked: {}",
                            panic_payload_message(payload.as_ref())
                        ),
                    );
                    None
                }
            }
        })
        .collect()
}

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

struct SilentPanicHook {
    previous: Option<PanicHook>,
    _guard: MutexGuard<'static, ()>,
}

impl SilentPanicHook {
    fn install() -> Self {
        // L-0049: catch_unwind still runs the panic hook before we convert extractor panics to warns.
        let guard = panic_hook_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        Self {
            previous: Some(previous),
            _guard: guard,
        }
    }
}

impl Drop for SilentPanicHook {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::panic::set_hook(previous);
        }
    }
}

fn panic_hook_lock() -> &'static Mutex<()> {
    static PANIC_HOOK_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    PANIC_HOOK_LOCK.get_or_init(|| Mutex::new(()))
}

fn warn_extraction_failure(path: &Path, error: String) {
    tracing::warn!(
        path = %path.display(),
        error = %error,
        "skipping file after graph extraction failure"
    );
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "unknown panic payload".to_string()
}

trait ExtractorBackend: Sync {
    fn extract(
        &self,
        worktree_root: &Path,
        rel_path: &Path,
    ) -> Result<ExtractedSourceFile, ExtractFileError>;
}

struct DefaultExtractorBackend;

impl ExtractorBackend for DefaultExtractorBackend {
    fn extract(
        &self,
        worktree_root: &Path,
        rel_path: &Path,
    ) -> Result<ExtractedSourceFile, ExtractFileError> {
        let path = worktree_root.join(rel_path);
        let bytes = fs::read(path.as_path()).map_err(|source| {
            ExtractFileError::new("read source file", format!("{}: {source}", path.display()))
        })?;
        let mtime_ns = mtime_ns(path.as_path())
            .map_err(|error| ExtractFileError::new("read source file mtime", error.to_string()))?;
        let extractors = languages::extractors();
        let extractor = extractors
            .iter()
            .find(|extractor| extractor.supports(rel_path))
            .ok_or_else(|| {
                ExtractFileError::new(
                    "select extractor",
                    format!("no registered extractor for {}", rel_path.display()),
                )
            })?;
        let rows = extractor.extract(rel_path, &bytes);
        Ok(ExtractedSourceFile {
            path: rel_path.to_path_buf(),
            file_path: normalize_path(rel_path),
            lang: extractor.lang(),
            content_hash: blake3::hash(&bytes).as_bytes().to_vec(),
            mtime_ns,
            byte_len: usize_to_i64("convert source byte length", bytes.len()).map_err(|error| {
                ExtractFileError::new("convert source byte length", error.to_string())
            })?,
            extracted_at: now_epoch_nanos("record extraction timestamp").map_err(|error| {
                ExtractFileError::new("record extraction timestamp", error.to_string())
            })?,
            rows,
        })
    }
}

#[derive(Debug)]
struct ExtractFileError {
    operation: &'static str,
    reason: String,
}

impl ExtractFileError {
    fn new(operation: &'static str, reason: impl Into<String>) -> Self {
        Self {
            operation,
            reason: reason.into(),
        }
    }
}

impl Display for ExtractFileError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.operation, self.reason)
    }
}

struct ExtractedSourceFile {
    path: PathBuf,
    file_path: String,
    lang: &'static str,
    content_hash: Vec<u8>,
    mtime_ns: i64,
    byte_len: i64,
    extracted_at: i64,
    rows: ExtractedFile,
}

fn open_writer_connection(db_path: &Path) -> Result<Connection, GraphError> {
    let conn = Connection::open(db_path)
        .map_err(|source| GraphError::sqlite("open graph database for pass1 writes", source))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|source| GraphError::sqlite("enable foreign keys for pass1 writes", source))?;
    Ok(conn)
}

fn delete_file_transaction(conn: &mut Connection, rel_path: &Path) -> Result<(), GraphError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|source| GraphError::sqlite("begin pass1 delete transaction", source))?;
    delete_fts_for_file(&tx, &normalize_path(rel_path))?;
    tx.execute(
        "DELETE FROM files WHERE path = ?1",
        params![normalize_path(rel_path)],
    )
    .map_err(|source| GraphError::sqlite("delete removed graph file", source))?;
    tx.commit()
        .map_err(|source| GraphError::sqlite("commit pass1 delete transaction", source))?;
    Ok(())
}

fn write_file_transaction(
    conn: &mut Connection,
    file: &ExtractedSourceFile,
) -> Result<(), GraphError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|source| GraphError::sqlite("begin pass1 file transaction", source))?;
    delete_fts_for_file(&tx, &file.file_path)?;
    tx.execute("DELETE FROM files WHERE path = ?1", params![file.file_path])
        .map_err(|source| GraphError::sqlite("delete prior graph file rows", source))?;
    tx.execute(
        "INSERT INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            file.file_path,
            file.content_hash,
            file.mtime_ns,
            file.lang,
            file.byte_len,
            file.extracted_at
        ],
    )
    .map_err(|source| GraphError::sqlite("insert graph file row", source))?;

    let symbol_ids = insert_symbols(&tx, &file.rows.symbols)?;
    insert_imports(&tx, &file.rows.imports)?;
    insert_relations(&tx, &file.rows.relations)?;
    insert_strings(&tx, &file.rows.strings, &symbol_ids)?;
    insert_configs(&tx, &file.rows.configs)?;

    tx.commit()
        .map_err(|source| GraphError::sqlite("commit pass1 file transaction", source))?;
    Ok(())
}

fn insert_commands_transaction(
    conn: &mut Connection,
    commands: &[RawCommand],
) -> Result<(), GraphError> {
    if commands.is_empty() {
        return Ok(());
    }

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|source| GraphError::sqlite("begin pass1 command transaction", source))?;
    insert_commands(&tx, commands)?;
    tx.commit()
        .map_err(|source| GraphError::sqlite("commit pass1 command transaction", source))?;
    Ok(())
}

fn insert_symbols(
    tx: &Transaction<'_>,
    symbols: &[RawSymbol],
) -> Result<BTreeMap<String, i64>, GraphError> {
    let mut symbol_ids = BTreeMap::new();
    for symbol in symbols {
        tx.execute(
            "INSERT INTO symbols (
                file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                symbol.file_path,
                symbol.name,
                symbol.qualified,
                symbol.kind,
                usize_to_i64("convert symbol span start", symbol.span_start)?,
                usize_to_i64("convert symbol span end", symbol.span_end)?,
                symbol.signature
            ],
        )
        .map_err(|source| GraphError::sqlite("insert graph symbol row", source))?;
        let symbol_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO symbols_fts (rowid, name, qualified, signature)
             VALUES (?1, ?2, ?3, ?4)",
            params![symbol_id, symbol.name, symbol.qualified, symbol.signature],
        )
        .map_err(|source| GraphError::sqlite("insert graph symbol fts row", source))?;
        symbol_ids.insert(symbol.qualified.clone(), symbol_id);
    }

    for symbol in symbols {
        let Some(parent_qualified) = symbol.parent_symbol.as_ref() else {
            continue;
        };
        let Some(parent_id) = symbol_ids.get(parent_qualified) else {
            continue;
        };
        let Some(symbol_id) = symbol_ids.get(&symbol.qualified) else {
            continue;
        };
        tx.execute(
            "UPDATE symbols SET parent_symbol = ?1 WHERE id = ?2",
            params![parent_id, symbol_id],
        )
        .map_err(|source| GraphError::sqlite("link graph symbol parent", source))?;
    }

    Ok(symbol_ids)
}

fn insert_imports(tx: &Transaction<'_>, imports: &[RawImport]) -> Result<(), GraphError> {
    for import in imports {
        tx.execute(
            "INSERT INTO imports (from_file, target_path, target_symbol)
             VALUES (?1, ?2, ?3)",
            params![import.from_file, import.target_path, import.target_symbol],
        )
        .map_err(|source| GraphError::sqlite("insert graph import row", source))?;
    }
    Ok(())
}

fn insert_relations(tx: &Transaction<'_>, relations: &[RawRelation]) -> Result<(), GraphError> {
    for relation in relations {
        tx.execute(
            "INSERT INTO relations (
                from_qualified, to_qualified, kind, def_file, def_span_start, def_span_end,
                confidence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                relation.from_qualified,
                relation.to_qualified,
                relation.kind,
                relation.def_file,
                usize_to_i64("convert relation span start", relation.def_span_start)?,
                usize_to_i64("convert relation span end", relation.def_span_end)?,
                relation.confidence
            ],
        )
        .map_err(|source| GraphError::sqlite("insert graph relation row", source))?;
    }
    Ok(())
}

fn insert_strings(
    tx: &Transaction<'_>,
    strings: &[RawString],
    symbol_ids: &BTreeMap<String, i64>,
) -> Result<(), GraphError> {
    for string in strings {
        let context_symbol = string
            .context_symbol
            .as_ref()
            .and_then(|qualified| symbol_ids.get(qualified))
            .copied();
        tx.execute(
            "INSERT INTO strings (file_path, line, value, context_symbol)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                string.file_path,
                usize_to_i64("convert string line", string.line)?,
                string.value,
                context_symbol
            ],
        )
        .map_err(|source| GraphError::sqlite("insert graph string row", source))?;
        let string_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO strings_fts (rowid, value) VALUES (?1, ?2)",
            params![string_id, string.value],
        )
        .map_err(|source| GraphError::sqlite("insert graph string fts row", source))?;
    }
    Ok(())
}

fn insert_configs(tx: &Transaction<'_>, configs: &[RawConfig]) -> Result<(), GraphError> {
    for config in configs {
        tx.execute(
            "INSERT INTO configs (file_path, line, key, kind)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                config.file_path,
                usize_to_i64("convert config line", config.line)?,
                config.key,
                config.kind
            ],
        )
        .map_err(|source| GraphError::sqlite("insert graph config row", source))?;
        let config_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO configs_fts (rowid, key) VALUES (?1, ?2)",
            params![config_id, config.key],
        )
        .map_err(|source| GraphError::sqlite("insert graph config fts row", source))?;
    }
    Ok(())
}

fn delete_fts_for_file(tx: &Transaction<'_>, file_path: &str) -> Result<(), GraphError> {
    // L-0056: cross-file command handler FKs must be cleared before refreshing symbol rows.
    tx.execute(
        "UPDATE commands
         SET handler_symbol = NULL
         WHERE handler_symbol IN (
             SELECT id FROM symbols WHERE file_path = ?1
         )",
        params![file_path],
    )
    .map_err(|source| GraphError::sqlite("clear command handlers for refreshed file", source))?;
    tx.execute(
        "DELETE FROM symbols_fts WHERE rowid IN (
            SELECT id FROM symbols WHERE file_path = ?1
         )",
        params![file_path],
    )
    .map_err(|source| GraphError::sqlite("delete prior symbol fts rows", source))?;
    tx.execute(
        "DELETE FROM strings_fts WHERE rowid IN (
            SELECT id FROM strings WHERE file_path = ?1
         )",
        params![file_path],
    )
    .map_err(|source| GraphError::sqlite("delete prior string fts rows", source))?;
    tx.execute(
        "DELETE FROM configs_fts WHERE rowid IN (
            SELECT id FROM configs WHERE file_path = ?1
         )",
        params![file_path],
    )
    .map_err(|source| GraphError::sqlite("delete prior config fts rows", source))?;
    Ok(())
}

fn insert_commands(tx: &Transaction<'_>, commands: &[RawCommand]) -> Result<(), GraphError> {
    for command in commands {
        let handler_symbol = match command.handler_symbol.as_ref() {
            Some(qualified) => unique_symbol_id_for_qualified(tx, qualified)?,
            None => None,
        };
        tx.execute(
            "INSERT INTO commands (name, file_path, span_start, handler_symbol)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                command.name,
                command.file_path,
                usize_to_i64("convert command span start", command.span_start)?,
                handler_symbol
            ],
        )
        .map_err(|source| GraphError::sqlite("insert graph command row", source))?;
    }
    Ok(())
}

fn unique_symbol_id_for_qualified(
    tx: &Transaction<'_>,
    qualified: &str,
) -> Result<Option<i64>, GraphError> {
    let mut stmt = tx
        .prepare_cached("SELECT id FROM symbols WHERE qualified = ?1 ORDER BY id LIMIT 2")
        .map_err(|source| GraphError::sqlite("prepare command handler symbol lookup", source))?;
    let ids = stmt
        .query_map(params![qualified], |row| row.get::<_, i64>(0))
        .map_err(|source| GraphError::sqlite("query command handler symbol", source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| GraphError::sqlite("collect command handler symbol", source))?;
    if ids.len() == 1 {
        Ok(ids.first().copied())
    } else {
        Ok(None)
    }
}

fn count_files(conn: &Connection) -> Result<usize, GraphError> {
    let count = conn
        .query_row("SELECT count(*) FROM files", [], |row| row.get::<_, i64>(0))
        .map_err(|source| GraphError::sqlite("count graph files after pass1", source))?;
    usize::try_from(count).map_err(|source| {
        GraphError::invalid_data("count graph files after pass1", source.to_string())
    })
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

#[cfg(test)]
#[path = "tests/pass1.rs"]
mod tests;
