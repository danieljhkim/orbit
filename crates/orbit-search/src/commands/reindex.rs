use orbit_common::types::{OrbitError, Task};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::commands::doc_index::DocIndexResult;
use crate::commands::learning_index::LearningIndexResult;
use crate::commands::parse_model;
use crate::vector::{UpsertReport, VectorStore};
use crate::{Embedder, SubprocessEmbedder};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexKind {
    Tasks,
    Docs,
    Learnings,
    All,
}

impl Default for IndexKind {
    fn default() -> Self {
        Self::Tasks
    }
}

impl FromStr for IndexKind {
    type Err = OrbitError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "tasks" => Ok(Self::Tasks),
            "docs" => Ok(Self::Docs),
            "learnings" => Ok(Self::Learnings),
            "all" => Ok(Self::All),
            value => Err(OrbitError::InvalidInput(format!(
                "unsupported semantic index kind `{value}`; supported values: tasks, docs, learnings, all"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SemanticIndexParams {
    pub model: Option<String>,
    pub force: bool,
    pub kind: Option<IndexKind>,
}

impl Default for SemanticIndexParams {
    fn default() -> Self {
        Self {
            model: None,
            force: false,
            kind: None,
        }
    }
}

impl SemanticIndexParams {
    pub fn resolved_kind(&self) -> IndexKind {
        self.kind.unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskIndexResult {
    pub model_id: String,
    pub report: UpsertReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SemanticIndexResult {
    Tasks {
        model_id: String,
        report: UpsertReport,
    },
    Docs {
        model_id: String,
        report: UpsertReport,
        indexed_sources: usize,
        stale_sources: Vec<String>,
    },
    Learnings {
        model_id: String,
        report: UpsertReport,
        indexed_sources: usize,
        stale_sources: Vec<String>,
    },
    All {
        tasks: TaskIndexResult,
        docs: DocIndexResult,
        learnings: LearningIndexResult,
    },
}

impl From<TaskIndexResult> for SemanticIndexResult {
    fn from(result: TaskIndexResult) -> Self {
        Self::Tasks {
            model_id: result.model_id,
            report: result.report,
        }
    }
}

impl From<DocIndexResult> for SemanticIndexResult {
    fn from(result: DocIndexResult) -> Self {
        Self::Docs {
            model_id: result.model_id,
            report: result.report,
            indexed_sources: result.indexed_sources,
            stale_sources: result.stale_sources,
        }
    }
}

impl From<LearningIndexResult> for SemanticIndexResult {
    fn from(result: LearningIndexResult) -> Self {
        Self::Learnings {
            model_id: result.model_id,
            report: result.report,
            indexed_sources: result.indexed_sources,
            stale_sources: result.stale_sources,
        }
    }
}

pub type SemanticReindexParams = SemanticIndexParams;
pub type SemanticReindexResult = TaskIndexResult;

pub fn run(
    vector_store: &VectorStore,
    tasks: &[Task],
    params: SemanticIndexParams,
) -> Result<TaskIndexResult, OrbitError> {
    let model = parse_model(params.model.as_deref())?;
    let embedder = SubprocessEmbedder::with_model(model.alias)?;
    let report = vector_store.reindex_tasks(tasks, &embedder, params.force)?;
    Ok(TaskIndexResult {
        model_id: embedder.model_id().to_string(),
        report,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn semantic_index_params_serde_defaults_to_tasks_at_runtime() {
        let empty: SemanticIndexParams = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.kind, None);
        assert_eq!(empty.resolved_kind(), IndexKind::Tasks);

        let model_only: SemanticIndexParams =
            serde_json::from_str(r#"{"model":"bge-small"}"#).unwrap();
        assert_eq!(model_only.model.as_deref(), Some("bge-small"));
        assert_eq!(model_only.kind, None);
        assert_eq!(model_only.resolved_kind(), IndexKind::Tasks);

        let docs: SemanticIndexParams = serde_json::from_str(r#"{"kind":"docs"}"#).unwrap();
        assert_eq!(docs.kind, Some(IndexKind::Docs));
        assert_eq!(docs.resolved_kind(), IndexKind::Docs);

        let learnings: SemanticIndexParams =
            serde_json::from_str(r#"{"kind":"learnings"}"#).unwrap();
        assert_eq!(learnings.kind, Some(IndexKind::Learnings));
        assert_eq!(learnings.resolved_kind(), IndexKind::Learnings);

        let all: SemanticIndexParams = serde_json::from_str(r#"{"kind":"all"}"#).unwrap();
        assert_eq!(all.kind, Some(IndexKind::All));
        assert_eq!(all.resolved_kind(), IndexKind::All);
    }

    #[test]
    fn semantic_index_kind_rejects_singular_learning() {
        let error = IndexKind::from_str("learning").expect_err("singular kind should fail");

        assert!(error.to_string().contains("`learning`"));
        assert!(error.to_string().contains("learnings"));
    }

    #[test]
    fn tasks_variant_serializes_like_legacy_reindex_result() {
        let result = SemanticIndexResult::Tasks {
            model_id: "bge-small-en-v1.5".to_string(),
            report: UpsertReport {
                embedded_chunks: 7,
                skipped_fields: 2,
            },
        };

        let expected = json!({
                "model_id": "bge-small-en-v1.5",
                "report": {
                    "embedded_chunks": 7,
                    "skipped_fields": 2
                }
        });
        assert_eq!(serde_json::to_value(&result).unwrap(), expected);
        assert_eq!(
            serde_json::to_string(&result).unwrap(),
            r#"{"model_id":"bge-small-en-v1.5","report":{"embedded_chunks":7,"skipped_fields":2}}"#
        );
    }
}
