//! Shared owner/role enforcement for every knowledge entry point.

use orbit_common::types::OrbitError;
use orbit_tools::OrbitBuiltinAction;
use serde_json::Value;

/// The authoritative relationship between this runtime and current knowledge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum KnowledgeOwnerAccess {
    /// Legacy single-host mode retains the per-workspace allocator and local
    /// authoring behavior.
    #[default]
    Standalone,
    /// This exact checkout belongs to the declared owner.
    Owner { owner_machine_id: String },
    /// This checkout is a Git-carried replica; its files are never current.
    Replica { owner_machine_id: String },
    /// A managed workspace has no usable exact owner checkout on this process.
    Unavailable { owner_machine_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnowledgeSurfaceClass {
    NonKnowledge,
    CompositeCreate,
    CurrentRead,
    CurrentMutation,
    LocalDerived,
}

/// Exhaustively classify the generic tool registry. Adding a new builtin makes
/// this match fail to compile until its knowledge semantics are declared.
pub fn classify_builtin(action: OrbitBuiltinAction, input: &Value) -> KnowledgeSurfaceClass {
    use KnowledgeSurfaceClass::{
        CompositeCreate, CurrentMutation, CurrentRead, LocalDerived, NonKnowledge,
    };
    use OrbitBuiltinAction::{
        AdrAdd, AdrList, AdrShow, AdrSupersede, AdrUpdate, AutoTaskAdd, AutoTaskList, AutoTaskShow,
        AutoTaskToggle, AutoTaskUpdate, DocsAdd, DocsIndex, DocsList, DocsMigrate, DocsShow,
        FrictionAdd, FrictionList, FrictionResolve, FrictionShow, FrictionStats, FrictionTags,
        FrictionUpdate, LearningAdd, LearningList, LearningPrune, LearningShow, LearningSupersede,
        LearningSync, LearningUpdate, PipelineInvoke, PipelineWait, ReviewThreadAdd,
        ReviewThreadList, ReviewThreadReply, ReviewThreadResolve, Search, SemanticIndex,
        SemanticInstall, SemanticStats, SemanticUninstall, StateGet, StateSet, TaskAdd,
        TaskApprove, TaskDelete, TaskLint, TaskList, TaskLocks, TaskLocksRelease, TaskLocksReserve,
        TaskReject, TaskShow, TaskStart, TaskUpdate,
    };

    match action {
        AdrAdd | LearningAdd => CompositeCreate,
        AdrShow | AdrList | LearningShow | LearningList => CurrentRead,
        AdrUpdate | AdrSupersede | LearningUpdate | LearningSupersede => CurrentMutation,
        LearningPrune => {
            if input
                .get("delete")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                CurrentMutation
            } else {
                CurrentRead
            }
        }
        LearningSync => LocalDerived,
        Search => match input.get("kind").and_then(Value::as_str).unwrap_or("all") {
            "task" | "doc" => NonKnowledge,
            _ => CurrentRead,
        },
        AutoTaskAdd | AutoTaskList | AutoTaskShow | AutoTaskUpdate | AutoTaskToggle | DocsList
        | DocsShow | DocsAdd | DocsIndex | DocsMigrate | FrictionAdd | FrictionList
        | FrictionResolve | FrictionShow | FrictionStats | FrictionTags | FrictionUpdate
        | PipelineInvoke | PipelineWait | ReviewThreadAdd | ReviewThreadList
        | ReviewThreadReply | ReviewThreadResolve | SemanticIndex | SemanticInstall
        | SemanticStats | SemanticUninstall | StateGet | StateSet | TaskAdd | TaskApprove
        | TaskDelete | TaskLint | TaskList | TaskLocks | TaskLocksRelease | TaskLocksReserve
        | TaskReject | TaskShow | TaskStart | TaskUpdate => NonKnowledge,
    }
}

pub fn enforce(
    access: &KnowledgeOwnerAccess,
    class: KnowledgeSurfaceClass,
) -> Result<(), OrbitError> {
    use KnowledgeSurfaceClass::{CompositeCreate, CurrentMutation, CurrentRead, LocalDerived};

    match (access, class) {
        (_, KnowledgeSurfaceClass::NonKnowledge)
        | (KnowledgeOwnerAccess::Standalone, _)
        | (KnowledgeOwnerAccess::Owner { .. }, CurrentRead | CurrentMutation | LocalDerived)
        | (KnowledgeOwnerAccess::Replica { .. }, LocalDerived)
        | (KnowledgeOwnerAccess::Unavailable { .. }, LocalDerived) => Ok(()),
        (KnowledgeOwnerAccess::Owner { owner_machine_id }, CompositeCreate) => {
            Err(OrbitError::InvalidInput(format!(
                "multi-host knowledge creation must use the owner broker and hub allocator; owner={owner_machine_id}"
            )))
        }
        (
            KnowledgeOwnerAccess::Replica { owner_machine_id }
            | KnowledgeOwnerAccess::Unavailable { owner_machine_id },
            CompositeCreate | CurrentMutation,
        ) => Err(OrbitError::InvalidInput(format!(
            "current-state unavailable; owner={owner_machine_id}; route actionable work to the owner as a task"
        ))),
        (
            KnowledgeOwnerAccess::Replica { owner_machine_id }
            | KnowledgeOwnerAccess::Unavailable { owner_machine_id },
            CurrentRead,
        ) => Err(OrbitError::InvalidInput(format!(
            "current-state unavailable; owner={owner_machine_id}; replica files are not current"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replica_matrix_fails_closed_except_local_derived_sync() {
        let replica = KnowledgeOwnerAccess::Replica {
            owner_machine_id: "hm-owner".to_string(),
        };
        for action in [
            OrbitBuiltinAction::AdrAdd,
            OrbitBuiltinAction::AdrShow,
            OrbitBuiltinAction::AdrList,
            OrbitBuiltinAction::AdrUpdate,
            OrbitBuiltinAction::AdrSupersede,
            OrbitBuiltinAction::LearningAdd,
            OrbitBuiltinAction::LearningShow,
            OrbitBuiltinAction::LearningList,
            OrbitBuiltinAction::LearningUpdate,
            OrbitBuiltinAction::LearningSupersede,
        ] {
            let error = enforce(&replica, classify_builtin(action, &Value::Null))
                .expect_err("replica must fail closed")
                .to_string();
            assert!(error.contains("owner=hm-owner"), "{action:?}: {error}");
        }
        assert!(
            enforce(
                &replica,
                classify_builtin(OrbitBuiltinAction::LearningSync, &Value::Null)
            )
            .is_ok()
        );
        assert!(
            enforce(
                &replica,
                classify_builtin(OrbitBuiltinAction::LearningPrune, &json!({"delete": true}))
            )
            .is_err()
        );
    }
}
