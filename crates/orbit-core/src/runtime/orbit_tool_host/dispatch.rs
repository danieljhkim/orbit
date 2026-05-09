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
