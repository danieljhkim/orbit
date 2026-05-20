use orbit_common::types::OrbitError;
use orbit_tools::{OrbitBuiltinAction, OrbitTaskScope, ReservationOwnerContext};
use serde_json::Value;

use crate::OrbitRuntime;

pub(super) fn execute(
    runtime: &OrbitRuntime,
    task_scope: &OrbitTaskScope,
    action: OrbitBuiltinAction,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
    reservation_owner: Option<ReservationOwnerContext>,
) -> Result<Value, OrbitError> {
    let (input, redaction_report) = super::artifact_redaction::sanitize_tool_input(action, input)?;
    let agent_for_audit = agent.clone();
    let model_for_audit = model.clone();
    let mut response = match action {
        OrbitBuiltinAction::AdrAdd => super::adr_tools::add(runtime, input, agent, model),
        OrbitBuiltinAction::AdrShow => super::adr_tools::show(runtime, input),
        OrbitBuiltinAction::AdrList => super::adr_tools::list(runtime, input),
        OrbitBuiltinAction::AdrUpdate => super::adr_tools::update(runtime, input, agent, model),
        OrbitBuiltinAction::AdrSupersede => {
            super::adr_tools::supersede(runtime, input, agent, model)
        }
        OrbitBuiltinAction::DocsList => super::docs_tools::list(runtime, input),
        OrbitBuiltinAction::DocsShow => super::docs_tools::show(runtime, input),
        OrbitBuiltinAction::DocsAdd => super::docs_tools::add(runtime, input),
        OrbitBuiltinAction::DocsReindex => super::docs_tools::reindex(runtime, input),
        OrbitBuiltinAction::DocsMigrate => super::docs_tools::migrate(runtime, input),
        OrbitBuiltinAction::FrictionAdd => super::friction_tools::add(runtime, input, model),
        OrbitBuiltinAction::FrictionList => super::friction_tools::list(runtime, input),
        OrbitBuiltinAction::FrictionResolve => super::friction_tools::resolve(runtime, input),
        OrbitBuiltinAction::FrictionShow => super::friction_tools::show(runtime, input),
        OrbitBuiltinAction::FrictionStats => super::friction_tools::stats(runtime),
        OrbitBuiltinAction::FrictionTags => super::friction_tools::tags(runtime),
        OrbitBuiltinAction::FrictionUpdate => super::friction_tools::update(runtime, input),
        OrbitBuiltinAction::LearningAdd => super::learning_tools::add(runtime, input, agent, model),
        OrbitBuiltinAction::LearningCommentAdd => {
            super::learning_tools::comment_add(runtime, input, agent, model)
        }
        OrbitBuiltinAction::LearningCommentDelete => {
            super::learning_tools::comment_delete(runtime, input, agent, model)
        }
        OrbitBuiltinAction::LearningCommentList => {
            super::learning_tools::comment_list(runtime, input)
        }
        OrbitBuiltinAction::LearningList => super::learning_tools::list(runtime, input),
        OrbitBuiltinAction::LearningPrune => super::learning_tools::prune(runtime, input),
        OrbitBuiltinAction::LearningReindex => super::learning_tools::reindex(runtime, input),
        OrbitBuiltinAction::LearningShow => super::learning_tools::show(runtime, input),
        OrbitBuiltinAction::LearningSupersede => {
            super::learning_tools::supersede(runtime, input, agent, model)
        }
        OrbitBuiltinAction::LearningUpdate => {
            super::learning_tools::update(runtime, input, agent, model)
        }
        OrbitBuiltinAction::LearningUpvote => {
            super::learning_tools::upvote(runtime, input, agent, model)
        }
        OrbitBuiltinAction::PipelineInvoke => {
            super::pipeline_tools::invoke(runtime, input, agent, model)
        }
        OrbitBuiltinAction::PipelineWait => {
            super::pipeline_tools::wait(runtime, input, agent, model)
        }
        OrbitBuiltinAction::ReviewThreadAdd => {
            super::review_threads::add(runtime, input, agent, model)
        }
        OrbitBuiltinAction::ReviewThreadList => super::review_threads::list(runtime, input),
        OrbitBuiltinAction::ReviewThreadReply => {
            super::review_threads::reply(runtime, input, agent, model)
        }
        OrbitBuiltinAction::ReviewThreadResolve => {
            super::review_threads::resolve(runtime, input, agent, model)
        }
        OrbitBuiltinAction::Search => super::search_tools::search(runtime, input),
        OrbitBuiltinAction::SemanticIndex => super::semantic_tools::index(runtime, input),
        OrbitBuiltinAction::SemanticInstall => super::semantic_tools::install(runtime, input),
        OrbitBuiltinAction::SemanticStats => super::semantic_tools::stats(runtime),
        OrbitBuiltinAction::SemanticUninstall => super::semantic_tools::uninstall(runtime, input),
        OrbitBuiltinAction::StateGet => super::state_tools::get(task_scope, input),
        OrbitBuiltinAction::StateSet => super::state_tools::set(task_scope, input),
        OrbitBuiltinAction::TaskAdd => super::task_tools::add(runtime, input, agent, model),
        OrbitBuiltinAction::TaskApprove => super::task_tools::approve(runtime, input, agent, model),
        OrbitBuiltinAction::TaskDelete => super::task_tools::delete(runtime, input),
        OrbitBuiltinAction::TaskLint => super::task_tools::lint(runtime, input),
        OrbitBuiltinAction::TaskList => super::task_tools::list(runtime, input),
        OrbitBuiltinAction::TaskLocks => super::task_locks::list(runtime),
        OrbitBuiltinAction::TaskLocksRelease => {
            super::task_locks::release(runtime, input, agent, model)
        }
        OrbitBuiltinAction::TaskLocksReserve => {
            super::task_locks::reserve(runtime, input, agent, model, reservation_owner)
        }
        OrbitBuiltinAction::TaskReject => super::task_tools::reject(runtime, input, agent, model),
        OrbitBuiltinAction::TaskShow => super::task_tools::show(runtime, input),
        OrbitBuiltinAction::TaskStart => super::task_tools::start(runtime, input, agent, model),
        OrbitBuiltinAction::TaskUpdate => super::task_tools::update(runtime, input, agent, model),
    }?;
    super::artifact_redaction::finish_tool_response(
        runtime,
        action,
        &mut response,
        &redaction_report,
        agent_for_audit.as_deref(),
        model_for_audit.as_deref(),
    )?;
    Ok(response)
}
