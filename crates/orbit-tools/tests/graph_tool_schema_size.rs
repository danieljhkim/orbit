use orbit_tools::ToolRegistry;

const BASELINE_BYTES: usize = 10_995;
const MAX_BYTES: usize = 8_246;

#[test]
fn graph_tool_schema_bytes_stay_under_budget() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    let total: usize = registry
        .schemas()
        .into_iter()
        .filter(|schema| schema.name.starts_with("orbit.graph."))
        .map(|schema| serde_json::to_string(&schema).unwrap().len())
        .sum();

    assert!(
        total <= MAX_BYTES,
        "graph tool schema bytes grew to {total} (baseline {BASELINE_BYTES}, max {MAX_BYTES})"
    );
}
