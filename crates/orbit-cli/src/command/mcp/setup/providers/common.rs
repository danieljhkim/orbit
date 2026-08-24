use crate::command::mcp::ORBIT_MCP_SERVER_ID;

pub(in crate::command::mcp::setup) const ORBIT_FEDERATED_MCP_SERVER_ID: &str = "orbit-federated";

/// The launch identity a generated client config is written for.
///
/// Local and federated servers are separate variants because the federated mux
/// cannot carry local operator authority or a local workspace binding. The
/// registration path chooses the variant explicitly, so bare `orbit mcp init`
/// cannot silently convert an existing v1 integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::command::mcp::setup) enum ServerLaunch<'a> {
    /// The v1 server, optionally operator-authorized and workspace-bound.
    Local {
        operator: bool,
        workspace: Option<&'a str>,
    },
    /// The session-unbound mux over operator-configured destinations.
    Federated,
}

impl Default for ServerLaunch<'_> {
    fn default() -> Self {
        Self::Local {
            operator: false,
            workspace: None,
        }
    }
}

impl<'a> ServerLaunch<'a> {
    pub(in crate::command::mcp::setup) const fn local(
        operator: bool,
        workspace: Option<&'a str>,
    ) -> Self {
        Self::Local {
            operator,
            workspace,
        }
    }
}

pub(super) fn server_id(launch: ServerLaunch<'_>) -> &'static str {
    match launch {
        ServerLaunch::Local { .. } => ORBIT_MCP_SERVER_ID,
        ServerLaunch::Federated => ORBIT_FEDERATED_MCP_SERVER_ID,
    }
}

/// The argv a generated client config launches Orbit's MCP server with.
pub(super) fn server_args(launch: ServerLaunch<'_>) -> Vec<String> {
    let mut args = vec!["mcp".to_string(), "serve".to_string()];
    match launch {
        ServerLaunch::Local {
            operator,
            workspace,
        } => {
            if operator {
                args.push("--operator".to_string());
            }
            if let Some(workspace) = workspace {
                args.push("--workspace".to_string());
                args.push(workspace.to_string());
            }
        }
        ServerLaunch::Federated => {
            args.push("--mode".to_string());
            args.push("federated".to_string());
        }
    }
    args
}
