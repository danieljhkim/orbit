//! TCP listener transport for the MCP kernel.
//!
//! The protocol handler performs no IO of its own, so serving it over a socket
//! is only a question of where the byte stream comes from and who owns the
//! session behind it. Framing, dispatch, capability filtering, and the trusted
//! session envelope are shared verbatim with stdio.

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;

use orbit_common::types::{OrbitError, ToolSessionContext};
use rmcp::ServiceExt;
use tokio::net::{TcpListener, TcpStream};

use crate::session::McpSessionFactory;
use crate::{McpHost, McpServerComposition, OrbitToolServer};

/// A bound MCP endpoint that serves each accepted connection as its own
/// session.
///
/// The endpoint carries no authentication of its own: it serves the trusted
/// local session context it was constructed with to whoever reaches the socket,
/// so restricting who can reach it (loopback bind, unix-level firewalling, an
/// authenticating proxy) is the deployment's responsibility. Bind to loopback
/// unless the deployment has taken that on.
pub struct McpTcpServer {
    listener: TcpListener,
    factory: McpSessionFactory,
}

impl McpTcpServer {
    /// Bind the endpoint without accepting anything yet.
    ///
    /// Binding is separated from accepting so a caller can learn the assigned
    /// address (notably when binding port 0) before connections start arriving.
    pub async fn bind(addr: SocketAddr, factory: McpSessionFactory) -> Result<Self, OrbitError> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|error| OrbitError::Execution(format!("mcp tcp bind {addr}: {error}")))?;
        Ok(Self { listener, factory })
    }

    /// The address actually bound, including the kernel-assigned port.
    pub fn local_addr(&self) -> Result<SocketAddr, OrbitError> {
        self.listener
            .local_addr()
            .map_err(|error| OrbitError::Execution(format!("mcp tcp local address: {error}")))
    }

    /// Accept connections until the listener fails.
    ///
    /// Each connection is served on its own task with its own server instance,
    /// so one client's initialize, workspace selection, and name map are
    /// unreachable from another's. A session that dies takes nothing else down
    /// with it.
    pub async fn serve(self) -> Result<(), OrbitError> {
        loop {
            let (stream, peer) = match self.listener.accept().await {
                Ok(accepted) => accepted,
                // A connection that died between the SYN and our accept is the
                // remote's problem, not the listener's; anything else means the
                // listener itself is no longer usable and spinning on it would
                // just burn the accept loop.
                Err(error) if is_transient_accept_error(&error) => {
                    tracing::warn!(error = %error, "mcp tcp accept skipped a connection");
                    continue;
                }
                Err(error) => {
                    return Err(OrbitError::Execution(format!("mcp tcp accept: {error}")));
                }
            };
            let server = self.factory.build_session();
            tokio::spawn(serve_connection(server, stream, peer));
        }
    }
}

/// Bind and serve MCP over TCP from a complete generic server composition.
///
/// The trusted session context — including its effective capability set — is
/// the caller's to choose; there is deliberately no capability-free convenience
/// form of this entry point.
pub async fn serve_tcp_with_context_and_composition(
    addr: SocketAddr,
    host: Arc<dyn McpHost>,
    trusted_context: ToolSessionContext,
    composition: McpServerComposition,
) -> Result<(), OrbitError> {
    let factory = McpSessionFactory::new(host, trusted_context, composition);
    McpTcpServer::bind(addr, factory).await?.serve().await
}

async fn serve_connection(server: OrbitToolServer, stream: TcpStream, peer: SocketAddr) {
    let running = match server.serve(stream).await {
        Ok(running) => running,
        Err(error) => {
            tracing::warn!(peer = %peer, error = %error, "mcp tcp session did not start");
            return;
        }
    };
    if let Err(error) = running.waiting().await {
        tracing::warn!(peer = %peer, error = %error, "mcp tcp session ended with an error");
    }
}

fn is_transient_accept_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset | ErrorKind::Interrupted
    )
}

#[cfg(test)]
#[path = "tests/tcp.rs"]
mod tests;
