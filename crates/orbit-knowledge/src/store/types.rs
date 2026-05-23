use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeEntryKind {
    Dir,
    File,
    Leaf,
    #[default]
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedSelectorReason {
    OutsideIndexedRoots,
    NotFound,
    StaleSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgePackEntry {
    #[serde(default)]
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default)]
    pub kind: KnowledgeEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<UnresolvedSelectorReason>,
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
    pub re_exports: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_summary: Option<Vec<SymbolSummary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
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

impl KnowledgePackEntry {
    pub(crate) fn unresolved(selector: String) -> Self {
        Self {
            selector,
            file: None,
            kind: KnowledgeEntryKind::Unresolved,
            reason: None,
            name: None,
            language: None,
            description: None,
            extension: None,
            imports: None,
            exports: None,
            re_exports: None,
            children: None,
            symbol_summary: None,
            source: None,
            hint: None,
            start_line: None,
            end_line: None,
            input_signature: None,
            output_signature: None,
            resolved: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolSummary {
    pub selector: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgePack {
    pub knowledge_dir: String,
    pub manifest_generated_at: String,
    pub unresolved_selectors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<KnowledgePackTimeout>,
    pub total_nodes: usize,
    pub entries: Vec<KnowledgePackEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgePackTimeout {
    pub timeout_ms: u64,
    pub processed_selectors: usize,
    pub total_selectors: usize,
    pub hint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct KnowledgePackResult {
    #[serde(default)]
    pub raw_read_token_baseline: u64,
    #[serde(default)]
    pub knowledge_pack_tokens: u64,
    #[serde(default)]
    pub knowledge_dir: String,
    #[serde(default)]
    pub manifest_generated_at: String,
    #[serde(default)]
    pub unresolved_selectors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<KnowledgePackTimeout>,
    #[serde(default)]
    pub total_nodes: usize,
    #[serde(default)]
    pub entries: Vec<KnowledgePackEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<KnowledgePackDiagnostics>,
}

impl KnowledgePackResult {
    pub fn from_pack(pack: KnowledgePack) -> Self {
        Self {
            raw_read_token_baseline: 0,
            knowledge_pack_tokens: 0,
            knowledge_dir: pack.knowledge_dir,
            manifest_generated_at: pack.manifest_generated_at,
            unresolved_selectors: pack.unresolved_selectors,
            timeout: pack.timeout,
            total_nodes: pack.total_nodes,
            entries: pack.entries,
            diagnostics: None,
        }
    }

    pub(crate) fn from_error(
        knowledge_dir: impl Into<String>,
        selectors: &[crate::Selector],
        error: crate::KnowledgeError,
    ) -> Self {
        let unresolved_selectors = selectors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let entries = unresolved_selectors
            .iter()
            .cloned()
            .map(KnowledgePackEntry::unresolved)
            .collect();
        Self {
            raw_read_token_baseline: 0,
            knowledge_pack_tokens: 0,
            knowledge_dir: knowledge_dir.into(),
            manifest_generated_at: String::new(),
            unresolved_selectors,
            timeout: None,
            total_nodes: 0,
            entries,
            diagnostics: Some(KnowledgePackDiagnostics {
                error: Some(KnowledgePackErrorDiagnostic {
                    kind: error.kind.to_string(),
                    reason: error.reason,
                    did_you_mean: error.did_you_mean,
                }),
                ..KnowledgePackDiagnostics::default()
            }),
        }
    }

    pub fn refresh_metric_fields(&mut self) {
        let raw_source_tokens = self
            .entries
            .iter()
            .filter_map(|entry| entry.source.as_deref())
            .map(string_token_count)
            .fold(0u64, u64::saturating_add);

        let mut pack_tokens = self.knowledge_pack_tokens;
        for _ in 0..6 {
            self.knowledge_pack_tokens = pack_tokens;
            self.raw_read_token_baseline = if raw_source_tokens > 0 {
                raw_source_tokens
            } else {
                pack_tokens
            };
            let next = serialized_token_count(self);
            if next == pack_tokens {
                break;
            }
            pack_tokens = next;
        }

        self.knowledge_pack_tokens = pack_tokens;
        self.raw_read_token_baseline = if raw_source_tokens > 0 {
            raw_source_tokens
        } else {
            pack_tokens
        };
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct KnowledgePackDiagnostics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_refresh: Option<KnowledgePackAutoRefreshDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<KnowledgePackErrorDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct KnowledgePackAutoRefreshDiagnostic {
    pub status: String,
    pub reason: String,
    pub remediation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct KnowledgePackErrorDiagnostic {
    pub kind: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub did_you_mean: Vec<String>,
}

fn serialized_token_count<T: Serialize>(value: &T) -> u64 {
    match serde_json::to_string(value) {
        Ok(text) => string_token_count(&text),
        Err(_) => 0,
    }
}

fn string_token_count(text: &str) -> u64 {
    tiktoken_rs::cl100k_base_singleton()
        .encode_with_special_tokens(text)
        .len() as u64
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
