//! Inbound reference and relation query.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_graph_extract::Selector;
use rusqlite::{Connection, Row, params};

use crate::{
    Graph, GraphError, RefConfidence, RefEntry, RefKind, RefOpts, RefResult, RefTarget,
    RelationEntry,
};

const CONFIDENCE_EXACT: &str = "exact";
const CONFIDENCE_IMPORT_RESOLVED: &str = "import_resolved";
const CONFIDENCE_SAME_MODULE: &str = "same_module";
const CONFIDENCE_FUZZY_NAME: &str = "fuzzy_name";

pub(crate) fn run(graph: &Graph, sel: &Selector, opts: &RefOpts) -> Result<RefResult, GraphError> {
    let conn = Connection::open(graph.db_path.path())
        .map_err(|source| GraphError::sqlite("open graph database for refs query", source))?;
    let target = resolve_target(&conn, sel)?;
    let Some(qualified) = target.qualified.as_deref() else {
        return Ok(empty_result(target));
    };

    let mut line_cache = LineCache::new(graph.worktree_root.as_path());
    let mut skipped_low_confidence = 0;
    let refs = if should_query_refs(opts.kind) {
        query_refs(
            &conn,
            qualified,
            opts,
            &mut line_cache,
            &mut skipped_low_confidence,
        )?
    } else {
        Vec::new()
    };
    let relations = if should_query_relations(opts.kind) {
        query_relations(
            &conn,
            qualified,
            opts,
            &mut line_cache,
            &mut skipped_low_confidence,
        )?
    } else {
        Vec::new()
    };

    Ok(RefResult {
        target,
        refs,
        relations,
        skipped_low_confidence,
    })
}

fn resolve_target(conn: &Connection, sel: &Selector) -> Result<RefTarget, GraphError> {
    let Selector::Symbol { path, symbol, kind } = sel else {
        return Ok(RefTarget {
            name: sel.path().to_string(),
            qualified: None,
        });
    };

    let mut stmt = conn
        .prepare_cached(
            "SELECT name, qualified FROM symbols
             WHERE file_path = ?1
               AND kind = ?2
               AND (name = ?3 OR qualified = ?3)
             ORDER BY CASE WHEN qualified = ?3 THEN 0 ELSE 1 END, id
             LIMIT 1",
        )
        .map_err(|source| GraphError::sqlite("prepare refs target resolution", source))?;
    let result = stmt.query_row(params![path, kind, symbol], |row| {
        Ok(RefTarget {
            name: row.get(0)?,
            qualified: Some(row.get(1)?),
        })
    });

    match result {
        Ok(target) => Ok(target),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(RefTarget {
            name: symbol.clone(),
            qualified: None,
        }),
        Err(source) => Err(GraphError::sqlite("resolve refs target symbol", source)),
    }
}

fn empty_result(target: RefTarget) -> RefResult {
    RefResult {
        target,
        refs: Vec::new(),
        relations: Vec::new(),
        skipped_low_confidence: 0,
    }
}

fn should_query_refs(kind: Option<RefKind>) -> bool {
    kind.is_none_or(RefKind::is_textual)
}

fn should_query_relations(kind: Option<RefKind>) -> bool {
    kind.is_none_or(RefKind::is_structural)
}

fn query_refs(
    conn: &Connection,
    qualified: &str,
    opts: &RefOpts,
    line_cache: &mut LineCache,
    skipped_low_confidence: &mut usize,
) -> Result<Vec<RefEntry>, GraphError> {
    let sql = match opts.kind {
        Some(_) => {
            "SELECT from_file, from_span_start, kind, confidence
             FROM refs
             WHERE target_qualified = ?1 AND kind = ?2
             ORDER BY from_file, from_span_start, id"
        }
        None => {
            "SELECT from_file, from_span_start, kind, confidence
             FROM refs
             WHERE target_qualified = ?1
             ORDER BY from_file, from_span_start, id"
        }
    };
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|source| GraphError::sqlite("prepare refs lookup", source))?;
    let rows = match opts.kind {
        Some(kind) => stmt
            .query_map(params![qualified, kind.as_str()], row_to_ref_row)
            .map_err(|source| GraphError::sqlite("query refs by target", source))?
            .collect::<Result<Vec<_>, _>>(),
        None => stmt
            .query_map(params![qualified], row_to_ref_row)
            .map_err(|source| GraphError::sqlite("query refs by target", source))?
            .collect::<Result<Vec<_>, _>>(),
    }
    .map_err(|source| GraphError::sqlite("collect refs lookup rows", source))?;

    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let kind = RefKind::from_db(row.kind.as_str())?;
        let confidence = RefConfidence::from_db(row.confidence.as_str())?;
        if !confidence.visible_at_floor(opts.confidence) {
            *skipped_low_confidence += 1;
            continue;
        }
        entries.push(RefEntry {
            line: line_cache.line_for(row.file.as_str(), row.span_start)?,
            file: row.file,
            kind,
            confidence,
        });
    }
    Ok(entries)
}

