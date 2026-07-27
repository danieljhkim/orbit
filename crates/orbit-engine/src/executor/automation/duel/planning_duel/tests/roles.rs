use orbit_common::types::RoleSlot;

use super::super::roles::{arbiter_input, planner_input_for_slot};

#[test]
fn planning_duel_inputs_thread_the_active_role_slot() {
    let planner_a = planner_input_for_slot("ORB-10393", RoleSlot::PlannerA);
    let planner_b = planner_input_for_slot("ORB-10393", RoleSlot::PlannerB);
    let arbiter = arbiter_input("ORB-10393");

    assert_eq!(planner_a["planning_duel_slot"], "planner_a");
    assert_eq!(planner_b["planning_duel_slot"], "planner_b");
    assert_eq!(arbiter["planning_duel_slot"], "arbiter");
    for input in [planner_a, planner_b, arbiter] {
        assert_eq!(input["task_id"], "ORB-10393");
    }
}
