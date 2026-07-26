//! Guardrail: the governed-operation registry names real tools [ORB-10453].
//!
//! `GOVERNED_OPERATIONS` is declared in `orbit-common`, which sits below the
//! tool registry and so cannot check itself against it. The registry is keyed by
//! string, so a renamed or deleted tool would silently stop being governed —
//! the failure mode is a *disappearing* guard, which no behavioural test
//! notices because nothing starts failing. This is the test that does.

use orbit_common::authorization::{GOVERNED_OPERATIONS, OperationSurface};

use crate::ToolRegistry;

fn builtin_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    registry
}

#[test]
fn every_governed_tool_operation_names_a_registered_tool() {
    let registry = builtin_registry();

    for operation in GOVERNED_OPERATIONS
        .iter()
        .filter(|operation| operation.surface == OperationSurface::Tool)
    {
        assert!(
            registry.has(operation.id),
            "governed operation '{}' names no registered tool — a rename dropped its guard \
             silently, since a governed id that matches nothing simply never denies",
            operation.id
        );
    }
}

#[test]
fn the_destructive_builtins_this_task_closed_are_still_governed() {
    // These are the tools ORB-10453 found reachable through `orbit tool run`
    // despite being hidden from the MCP surface. Pinning them by name keeps a
    // future refactor from quietly returning them to the ungoverned set.
    let governed: Vec<&str> = GOVERNED_OPERATIONS
        .iter()
        .filter(|operation| operation.surface == OperationSurface::Tool)
        .map(|operation| operation.id)
        .collect();

    for expected in [
        "orbit.task.delete",
        "orbit.task.reject",
        "orbit.task.locks.release",
        "orbit.learning.prune",
        "orbit.semantic.uninstall",
    ] {
        assert!(
            governed.contains(&expected),
            "'{expected}' must stay governed; currently governed: {governed:?}"
        );
    }
}