fn query_relations(
    conn: &Connection,
    qualified: &str,
    opts: &RefOpts,
    line_cache: &mut LineCache,
    skipped_low_confidence: &mut usize,
) -> Result<Vec<RelationEntry>, GraphError> {
    let sql = match opts.kind {
        Some(_) => {
            "SELECT from_qualified, kind, def_file, def_span_start, confidence
             FROM relations
             WHERE to_qualified = ?1 AND kind = ?2
             ORDER BY def_file, def_span_start, id"
        }
        None => {
            "SELECT from_qualified, kind, def_file, def_span_start, confidence
             FROM relations
             WHERE to_qualified = ?1
             ORDER BY def_file, def_span_start, id"
        }
    };
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|source| GraphError::sqlite("prepare relations lookup", source))?;
    let rows = match opts.kind {
        Some(kind) => stmt
            .query_map(params![qualified, kind.as_str()], row_to_relation_row)
            .map_err(|source| GraphError::sqlite("query relations by target", source))?
            .collect::<Result<Vec<_>, _>>(),
        None => stmt
            .query_map(params![qualified], row_to_relation_row)
            .map_err(|source| GraphError::sqlite("query relations by target", source))?
            .collect::<Result<Vec<_>, _>>(),
    }
    .map_err(|source| GraphError::sqlite("collect relations lookup rows", source))?;

    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let kind = RefKind::from_db(row.kind.as_str())?;
        let confidence = RefConfidence::from_db(row.confidence.as_str())?;
        if !confidence.visible_at_floor(opts.confidence) {
            *skipped_low_confidence += 1;
            continue;
        }
        entries.push(RelationEntry {
            from: row.from,
            kind,
            line: line_cache.line_for(row.file.as_str(), row.span_start)?,
            file: row.file,
            confidence,
        });
    }
    Ok(entries)
}

fn row_to_ref_row(row: &Row<'_>) -> rusqlite::Result<StoredRefRow> {
    Ok(StoredRefRow {
        file: row.get(0)?,
        span_start: row.get(1)?,
        kind: row.get(2)?,
        confidence: row.get(3)?,
    })
}

fn row_to_relation_row(row: &Row<'_>) -> rusqlite::Result<StoredRelationRow> {
    Ok(StoredRelationRow {
        from: row.get(0)?,
        kind: row.get(1)?,
        file: row.get(2)?,
        span_start: row.get(3)?,
        confidence: row.get(4)?,
    })
}

impl RefConfidence {
    fn from_db(value: &str) -> Result<Self, GraphError> {
        match value {
            CONFIDENCE_EXACT => Ok(Self::Exact),
            CONFIDENCE_IMPORT_RESOLVED => Ok(Self::ImportResolved),
            CONFIDENCE_SAME_MODULE => Ok(Self::SameModule),
            CONFIDENCE_FUZZY_NAME => Ok(Self::FuzzyName),
            other => Err(GraphError::invalid_data(
                "parse graph ref confidence",
                format!("unknown confidence `{other}`"),
            )),
        }
    }

    fn visible_at_floor(self, floor: Self) -> bool {
        self.rank() <= floor.rank()
    }

    fn rank(self) -> u8 {
        match self {
            Self::Exact => 1,
            Self::ImportResolved => 2,
            Self::SameModule => 3,
            Self::FuzzyName => 4,
        }
    }
}

impl RefKind {
    fn from_db(value: &str) -> Result<Self, GraphError> {
        match value {
            "call" => Ok(Self::Call),
            "type" => Ok(Self::Type),
            "use" => Ok(Self::Use),
            "trait_bound" => Ok(Self::TraitBound),
            "impl" => Ok(Self::Impl),
            "extends" => Ok(Self::Extends),
            "implements" => Ok(Self::Implements),
            other => Err(GraphError::invalid_data(
                "parse graph ref kind",
                format!("unknown ref kind `{other}`"),
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Type => "type",
            Self::Use => "use",
            Self::TraitBound => "trait_bound",
            Self::Impl => "impl",
            Self::Extends => "extends",
            Self::Implements => "implements",
        }
    }

    fn is_textual(self) -> bool {
        matches!(self, Self::Call | Self::Type | Self::Use | Self::TraitBound)
    }

    fn is_structural(self) -> bool {
        matches!(self, Self::Impl | Self::Extends | Self::Implements)
    }
}

struct StoredRefRow {
    file: String,
    span_start: i64,
    kind: String,
    confidence: String,
}

struct StoredRelationRow {
    from: String,
    kind: String,
    file: String,
    span_start: i64,
    confidence: String,
}

struct LineCache<'a> {
    worktree_root: &'a Path,
    files: BTreeMap<String, LineIndex>,
}

impl<'a> LineCache<'a> {
    fn new(worktree_root: &'a Path) -> Self {
        Self {
            worktree_root,
            files: BTreeMap::new(),
        }
    }

    fn line_for(&mut self, file: &str, byte_offset: i64) -> Result<usize, GraphError> {
        if byte_offset < 0 {
            return Err(GraphError::invalid_data(
                "compute graph ref line",
                format!("negative byte offset {byte_offset} for {file}"),
            ));
        }
        if !self.files.contains_key(file) {
            let path = source_path(self.worktree_root, file);
            let bytes = fs::read(path.as_path()).map_err(|source| {
                GraphError::io("read source file for graph ref line", path, source)
            })?;
            self.files.insert(file.to_string(), LineIndex::new(bytes));
        }
        let offset = usize::try_from(byte_offset).map_err(|source| {
            GraphError::invalid_data("compute graph ref line", source.to_string())
        })?;
        let Some(index) = self.files.get(file) else {
            return Err(GraphError::invalid_data(
                "compute graph ref line",
                format!("line index missing for {file} after load"),
            ));
        };
        Ok(index.line_for(offset))
    }
}

fn source_path(worktree_root: &Path, file: &str) -> PathBuf {
    let path = Path::new(file);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        worktree_root.join(path)
    }
}

struct LineIndex {
    byte_len: usize,
    line_starts: Vec<usize>,
}

impl LineIndex {
    fn new(bytes: Vec<u8>) -> Self {
        let mut line_starts = vec![0];
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        Self {
            byte_len: bytes.len(),
            line_starts,
        }
    }

    fn line_for(&self, byte_offset: usize) -> usize {
        let capped = byte_offset.min(self.byte_len);
        self.line_starts
            .partition_point(|line_start| *line_start <= capped)
    }
}
