use std::collections::BTreeSet;

use super::*;

fn envelope() -> CallerEnvelope {
    CallerEnvelope::default()
}

fn operation(id: &str) -> &'static GovernedOperation {
    GOVERNED_OPERATIONS
        .iter()
        .find(|operation| operation.id == id)
        .expect("governed operation is declared")
}

#[test]
fn registry_declares_each_operation_once_with_a_capability() {
    let mut seen = BTreeSet::new();
    for operation in GOVERNED_OPERATIONS {
        assert!(
            seen.insert((operation.surface, operation.id)),
            "duplicate governed operation: {}",
            operation.id
        );
        assert!(
            !operation.allowed.is_empty(),
            "governed operation '{}' allows no capability, which would deny every caller",
            operation.id
        );
        assert!(
            !operation.rationale.trim().is_empty(),
            "governed operation '{}' has no rationale to show a denied caller",
            operation.id
        );
        match operation.surface {
            OperationSurface::Tool => assert!(
                !operation.id.contains(' '),
                "tool operation '{}' must be a canonical tool name",
                operation.id
            ),
            OperationSurface::CliCommand => assert!(
                operation.id.split(' ').count() == 2,
                "command operation '{}' must be `<command> <subcommand>`",
                operation.id
            ),
        }
    }
}

#[test]
fn lookup_is_surface_scoped() {
    assert!(governed_tool("orbit.task.delete").is_some());
    assert!(governed_tool("workspace teardown").is_none());
    assert!(governed_command("workspace", "teardown").is_some());
    assert!(governed_command("orbit.task.delete", "").is_none());
    assert!(governed_tool("orbit.task.show").is_none());
    assert!(governed_command("workspace", "list").is_none());
}

#[test]
fn session_grants_win_over_process_signals() {
    let caller = CallerCapabilities::resolve(&CallerEnvelope {
        session_capabilities: BTreeSet::from([McpCapability::Runner]),
        agent_declared: true,
        interactive_terminal: true,
        operator_override: true,
    });
    assert_eq!(caller.provenance(), CallerProvenance::Session);
    assert_eq!(caller.grants(), &BTreeSet::from([McpCapability::Runner]));
}

#[test]
fn agent_envelope_outranks_a_terminal() {
    let caller = CallerCapabilities::resolve(&CallerEnvelope {
        agent_declared: true,
        interactive_terminal: true,
        ..envelope()
    });
    assert_eq!(caller.provenance(), CallerProvenance::AgentEnvelope);
    assert_eq!(caller.grants(), &BTreeSet::from([McpCapability::Agent]));
}

#[test]
fn override_outranks_an_agent_envelope_and_is_marked() {
    let caller = CallerCapabilities::resolve(&CallerEnvelope {
        operator_override: true,
        agent_declared: true,
        ..envelope()
    });
    assert_eq!(caller.provenance(), CallerProvenance::OperatorOverride);
    assert!(caller.is_override());
    assert!(authorize(operation("orbit.task.delete"), &caller).is_ok());
}

#[test]
fn a_terminal_resolves_to_operator() {
    let caller = CallerCapabilities::resolve(&CallerEnvelope {
        interactive_terminal: true,
        ..envelope()
    });
    assert_eq!(caller.provenance(), CallerProvenance::InteractiveTerminal);
    assert!(authorize(operation("workspace teardown"), &caller).is_ok());
}

#[test]
fn an_unidentified_caller_is_denied() {
    let caller = CallerCapabilities::resolve(&envelope());
    assert_eq!(caller.provenance(), CallerProvenance::Unknown);
    assert!(caller.grants().is_empty());

    let denial = authorize(operation("orbit.task.delete"), &caller).expect_err("must deny");
    assert_eq!(denial.provenance, CallerProvenance::Unknown);
    assert_eq!(denial.granted, "none");
}

#[test]
fn an_agent_is_denied_out_of_scope_destruction() {
    let caller = CallerCapabilities::resolve(&CallerEnvelope {
        agent_declared: true,
        ..envelope()
    });
    for id in [
        "orbit.task.delete",
        "orbit.task.reject",
        "orbit.semantic.uninstall",
    ] {
        assert!(
            authorize(operation(id), &caller).is_err(),
            "agent must not reach '{id}'"
        );
    }
}

#[test]
fn a_run_retains_the_operations_it_dispatches() {
    let caller = CallerCapabilities::resolve(&CallerEnvelope {
        session_capabilities: BTreeSet::from([McpCapability::Runner]),
        ..envelope()
    });
    assert!(authorize(operation("orbit.task.locks.release"), &caller).is_ok());
    assert!(authorize(operation("gc worktrees"), &caller).is_ok());
    // Runner sanctions what the run dispatches, not everything destructive.
    assert!(authorize(operation("orbit.task.delete"), &caller).is_err());
}

#[test]
fn denial_names_the_capability_the_rationale_and_the_escape_hatch() {
    let caller = CallerCapabilities::resolve(&CallerEnvelope {
        agent_declared: true,
        ..envelope()
    });
    let denial =
        authorize(operation("orbit.task.locks.release"), &caller).expect_err("agent is denied");
    let message = denial.to_string();

    assert!(message.contains("orbit.task.locks.release"), "{message}");
    assert!(message.contains("operator or runner"), "{message}");
    assert!(message.contains("same files"), "{message}");
    assert!(message.contains("agent-envelope"), "{message}");
    assert!(message.contains(OPERATOR_OVERRIDE_ENV), "{message}");
}

#[test]
fn ungoverned_operations_have_no_registry_entry_to_enforce() {
    // The registry is opt-in: an operation absent from it is not gated. This
    // pins retained public operations without granting any retired VCS/PR
    // operation through authorization.
    for tool in ["git.commit", "fs.write", "proc.spawn"] {
        assert!(
            governed_tool(tool).is_none(),
            "'{tool}' remains outside the governed-operation registry"
        );
    }
}
