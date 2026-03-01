use std::fs;
use std::path::Path;

use orbit_types::OrbitError;
use serde_json::json;

use crate::OrbitRuntime;
use crate::command::work::WorkAddParams;

const DEFAULT_IDENTITIES: [(&str, &str); 6] = [
    ("linus", include_str!("../../assets/identities/linus.yaml")),
    ("john", include_str!("../../assets/identities/john.yaml")),
    ("kent", include_str!("../../assets/identities/kent.yaml")),
    ("rob", include_str!("../../assets/identities/rob.yaml")),
    ("grace", include_str!("../../assets/identities/grace.yaml")),
    ("steve", include_str!("../../assets/identities/steve.yaml")),
];
const DEFAULT_APPROVAL_WORK_ID: &str = "approve-task-leader";

#[derive(Debug, Clone)]
pub struct InitResult {
    pub created_identity_files: usize,
    pub identity_root: String,
    pub created_default_work: bool,
}

impl OrbitRuntime {
    pub fn init_workspace(&self) -> Result<InitResult, OrbitError> {
        let identity_root = self.identity_root();
        fs::create_dir_all(&identity_root).map_err(|e| OrbitError::Io(e.to_string()))?;

        let mut created = 0usize;
        for (name, content) in DEFAULT_IDENTITIES {
            let path = identity_root.join(format!("{name}.yaml"));
            if path.exists() {
                continue;
            }
            write_identity_file(&path, content)?;
            created += 1;
        }

        let created_default_work = self.show_work(DEFAULT_APPROVAL_WORK_ID).is_err()
            && self
                .add_work(WorkAddParams {
                    id: DEFAULT_APPROVAL_WORK_ID.to_string(),
                    spec_type: "task_approval".to_string(),
                    description: "Leader review and delegated task approval workflow".to_string(),
                    input_schema_json: json!({
                        "type": "object",
                        "required": ["task_id", "decision"],
                        "properties": {
                            "task_id": { "type": "string" },
                            "decision": { "type": "string", "enum": ["approve", "reject"] },
                            "note": { "type": "string" }
                        },
                        "additionalProperties": false
                    }),
                    output_schema_json: json!({
                        "type": "object",
                        "required": ["task_id", "decision", "approved"],
                        "properties": {
                            "task_id": { "type": "string" },
                            "decision": { "type": "string" },
                            "approved": { "type": "boolean" },
                            "comment": { "type": "string" }
                        }
                    }),
                    artifact_path_template: Some(
                        "~/.orbit/agents/{{repo_name}}/executions/{{date}}-approve-task.md"
                            .to_string(),
                    ),
                    skill_refs: Vec::new(),
                    identity_id: Some("linus".to_string()),
                    assigned_to: Some("Linus Torvalds (Maintainer)".to_string()),
                    created_by: Some("system".to_string()),
                })
                .is_ok();

        Ok(InitResult {
            created_identity_files: created,
            identity_root: identity_root.to_string_lossy().to_string(),
            created_default_work,
        })
    }
}

fn write_identity_file(path: &Path, content: &str) -> Result<(), OrbitError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    fs::write(path, content).map_err(|e| OrbitError::Io(e.to_string()))
}
