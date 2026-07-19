use super::super::roles::{arbiter_activity, planner_activity};

#[test]
fn planning_duel_roles_do_not_require_graph_mcp_tools() {
    for activity in [planner_activity(), arbiter_activity()] {
        assert!(
            activity
                .tools
                .iter()
                .all(|tool| !tool.starts_with("orbit.graph.")),
            "{} must not grant removed graph MCP tools: {:?}",
            activity.id,
            activity.tools
        );
        assert!(activity.tools.iter().any(|tool| tool == "orbit.search"));
        assert!(activity.tools.iter().any(|tool| tool == "fs.read"));
        assert!(activity.proc_allowed_programs.is_empty());

        let instruction = activity.spec_config["instruction"]
            .as_str()
            .expect("planning-duel instruction");
        for removed_mandate in [
            "orbit.graph.",
            "map the call and import graph",
            "enumerate its callers and consumers BY NAME",
            "Reading symbol bodies alone is not enough",
        ] {
            assert!(
                !instruction.contains(removed_mandate),
                "{} still carries removed graph mandate {removed_mandate:?}",
                activity.id
            );
        }
    }
}
