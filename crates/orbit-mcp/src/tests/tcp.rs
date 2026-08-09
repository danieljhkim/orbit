use std::io::{Error, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use orbit_common::types::{McpToolDefinition, OrbitError, ToolSessionContext};
use serde_json::Value;

use super::{McpTcpServer, is_transient_accept_error};
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

fn factory() -> McpSessionFactory {
    McpSessionFactory::new(
        Arc::new(SilentHost) as Arc<dyn McpHost>,
        ToolSessionContext::trusted_local(None, None, None),
        McpServerComposition::new(),
    )
}

#[tokio::test]
async fn bind_reports_the_kernel_assigned_port() {
    let requested = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let server = McpTcpServer::bind(requested, factory())
        .await
        .expect("bind loopback");

    let bound = server.local_addr().expect("bound address");

    assert_eq!(bound.ip(), requested.ip());
    assert_ne!(bound.port(), 0, "an ephemeral bind must resolve to a port");
}

#[test]
fn only_per_connection_accept_failures_are_transient() {
    for kind in [
        ErrorKind::ConnectionAborted,
        ErrorKind::ConnectionReset,
        ErrorKind::Interrupted,
    ] {
        assert!(is_transient_accept_error(&Error::new(kind, "peer")));
    }
    // A listener-level failure must end the accept loop rather than spin on it.
    assert!(!is_transient_accept_error(&Error::new(
        ErrorKind::InvalidInput,
        "listener"
    )));
}
