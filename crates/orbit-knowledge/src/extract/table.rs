//! Tabular extractor for CSV / TSV.
//!
//! Table files are indexed at file granularity. The extractor stays registered
//! so the pipeline captures file-level source, but it deliberately emits no
//! per-column leaves.

use super::FileExtractor;
use super::common::ExtractionResult;
use super::language::{FileKind, TableFormat};

pub struct TableExtractor {
    format: TableFormat,
}

impl TableExtractor {
    pub fn new(format: TableFormat) -> Self {
        Self { format }
    }
}

impl FileExtractor for TableExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Table(self.format)
    }

    fn extract(&self, _source: &str) -> ExtractionResult {
        ExtractionResult::default()
    }
}

#[cfg(test)]
mod tests;
