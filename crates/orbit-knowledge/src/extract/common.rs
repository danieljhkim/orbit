use sha2::{Digest, Sha256};

/// A single extracted leaf from a source file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedLeaf {
    pub qualified_name: String,
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub source: String,
    pub source_hash: String,
    /// For methods: the qualified_name of the parent impl/class block.
    pub parent_qualified_name: Option<String>,
    /// For impl/class blocks: qualified_names of child methods.
    pub children_qualified_names: Vec<String>,
    /// ATX heading depth (1–6) for `LeafKind::Section`. `None` for every other kind.
    /// Added T20260422-1540 alongside markdown/config/tabular extraction.
    pub depth: Option<u8>,
}

/// A file-level export discovered while extracting source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedExport {
    pub name: String,
    /// Source path for `pub use` re-exports. `None` means the symbol is defined
    /// in the file itself.
    pub source_path: Option<String>,
}

/// Result of extracting a single file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractionResult {
    pub leaves: Vec<ExtractedLeaf>,
    pub exports: Vec<ExtractedExport>,
}

pub fn compute_source_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Build the identity key for a graph node: `"{node_type}:{location}:{kind}"`.
pub fn identity_key(node_type: &str, location: &str, kind: &str) -> String {
    format!("{node_type}:{location}:{kind}")
}

/// Deterministic node ID: `"{node_type}:{sha256_hex(identity_key)}"`.
pub fn node_id(node_type: &str, location: &str, kind: &str) -> String {
    let key = identity_key(node_type, location, kind);
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("{node_type}:{digest}")
}

/// Build a leaf location string: `"{path}#{qualified_name}"`.
pub fn leaf_location(path: &str, qualified_name: &str) -> String {
    format!("{path}#{qualified_name}")
}
