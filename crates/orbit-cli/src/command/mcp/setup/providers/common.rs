/// The argv a generated client config launches Orbit's MCP server with.
///
/// `operator` is the one authority signal threaded through config generation:
/// `true` produces `mcp serve --operator` (the `orbit workspace init --mcp`
/// bootstrap path), `false` produces plain `mcp serve` (bare `orbit mcp
/// init`). Every caller passes it explicitly so no registration path silently
/// upgrades.
pub(super) fn server_args(operator: bool) -> Vec<String> {
    let mut args = vec!["mcp".to_string(), "serve".to_string()];
    if operator {
        args.push("--operator".to_string());
    }
    args
}
