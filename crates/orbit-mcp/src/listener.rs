//! TCP listener transport for the MCP kernel.
//!
//! The protocol handler performs no IO of its own, so serving it over a socket
//! is only a question of where the byte stream comes from, who owns the session
//! behind it, and which addresses may be bound. Framing, dispatch, and the
//! trusted session envelope are shared verbatim with stdio: this module adds no
//! capability, placement, routing, or authorization step of its own.

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;

use orbit_common::OrbitError;
use orbit_types::tool::ToolSessionContext;
use rmcp::ServiceExt;
use tokio::net::{TcpListener, TcpStream};

use crate::{McpHost, OrbitToolServer};

/// Loopback port `orbit mcp listen` binds when no address is given.
pub const DEFAULT_MCP_LISTEN_PORT: u16 = 7879;

/// How far a listener is allowed to be reachable.
///
/// The listener authenticates no one: whoever reaches the socket gets the
/// accepting machine's full tool surface. Restricting who can reach it is
/// therefore a deployment decision, and the safe default is the one that cannot
/// be reached off-box at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListenerExposure {
    /// Refuse any bind address that is not a loopback address.
    #[default]
    LoopbackOnly,
    /// Bind the requested address even when other hosts can reach it. The
    /// deployment has taken on restricting access by other means.
    AnyInterface,
}

/// A bound MCP endpoint that serves each accepted connection as its own
/// session.
pub struct McpListener {
    listener: TcpListener,
    host: Arc<dyn McpHost>,
    trusted_context: ToolSessionContext,
}

impl McpListener {
    /// Apply the bind policy and bind the endpoint without accepting anything
    /// yet.
    ///
    /// Binding is separated from accepting so a caller can report the assigned
    /// address — notably when binding port 0 — before connections arrive.
    pub async fn bind(
        addr: SocketAddr,
        exposure: ListenerExposure,
        host: Arc<dyn McpHost>,
        trusted_context: ToolSessionContext,
    ) -> Result<Self, OrbitError> {
        ensure_bind_allowed(addr, exposure)?;
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|error| OrbitError::Execution(format!("mcp listen bind {addr}: {error}")))?;
        Ok(Self {
            listener,
            host,
            trusted_context,
        })
    }

    /// The address actually bound, including any kernel-assigned port.
    pub fn local_addr(&self) -> Result<SocketAddr, OrbitError> {
        self.listener
            .local_addr()
            .map_err(|error| OrbitError::Execution(format!("mcp listen local address: {error}")))
    }

    /// Accept connections until the listener itself fails.
    ///
    /// Each connection is served on its own task with its own server instance.
    /// That isolation is load-bearing rather than defensive: the adapter's
    /// session state is written during `initialize`, so two clients sharing one
    /// instance would race on the announced workspace selector and the loser
    /// would receive a successful response computed against the other client's
    /// workspace.
    pub async fn serve(self) -> Result<(), OrbitError> {
        loop {
            let (stream, peer) = match self.listener.accept().await {
                Ok(accepted) => accepted,
                // A connection that died between the SYN and our accept is the
                // remote's problem. Anything else means the listener is no
                // longer usable, and spinning on it would burn the accept loop.
                Err(error) if is_transient_accept_error(&error) => {
                    tracing::warn!(error = %error, "mcp listener skipped a connection");
                    continue;
                }
                Err(error) => {
                    return Err(OrbitError::Execution(format!("mcp listen accept: {error}")));
                }
            };
            let server = OrbitToolServer::new_with_context(
                Arc::clone(&self.host),
                self.session_context_for(peer),
            );
            tokio::spawn(serve_connection(server, stream, peer));
        }
    }

    /// One session's trusted context: the listener's template, plus the peer
    /// address this process observed, minus the correlation identifiers so the
    /// adapter mints a fresh origin session id per connection. A listener-wide
    /// id would collapse concurrent clients into one audit identity.
    ///
    /// The peer address is a network observation, never an authenticated
    /// identity; it lands in the same audit field an SSH session fills from
    /// `SSH_CONNECTION`.
    fn session_context_for(&self, peer: SocketAddr) -> ToolSessionContext {
        let mut context = self.trusted_context.clone();
        context.origin_session_id = None;
        context.mcp_call_id = None;
        context.caller_ip = Some(peer.ip().to_string());
        context
    }
}

fn ensure_bind_allowed(addr: SocketAddr, exposure: ListenerExposure) -> Result<(), OrbitError> {
    if exposure == ListenerExposure::AnyInterface || addr.ip().is_loopback() {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "refusing to bind the MCP listener to non-loopback address {addr}: the listener \
         authenticates no client. Reach it through an authenticated tunnel, or pass \
         `--allow-non-loopback` once the network path is restricted by other means."
    )))
}

async fn serve_connection(server: OrbitToolServer, stream: TcpStream, peer: SocketAddr) {
    let running = match server.serve(stream).await {
        Ok(running) => running,
        Err(error) => {
            tracing::warn!(peer = %peer, error = %error, "mcp listener session did not start");
            return;
        }
    };
    if let Err(error) = running.waiting().await {
        tracing::warn!(peer = %peer, error = %error, "mcp listener session ended with an error");
    }
}

fn is_transient_accept_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset | ErrorKind::Interrupted
    )
}

#[cfg(test)]
#[path = "tests/listener.rs"]
mod tests;
