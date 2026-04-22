use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// LeafKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeafKind {
    Function,
    Method,
    Class,
    Struct,
    Interface,
    Trait,
    Impl,
    Field,
    Module,
    /// Markdown ATX heading (`#`–`######`). `depth` is 1–6. Added T20260422-1540.
    Section {
        depth: u8,
    },
    /// Top-level key in a structured config file (YAML / JSON / TOML).
    /// Added T20260422-1540.
    ConfigKey,
    /// Header cell in a tabular data file (CSV / TSV). Added T20260422-1540.
    Column,
}

impl fmt::Display for LeafKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Field => "field",
            Self::Module => "module",
            Self::Section { .. } => "section",
            Self::ConfigKey => "config_key",
            Self::Column => "column",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// SignatureField
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureField {
    pub name: String,
    #[serde(default)]
    pub annotation: Option<String>,
    #[serde(default)]
    pub description: String,
}

// ---------------------------------------------------------------------------
// LeafHistoryEntry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeafHistoryEntry {
    pub timestamp: DateTime<Utc>,
    pub actor: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub source: String,
    pub source_blob_hash: Option<String>,
    pub source_hash: Option<String>,
    pub file_hash_at_capture: Option<String>,
}

// ---------------------------------------------------------------------------
// BaseNodeFields
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseNodeFields {
    pub id: String,
    #[serde(default)]
    pub identity_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_hash: Option<String>,
    pub name: String,
    pub location: String,
    pub language: String,
    #[serde(default)]
    pub description: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub is_locked: bool,
    #[serde(default)]
    pub lineage_locked: bool,
    pub lock_owner: Option<String>,
    #[serde(default)]
    pub lock_reason: String,
    /// Sorted, deduplicated task IDs (without surrounding brackets) parsed from
    /// commit messages touching this node. Populated by the attribute-history
    /// pipeline stage; see `pipeline::attribute` (T20260421-0528).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_ids: Vec<String>,
    /// Set to `true` when both sides of a merge-commit touched this node.
    /// Informational — git already resolved the text-level conflict.
    #[serde(default, skip_serializing_if = "is_false")]
    pub structural_conflict: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

// ---------------------------------------------------------------------------
// DirNode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirNode {
    #[serde(flatten)]
    pub base: BaseNodeFields,
    #[serde(default)]
    pub dir_children: Vec<String>,
    #[serde(default)]
    pub file_children: Vec<String>,
}

// ---------------------------------------------------------------------------
// FileNode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileNode {
    #[serde(flatten)]
    pub base: BaseNodeFields,
    pub extension: Option<String>,
    pub source_blob_hash: Option<String>,
    #[serde(default)]
    pub imports: Vec<String>,
    #[serde(default)]
    pub exports: Vec<String>,
    #[serde(default)]
    pub leaf_children: Vec<String>,
}

// ---------------------------------------------------------------------------
// LeafNode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeafNode {
    #[serde(flatten)]
    pub base: BaseNodeFields,
    pub kind: LeafKind,
    #[serde(default)]
    pub source: String,
    pub source_blob_hash: Option<String>,
    pub source_hash: Option<String>,
    pub file_hash_at_capture: Option<String>,
    #[serde(default)]
    pub history: Vec<LeafHistoryEntry>,
    #[serde(default)]
    pub input_signature: Vec<SignatureField>,
    #[serde(default)]
    pub output_signature: Vec<SignatureField>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    #[serde(default)]
    pub children: Vec<String>,
}

// ---------------------------------------------------------------------------
// GraphNode (untagged wrapper — constructed programmatically, not via serde)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum GraphNode {
    Dir(DirNode),
    File(FileNode),
    Leaf(LeafNode),
}

impl GraphNode {
    pub fn id(&self) -> &str {
        &self.base().id
    }

    pub fn base(&self) -> &BaseNodeFields {
        match self {
            Self::Dir(n) => &n.base,
            Self::File(n) => &n.base,
            Self::Leaf(n) => &n.base,
        }
    }

    pub fn location(&self) -> &str {
        &self.base().location
    }

    pub fn parent_id(&self) -> Option<&str> {
        self.base().parent_id.as_deref()
    }
}

// ---------------------------------------------------------------------------
// CodebaseGraphV1
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebaseGraphV1 {
    pub root_dir_id: String,
    #[serde(default)]
    pub dirs: Vec<DirNode>,
    #[serde(default)]
    pub files: Vec<FileNode>,
    #[serde(default)]
    pub leaves: Vec<LeafNode>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
