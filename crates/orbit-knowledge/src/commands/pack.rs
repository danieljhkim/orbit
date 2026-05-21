use crate::commands::{GraphCommandContext, knowledge_error_from_orbit};
use crate::graph::GraphReadOptions;
use crate::graph::object_store::{GraphObjectStore, resolve_graph_read_target};
use crate::{KnowledgeError, KnowledgePackResult, Selector};

#[derive(Debug, Clone)]
pub struct PackInput {
    pub context: GraphCommandContext,
    pub selectors: Vec<String>,
    pub hydrate_leaf_source: bool,
    pub refresh: bool,
    pub selector_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackResult {
    pub pack: KnowledgePackResult,
    pub auto_refresh_skipped: bool,
}

pub fn run(input: PackInput) -> Result<PackResult, KnowledgeError> {
    let selectors = Selector::parse_many(&input.selectors)
        .map_err(|error| KnowledgeError::invalid_data(error.to_string()))?;
    let service = input.context.task_service();
    let auto_refresh_skipped = !input.refresh && current_branch_ref_available(&input.context);
    let skip_auto_refresh = input.context.explicit_knowledge_dir || auto_refresh_skipped;
    let pack = service
        .pack_result(
            &selectors,
            input.context.workspace_root.as_deref(),
            skip_auto_refresh,
            input.context.explicit_ref.as_deref(),
            GraphReadOptions {
                hydrate_leaf_source: input.hydrate_leaf_source,
                ..Default::default()
            },
            Some(input.selector_timeout_ms),
        )
        .map_err(knowledge_error_from_orbit)?;

    Ok(PackResult {
        pack,
        auto_refresh_skipped,
    })
}

fn current_branch_ref_available(context: &GraphCommandContext) -> bool {
    if context.explicit_ref.is_some() || context.explicit_knowledge_dir {
        return false;
    }

    let Some(workspace_root) = context.workspace_root.as_deref() else {
        return false;
    };
    let Ok(read_target) = resolve_graph_read_target(Some(workspace_root), None) else {
        return false;
    };
    let graph_store = GraphObjectStore::new(context.knowledge_dir.join("graph"));
    if graph_store
        .prepare_refs_layout(read_target.default.as_ref())
        .is_err()
    {
        return false;
    }

    graph_store.ref_path(&read_target.requested).is_file()
}

