use std::path::PathBuf;

use orbit_common::types::OrbitError;
use serde_json::Value;

use crate::KnowledgeError;
use crate::commands::{GraphCommandContext, knowledge_error_from_orbit};
use orbit_graph_extract::Selector;

#[derive(Debug, Clone)]
pub struct MutationContext {
    pub graph: GraphCommandContext,
    pub workspace_root: PathBuf,
}

#[derive(Debug, Clone)]
pub enum MutationInput {
    Write {
        context: MutationContext,
        selector: String,
        new_source: String,
        position: Option<String>,
        start_line: Option<u64>,
        end_line: Option<u64>,
        reason: Option<String>,
    },
    Add {
        context: MutationContext,
        selector: String,
        source: String,
        position: Option<String>,
        reason: Option<String>,
    },
    Delete {
        context: MutationContext,
        selector: String,
        reason: Option<String>,
    },
    Move {
        context: MutationContext,
        selector: String,
        target_file: String,
        position: Option<String>,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MutationResult {
    pub value: Value,
}

pub fn run(input: MutationInput) -> Result<MutationResult, KnowledgeError> {
    let value = match input {
        MutationInput::Write {
            context,
            selector,
            new_source,
            position,
            start_line,
            end_line,
            reason,
        } => write(
            context, selector, new_source, position, start_line, end_line, reason,
        )?,
        MutationInput::Add {
            context,
            selector,
            source,
            position,
            reason,
        } => add(context, selector, source, position, reason)?,
        MutationInput::Delete {
            context,
            selector,
            reason,
        } => delete(context, selector, reason)?,
        MutationInput::Move {
            context,
            selector,
            target_file,
            position,
            reason,
        } => move_leaf(context, selector, target_file, position, reason)?,
    };

    Ok(MutationResult { value })
}

fn write(
    context: MutationContext,
    selector_str: String,
    new_source: String,
    position: Option<String>,
    start_line: Option<u64>,
    end_line: Option<u64>,
    reason: Option<String>,
) -> Result<Value, KnowledgeError> {
    if new_source.trim().is_empty() {
        return Err(KnowledgeError::invalid_data(
            "`new_source` must not be empty".to_string(),
        ));
    }
    let selector = parse_selector(&selector_str)?;
    if matches!(selector, Selector::Dir { .. }) {
        return Err(KnowledgeError::invalid_data(
            "graph.write does not accept dir selectors".to_string(),
        ));
    }
    let position_selector = parse_position_selector(position.as_deref())?;
    let service = context.graph.task_service();
    let workspace_root = context.workspace_root;
    let result = service
        .mutate(
            &selector,
            &[],
            reason.as_deref().unwrap_or("editing"),
            &workspace_root,
            |working_graph| match &selector {
                Selector::File { path } => match (start_line, end_line) {
                    (Some(start_line), Some(end_line)) => working_graph
                        .rewrite_file_region(
                            path.as_str(),
                            start_line as usize,
                            end_line as usize,
                            &new_source,
                            reason.as_deref(),
                            &workspace_root,
                        )
                        .map_err(write_err_to_orbit),
                    (None, None) => working_graph
                        .rewrite_file(
                            path.as_str(),
                            &new_source,
                            reason.as_deref(),
                            &workspace_root,
                        )
                        .map_err(write_err_to_orbit),
                    _ => Err(OrbitError::InvalidInput(
                        "both start_line and end_line are required for region edits".to_string(),
                    )),
                },
                Selector::Symbol { .. } => {
                    if working_graph.has_leaf(&selector) {
                        working_graph
                            .edit_leaf(&selector, &new_source, reason.as_deref(), &workspace_root)
                            .map_err(write_err_to_orbit)
                    } else {
                        working_graph
                            .insert_leaf(
                                &selector,
                                &new_source,
                                position_selector.as_ref(),
                                reason.as_deref(),
                                &workspace_root,
                            )
                            .map_err(write_err_to_orbit)
                    }
                }
                Selector::Dir { .. } => unreachable!(),
            },
        )
        .map_err(knowledge_error_from_orbit)?;

    serde_json::to_value(result).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!("serialize result: {error}"))
    })
}

