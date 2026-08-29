//! Thin SSH transport and remote-machine MCP support data.

mod callers;
mod discovery;
mod identity;
mod proxy;
mod ssh_auth;
mod surface;

#[cfg(test)]
mod tests;

pub use self::callers::{
    CALLERS_FILE, CALLERS_FILE_DISPLAY, CallerAuthorizationHealth, CallerRow, CallersFile,
    DefaultGrant, RemoteCallerIdentity, ResolvedCallerGrant, SeedCaller, SessionCapabilityPolicy,
    callers_path, inspect_caller_authorization, load_callers, remote_originated,
    render_callers_seed, write_callers_seed,
};
pub use self::discovery::{
    FEDERATED_DESTINATION_WORKSPACE_LIST_TOOL, execute_discovery_tool,
    execute_federated_workspace_discovery,
};
pub use self::identity::{
    McpServerIdentity, McpSessionAuthority, mcp_serve_session_policy, mcp_server_identity,
};
pub use self::proxy::{RemoteProxyArgs, serve_mcp_remote_proxy};
pub use self::ssh_auth::{
    FORCED_COMMAND_RESTRICTIONS, SshAcceptance, SshPublicKey, parse_public_key,
};
// The federated mux reuses the v1 remote argv verbatim rather than restating
// it, so both client paths present one session shape to a destination.
pub(crate) use self::proxy::remote_serve_command;
pub use self::surface::{canonical_mcp_tool_definitions, safe_mcp_tool_names};
