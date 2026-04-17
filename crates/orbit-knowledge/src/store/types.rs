use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeEntryKind {
    Dir,
    File,
    Leaf,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KnowledgePackEntry {
    pub selector: String,
    pub kind: KnowledgeEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imports: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exports: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_summary: Option<Vec<SymbolSummary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_signature: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_signature: Option<Vec<Value>>,
    #[serde(skip)]
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SymbolSummary {
    pub selector: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KnowledgePack {
    pub knowledge_dir: String,
    pub manifest_generated_at: String,
    pub unresolved_selectors: Vec<String>,
    pub total_nodes: usize,
    pub entries: Vec<KnowledgePackEntry>,
}

#[derive(Debug, Clone)]
pub struct LeafData {
    pub file_path: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub source: String,
    pub source_hash: String,
    pub parent_qualified_name: Option<String>,
    pub children_qualified_names: Vec<String>,
}
