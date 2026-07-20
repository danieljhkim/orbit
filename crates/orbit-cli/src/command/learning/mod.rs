mod add;
mod command;
mod list;
mod migrate_layout;
pub(crate) mod output;
mod prune;
mod show;
mod stats;
mod supersede;
mod sync;
mod update;

pub use command::{LearningCommand, LearningSubcommand};

fn managed_tool(
    runtime: &orbit_core::OrbitRuntime,
    name: &str,
    input: serde_json::Value,
) -> Result<Option<serde_json::Value>, orbit_core::OrbitError> {
    if matches!(
        runtime.knowledge_owner_access(),
        orbit_core::command::knowledge_policy::KnowledgeOwnerAccess::Standalone
    ) {
        return Ok(None);
    }
    orbit_remote::execute_managed_knowledge_tool(
        &runtime.global_root(),
        &runtime.paths().repo_root.to_string_lossy(),
        name,
        input,
        Some("human".to_string()),
    )
    .map(Some)
}
