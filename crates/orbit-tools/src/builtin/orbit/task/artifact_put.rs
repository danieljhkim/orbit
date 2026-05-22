use std::path::PathBuf;

use orbit_common::types::{OrbitError, TaskArtifact, ToolParam, ToolSchema};
use serde_json::{Map, Value, json};

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskArtifactPutTool;

impl Tool for OrbitTaskArtifactPutTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::super::orbit_id_params("task");
        parameters.extend([
            ToolParam {
                name: "source_path".to_string(),
                description: "Source file to store as a task artifact.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "path".to_string(),
                description:
                    "Artifact path relative to the task artifacts directory. Defaults to the source file name."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ]);
        parameters.extend(super::super::identity_params());

        ToolSchema {
            name: "orbit.task.artifact.put".to_string(),
            description: "Store a source file under a task's artifacts directory".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let id = super::super::required_string(&input, &["id"], "id")?;
        let source_path = super::super::required_string(
            &input,
            &["source_path", "sourcePath", "source-path"],
            "source_path",
        )?;
        let artifact_path = super::super::optional_string_alias(
            &input,
            &["path", "artifact_path", "artifactPath"],
        )?;
        let resolved_source_path = resolve_source_path(ctx, &source_path);
        let artifact =
            TaskArtifact::from_source_file(&resolved_source_path, artifact_path.as_deref())?;

        let mut update_input = input.as_object().cloned().unwrap_or_else(Map::new);
        update_input.insert("id".to_string(), Value::String(id));
        update_input.remove("source_path");
        update_input.remove("sourcePath");
        update_input.remove("source-path");
        update_input.remove("path");
        update_input.remove("artifact_path");
        update_input.remove("artifactPath");
        update_input.insert(
            "artifacts".to_string(),
            json!([{
                "path": artifact.path,
                "media_type": artifact.media_type,
                "content": artifact.content,
            }]),
        );

        super::super::execute_host_action(
            ctx,
            Value::Object(update_input),
            OrbitBuiltinAction::TaskUpdate,
        )
    }
}

fn resolve_source_path(ctx: &ToolContext, source_path: &str) -> PathBuf {
    let path = PathBuf::from(source_path);
    if path.is_absolute() {
        return path;
    }
    ctx.cwd
        .as_ref()
        .map(PathBuf::from)
        .map(|cwd| cwd.join(&path))
        .unwrap_or(path)
}
