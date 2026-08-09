//! Streamable HTTP service construction for an embedding HTTP server.

use std::sync::Arc;

use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

use crate::{McpSessionFactory, OrbitToolServer};

/// Tower service implementing stateful MCP Streamable HTTP.
pub type McpStreamableHttpService = StreamableHttpService<OrbitToolServer, LocalSessionManager>;

/// Cancellation handle for every session owned by one HTTP service.
///
/// The embedding server must cancel this before waiting for its HTTP listener
/// to drain. Otherwise an open MCP SSE stream can keep graceful shutdown alive
/// indefinitely.
#[derive(Clone)]
pub struct McpHttpServerControl {
    cancel: Arc<dyn Fn() + Send + Sync>,
}

impl McpHttpServerControl {
    /// Stop accepting MCP requests and terminate every active session.
    pub fn cancel(&self) {
        (self.cancel)();
    }
}

/// Build a stateful Streamable HTTP service with one isolated MCP server per
/// session and a handle that participates in the embedding server's shutdown.
///
/// The rmcp default retains DNS-rebinding protection by accepting only
/// loopback `Host` authorities. Origin policy is deliberately left to the
/// embedding router: MCP clients are not browsers and may send any `Origin`.
pub fn streamable_http_service(
    factory: McpSessionFactory,
) -> (McpStreamableHttpService, McpHttpServerControl) {
    let config = StreamableHttpServerConfig::default();
    let cancellation_token = config.cancellation_token.clone();
    let control = McpHttpServerControl {
        cancel: Arc::new(move || cancellation_token.cancel()),
    };
    let service = StreamableHttpService::new(
        move || Ok(factory.build_session()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    (service, control)
}
