/// The launch identity a generated client config is written for.
///
/// Both fields answer "what is this integration for", and both are decided by
/// the registration path rather than by the client that later connects. Every
/// caller names them explicitly so no path silently upgrades authority or
/// silently drops the workspace binding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::command::mcp::setup) struct ServerLaunch<'a> {
    /// `true` produces `mcp serve --operator` (the `orbit workspace init
    /// --mcp` bootstrap path), `false` produces plain `mcp serve` (bare
    /// `orbit mcp init`).
    pub(in crate::command::mcp::setup) operator: bool,
    /// The workspace this integration was registered for, emitted as
    /// `--workspace <selector>`. `None` when the checkout is not registered on
    /// this machine, leaving the generated session unbound and every
    /// workspace-scoped call responsible for its own selector.
    pub(in crate::command::mcp::setup) workspace: Option<&'a str>,
}

/// The argv a generated client config launches Orbit's MCP server with.
pub(super) fn server_args(launch: ServerLaunch<'_>) -> Vec<String> {
    let mut args = vec!["mcp".to_string(), "serve".to_string()];
    if launch.operator {
        args.push("--operator".to_string());
    }
    if let Some(workspace) = launch.workspace {
        args.push("--workspace".to_string());
        args.push(workspace.to_string());
    }
    args
}
