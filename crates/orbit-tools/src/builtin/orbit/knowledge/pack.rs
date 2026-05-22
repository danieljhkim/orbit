use orbit_common::types::{
    OrbitError, ToolParam, ToolSchema, optional_string, optional_string_list_alias,
};
use orbit_knowledge::commands::pack::{self, PackInput};
use orbit_knowledge::{
    KnowledgeEntryKind, KnowledgePackAutoRefreshDiagnostic, KnowledgePackDiagnostics,
    KnowledgePackResult,
};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgePackTool;

const DEFAULT_PACK_TIMEOUT_MS: u64 = 15_000;
const MAX_PACK_TIMEOUT_MS: u64 = 300_000;

impl Tool for OrbitKnowledgePackTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.pack".to_string(),
            description:
                "Use when you need exact selectors with context. Prefer over grep when raw text pulls the wrong symbols. Behavior: `file:` stays metadata-only; `summary` hides leaf bodies unless false."
                    .to_string(),
            parameters: vec![
                ToolParam {
                    name: "selectors".to_string(),
                    description: "Selector string or array.".to_string(),
                    param_type: "string_list".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "summary".to_string(),
                    description: "Default true; drop leaf bodies.".to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "timeout_ms".to_string(),
                    description: "Maximum selector-packing time in milliseconds. Default 15000; returns partial unresolved selector entries on timeout.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "refresh".to_string(),
                    description: "Default false; use the existing graph snapshot instead of doing an inline auto-refresh. Set true only when a potentially slow refresh is acceptable.".to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "knowledge_dir".to_string(),
                    description: "Override knowledge dir.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                super::super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selectors = parse_selector_strings(&input)?;
        let summary = input
            .get("summary")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let selector_timeout_ms = parse_timeout_ms(&input)?;
        let refresh = parse_refresh(&input)?;
        let explicit_ref = super::super::optional_string(&input, "ref")?;
        let explicit_knowledge_dir = super::has_explicit_knowledge_dir(&input);
        let pack_result = pack::run(PackInput {
            context: super::command_context(ctx, &input)?,
            selectors,
            hydrate_leaf_source: !summary,
            refresh,
            selector_timeout_ms,
        })
        .map_err(super::knowledge_error_to_orbit)?;
        let mut pack = pack_result.pack;
        add_refresh_diagnostics(
            &mut pack,
            pack_result.auto_refresh_skipped,
            explicit_ref.as_deref(),
            explicit_knowledge_dir,
        );

        if summary {
            summarize_pack(&mut pack);
        }
        pack.refresh_metric_fields();

        serde_json::to_value(pack)
            .map_err(|error| OrbitError::Execution(format!("serialize knowledge pack: {error}")))
    }
}

fn parse_selector_strings(input: &Value) -> Result<Vec<String>, OrbitError> {
    let selectors = if let Some(selectors) = optional_string_list_alias(input, &["selectors"])? {
        selectors
    } else if let Some(file) = optional_string(input, "file")? {
        let selector = if file.starts_with("file:") {
            file
        } else {
            format!("file:{file}")
        };
        vec![selector]
    } else {
        return Err(OrbitError::InvalidInput("missing `selectors`".to_string()));
    };
    if selectors.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`selectors` must contain at least one selector".to_string(),
        ));
    }
    Ok(selectors)
}

fn parse_timeout_ms(input: &Value) -> Result<u64, OrbitError> {
    let Some(value) = input.get("timeout_ms") else {
        return Ok(DEFAULT_PACK_TIMEOUT_MS);
    };
    if value.is_null() {
        return Ok(DEFAULT_PACK_TIMEOUT_MS);
    }
    let Some(timeout_ms) = value.as_u64() else {
        return Err(OrbitError::InvalidInput(
            "`timeout_ms` must be a non-negative integer".to_string(),
        ));
    };
    if timeout_ms > MAX_PACK_TIMEOUT_MS {
        return Err(OrbitError::InvalidInput(format!(
            "`timeout_ms` must be <= {MAX_PACK_TIMEOUT_MS}"
        )));
    }
    Ok(timeout_ms)
}

fn parse_refresh(input: &Value) -> Result<bool, OrbitError> {
    let Some(value) = input.get("refresh") else {
        return Ok(false);
    };
    if value.is_null() {
        return Ok(false);
    }
    value
        .as_bool()
        .ok_or_else(|| OrbitError::InvalidInput("`refresh` must be a boolean".to_string()))
}

// pub(super) visibility widened from private so that knowledge::tests::pack (sibling test after nested collapse)
// can invoke the helper. See ORB-00243 and docs/design-patterns/test_layout.md.
pub(super) fn add_refresh_diagnostics(
    pack: &mut KnowledgePackResult,
    auto_refresh_skipped: bool,
    explicit_ref: Option<&str>,
    explicit_knowledge_dir: bool,
) {
    if !auto_refresh_skipped || explicit_ref.is_some() || explicit_knowledge_dir {
        return;
    }

    pack.diagnostics
        .get_or_insert_with(KnowledgePackDiagnostics::default)
        .auto_refresh = Some(KnowledgePackAutoRefreshDiagnostic {
        status: "skipped".to_string(),
        reason: "orbit.graph.pack reads the existing graph snapshot by default so selector gathering returns promptly.".to_string(),
        remediation: "Run `orbit graph build` for an explicit refresh, or pass `refresh: true` when an inline refresh is acceptable.".to_string(),
    });
}

fn summarize_pack(pack: &mut KnowledgePackResult) {
    for entry in &mut pack.entries {
        summarize_pack_entry(entry);
    }
}

fn summarize_pack_entry(entry: &mut orbit_knowledge::KnowledgePackEntry) {
    if entry.kind != KnowledgeEntryKind::Leaf {
        return;
    }

    entry.source = None;

    let Some(file_path) = entry
        .selector
        .strip_prefix("symbol:")
        .and_then(|rest| rest.split_once('#').map(|(path, _)| path.to_string()))
    else {
        return;
    };
    entry.file = Some(file_path);
}
