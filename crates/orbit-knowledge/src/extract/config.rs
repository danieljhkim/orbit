//! Structured-config extractor for YAML / JSON / TOML.
//!
//! Config files are indexed at file granularity. The extractor stays registered
//! so the pipeline captures file-level source, but it deliberately emits no
//! per-key leaves.

use super::FileExtractor;
use super::common::ExtractionResult;
use super::language::{ConfigFormat, FileKind};

pub struct ConfigExtractor {
    format: ConfigFormat,
}

impl ConfigExtractor {
    pub fn new(format: ConfigFormat) -> Self {
        Self { format }
    }
}

impl FileExtractor for ConfigExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Config(self.format)
    }

    fn extract(&self, _source: &str) -> ExtractionResult {
        ExtractionResult::default()
    }
}
