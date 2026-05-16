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
    match action {
        OrbitBuiltinAction::AdrAdd => super::adr_tools::add(runtime, input, agent, model),
        OrbitBuiltinAction::AdrShow => super::adr_tools::show(runtime, input),
        OrbitBuiltinAction::AdrList => super::adr_tools::list(runtime, input),
        OrbitBuiltinAction::AdrUpdate => super::adr_tools::update(runtime, input, agent, model),
        OrbitBuiltinAction::AdrSupersede => {
            super::adr_tools::supersede(runtime, input, agent, model)
        }
        OrbitBuiltinAction::DesignCheck => super::design_tools::check(runtime, input),
        OrbitBuiltinAction::DesignInit => super::design_tools::init(runtime, input, agent, model),
        OrbitBuiltinAction::DesignList => super::design_tools::list(runtime, input),
        OrbitBuiltinAction::DesignShow => super::design_tools::show(runtime, input),
        OrbitBuiltinAction::FrictionAdd => super::friction_tools::add(runtime, input, model),
        OrbitBuiltinAction::FrictionList => super::friction_tools::list(runtime, input),
        OrbitBuiltinAction::FrictionResolve => super::friction_tools::resolve(runtime, input),
        OrbitBuiltinAction::FrictionShow => super::friction_tools::show(runtime, input),
        OrbitBuiltinAction::FrictionStats => super::friction_tools::stats(runtime),
        OrbitBuiltinAction::FrictionTags => super::friction_tools::tags(runtime),
        OrbitBuiltinAction::FrictionUpdate => super::friction_tools::update(runtime, input),
        OrbitBuiltinAction::LearningAdd => super::learning_tools::add(runtime, input, agent, model),
        OrbitBuiltinAction::LearningList => super::learning_tools::list(runtime, input),
        OrbitBuiltinAction::LearningPrune => super::learning_tools::prune(runtime, input),
        OrbitBuiltinAction::LearningReindex => super::learning_tools::reindex(runtime, input),
        OrbitBuiltinAction::LearningSearch => super::learning_tools::search(runtime, input),
        OrbitBuiltinAction::LearningShow => super::learning_tools::show(runtime, input),
        OrbitBuiltinAction::LearningSupersede => {
            super::learning_tools::supersede(runtime, input, agent, model)
        }
        OrbitBuiltinAction::LearningUpdate => {
            super::learning_tools::update(runtime, input, agent, model)
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
        OrbitBuiltinAction::SemanticRelated => super::semantic_tools::related(runtime, input),
        OrbitBuiltinAction::SemanticSearch => super::semantic_tools::search(runtime, input),
        OrbitBuiltinAction::StateGet => super::state_tools::get(task_scope, input),
        OrbitBuiltinAction::StateSet => super::state_tools::set(task_scope, input),
        OrbitBuiltinAction::TaskAdd => super::task_tools::add(runtime, input, agent, model),
        OrbitBuiltinAction::TaskApprove => super::task_tools::approve(runtime, input, agent, model),
        OrbitBuiltinAction::TaskDelete => super::task_tools::delete(runtime, input),
        OrbitBuiltinAction::TaskLint => super::task_tools::lint(runtime, input),
        OrbitBuiltinAction::TaskList => super::task_tools::list(runtime, input),
        OrbitBuiltinAction::TaskSearch => super::task_tools::search(runtime, input),
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
    }
}
