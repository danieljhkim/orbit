use std::collections::BTreeSet;
use std::sync::Arc;

use orbit_common::types::{McpCapability, McpToolDefinition, OrbitError, ToolSessionContext};
use serde_json::Value;

use crate::{McpHost, McpServerComposition, McpSessionFactory};

struct SilentHost;

impl McpHost for SilentHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        Ok(Vec::new())
    }

    fn call_tool(
        &self,
        _name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        Ok(Value::Null)
    }
}

fn factory(trusted: ToolSessionContext) -> McpSessionFactory {
    McpSessionFactory::new(
        Arc::new(SilentHost) as Arc<dyn McpHost>,
        trusted,
        McpServerComposition::new(),
    )
}

#[test]
fn one_session_workspace_selection_is_invisible_to_another() {
    let factory = factory(ToolSessionContext::trusted_local(None, None, None));

    let first = factory.build_session();
    let second = factory.build_session();

    // This is exactly what `initialize` writes: the announced legacy selector,
    // installed on the server handling that request.
    let mut announced = first.session_context();
    announced.workspace = Some("/tmp/first-workspace".to_string());
    first.replace_session_context(announced);

    assert_eq!(
        first.session_context().workspace.as_deref(),
        Some("/tmp/first-workspace")
    );
    assert_eq!(second.session_context().workspace, None);
}

#[test]
fn every_session_gets_its_own_origin_session_id() {
    let mut trusted = ToolSessionContext::trusted_local(None, None, None);
    // A listener-wide correlation id must not survive into per-session state,
    // or concurrent clients would share one audit identity.
    trusted.origin_session_id = Some("listener-wide".to_string());
    let factory = factory(trusted);

    let first = factory.build_session().session_context();
    let second = factory.build_session().session_context();

    assert!(first.origin_session_id.is_some());
    assert!(second.origin_session_id.is_some());
    assert_ne!(first.origin_session_id, second.origin_session_id);
    assert_ne!(
        first.origin_session_id.as_deref(),
        Some("listener-wide"),
        "the template's correlation id must not be reused per session"
    );
    assert_eq!(first.mcp_call_id, None);
}

#[test]
fn sessions_carry_the_caller_selected_capability_set_verbatim() {
    let mut trusted = ToolSessionContext::trusted_local(None, None, None);
    trusted.effective_capabilities = BTreeSet::from([McpCapability::Runner]);
    let factory = factory(trusted);

    let session = factory.build_session().session_context();

    assert_eq!(
        session.effective_capabilities,
        BTreeSet::from([McpCapability::Runner]),
        "the factory neither supplies nor widens a capability set"
    );
}