fn add(
    context: MutationContext,
    selector_str: String,
    source: String,
    position: Option<String>,
    reason: Option<String>,
) -> Result<Value, KnowledgeError> {
    let selector = parse_selector(&selector_str)?;
    if !matches!(selector, Selector::Symbol { .. }) {
        return Err(KnowledgeError::invalid_data(
            "graph.add requires a symbol selector (symbol:path#name:kind)".to_string(),
        ));
    }
    let position_selector = parse_position_selector(position.as_deref())?;
    let service = context.graph.task_service();
    let workspace_root = context.workspace_root;
    let result = service
        .mutate(
            &selector,
            &[],
            reason.as_deref().unwrap_or("adding"),
            &workspace_root,
            |working_graph| {
                if working_graph.has_leaf(&selector) {
                    let error = crate::WriteError::leaf_already_exists(&selector_str);
                    return Err(write_err_to_orbit(error));
                }
                working_graph
                    .insert_leaf(
                        &selector,
                        &source,
                        position_selector.as_ref(),
                        reason.as_deref(),
                        &workspace_root,
                    )
                    .map_err(write_err_to_orbit)
            },
        )
        .map_err(knowledge_error_from_orbit)?;

    serde_json::to_value(result).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!("serialize result: {error}"))
    })
}

fn delete(
    context: MutationContext,
    selector_str: String,
    reason: Option<String>,
) -> Result<Value, KnowledgeError> {
    let selector = parse_selector(&selector_str)?;
    if !matches!(selector, Selector::Symbol { .. }) {
        return Err(KnowledgeError::invalid_data(
            "graph.delete requires a symbol selector (symbol:path#name:kind)".to_string(),
        ));
    }
    let service = context.graph.task_service();
    let workspace_root = context.workspace_root;
    let result = service
        .mutate(
            &selector,
            &[],
            reason.as_deref().unwrap_or("deleting"),
            &workspace_root,
            |working_graph| {
                working_graph
                    .delete_leaf(&selector, reason.as_deref(), &workspace_root)
                    .map_err(write_err_to_orbit)
            },
        )
        .map_err(knowledge_error_from_orbit)?;

    serde_json::to_value(result).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!("serialize result: {error}"))
    })
}

fn move_leaf(
    context: MutationContext,
    selector_str: String,
    target_file: String,
    position: Option<String>,
    reason: Option<String>,
) -> Result<Value, KnowledgeError> {
    let selector = parse_selector(&selector_str)?;
    if !matches!(selector, Selector::Symbol { .. }) {
        return Err(KnowledgeError::invalid_data(
            "graph.move requires a symbol selector (symbol:path#name:kind)".to_string(),
        ));
    }
    let position_selector = parse_position_selector(position.as_deref())?;
    let service = context.graph.task_service();
    let workspace_root = context.workspace_root;
    let result = service
        .mutate(
            &selector,
            &[target_file.as_str()],
            reason.as_deref().unwrap_or("moving"),
            &workspace_root,
            |working_graph| {
                working_graph
                    .move_leaf(
                        &selector,
                        &target_file,
                        position_selector.as_ref(),
                        reason.as_deref(),
                        &workspace_root,
                    )
                    .map_err(write_err_to_orbit)
            },
        )
        .map_err(knowledge_error_from_orbit)?;

    serde_json::to_value(result).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!("serialize result: {error}"))
    })
}

fn parse_selector(selector: &str) -> Result<Selector, KnowledgeError> {
    selector
        .parse::<Selector>()
        .map_err(|error| KnowledgeError::invalid_data(error.to_string()))
}

fn parse_position_selector(position: Option<&str>) -> Result<Option<Selector>, KnowledgeError> {
    let Some(position) = position else {
        return Ok(None);
    };
    let selector = position.strip_prefix("after:").unwrap_or(position);
    selector
        .parse()
        .map(Some)
        .map_err(|error| KnowledgeError::invalid_data(format!("invalid position: {error}")))
}

fn write_err_to_orbit(error: crate::WriteError) -> OrbitError {
    serde_json::to_value(&error)
        .map(|value| OrbitError::Execution(value.to_string()))
        .unwrap_or_else(|_| OrbitError::Execution(format!("{error:?}")))
}
