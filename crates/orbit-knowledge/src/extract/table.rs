//! Shallow tabular extractor for CSV / TSV (T20260422-1540).
//!
//! Emits one `LeafKind::Column` per header cell (splitting the first
//! non-empty line on `,` for CSV and `\t` for TSV). Files larger than
//! `SIZE_CAP_BYTES` produce zero leaves — the cap short-circuits before
//! parsing to avoid ingesting very large data dumps.
//!
//! Deliberately out of scope: quoted/escaped cells, multi-line headers,
//! type inference, row-level nodes.

use super::FileExtractor;
use super::common::{ExtractedLeaf, ExtractionResult, compute_source_hash};
use super::language::{FileKind, TableFormat};

/// Files above this size are not parsed. Deliberate MVP default; not
/// user-configurable per task T20260422-1540 scope.
pub(crate) const SIZE_CAP_BYTES: usize = 1024 * 1024;

pub struct TableExtractor {
    format: TableFormat,
}

impl TableExtractor {
    pub fn new(format: TableFormat) -> Self {
        Self { format }
    }

    fn delimiter(&self) -> char {
        match self.format {
            TableFormat::Csv => ',',
            TableFormat::Tsv => '\t',
        }
    }
}

impl FileExtractor for TableExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Table(self.format)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        extract_with_cap(self, source, SIZE_CAP_BYTES)
    }
}

fn extract_with_cap(ext: &TableExtractor, source: &str, cap: usize) -> ExtractionResult {
    if source.len() > cap {
        return ExtractionResult::default();
    }
    let Some(header) = source.lines().find(|line| !line.trim().is_empty()) else {
        return ExtractionResult::default();
    };
    let cells: Vec<&str> = header.split(ext.delimiter()).collect();
    let mut leaves = Vec::with_capacity(cells.len());
    for raw in cells.into_iter() {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        let hash = compute_source_hash(&trimmed);
        leaves.push(ExtractedLeaf {
            qualified_name: trimmed.clone(),
            name: trimmed,
            kind: "column".to_string(),
            start_line: 1,
            end_line: 1,
            source: String::new(),
            source_hash: hash,
            parent_qualified_name: None,
            children_qualified_names: Vec::new(),
            depth: None,
        });
    }
    ExtractionResult { leaves }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_csv_header_columns() {
        let src = "id,name,email\n1,alice,a@x\n2,bob,b@x\n";
        let out = TableExtractor::new(TableFormat::Csv).extract(src);
        let names: Vec<&String> = out.leaves.iter().map(|l| &l.name).collect();
        assert_eq!(names, vec!["id", "name", "email"]);
        assert!(out.leaves.iter().all(|l| l.kind == "column"));
    }

    #[test]
    fn extracts_tsv_header_columns() {
        let src = "id\tname\temail\n1\talice\ta@x\n";
        let out = TableExtractor::new(TableFormat::Tsv).extract(src);
        let names: Vec<&String> = out.leaves.iter().map(|l| &l.name).collect();
        assert_eq!(names, vec!["id", "name", "email"]);
    }

    #[test]
    fn oversized_input_yields_zero_leaves() {
        // Use a stub cap so the test doesn't actually allocate 1 MiB.
        let ext = TableExtractor::new(TableFormat::Csv);
        let src = "a,b,c\n";
        let out = extract_with_cap(&ext, src, 2);
        assert!(out.leaves.is_empty());
    }

    #[test]
    fn empty_input_yields_zero_leaves() {
        let out = TableExtractor::new(TableFormat::Csv).extract("");
        assert!(out.leaves.is_empty());
    }

    #[test]
    fn trims_whitespace_and_skips_empty_cells() {
        let src = "id, name ,,email\n";
        let out = TableExtractor::new(TableFormat::Csv).extract(src);
        let names: Vec<&String> = out.leaves.iter().map(|l| &l.name).collect();
        assert_eq!(names, vec!["id", "name", "email"]);
    }
}
