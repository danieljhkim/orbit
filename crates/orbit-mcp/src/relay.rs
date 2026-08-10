//! Byte-faithful MCP relay — the client end of [`crate::McpTcpServer`]
//! [ORB-10710].
//!
//! A client with no local checkout reaches a remote Orbit by registering an
//! ordinary stdio MCP server that does nothing but move bytes to a loopback
//! listener on the far side of an SSH tunnel ([ADR-0350],
//! [mcp-bridge/2_design.md §5.3]).
//!
//! Relaying is deliberately the *whole* job. This module parses no frames,
//! declares no schema, and reshapes no response: both ends are the same build,
//! so a response is byte-identical to the same call made against the listener
//! directly, and the schema drift a re-declaring parity layer would introduce is
//! structurally absent rather than tested for. Nothing here knows what a tool
//! is.
//!
//! Session lifetime follows the client. One relay serves one stdio session over
//! one already-established connection; the transport is not re-opened per call.

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use orbit_common::types::OrbitError;

/// Relay bytes in both directions between an MCP client and a server stream
/// until the session ends.
///
/// Shutdown is asymmetric on purpose. When the **server** stream ends the
/// session is over and the relay returns immediately, so the client observes
/// EOF on its own input. When the **client** ends (it closed stdin), the
/// server's write half is shut down — which is what tells the far side no more
/// requests are coming — and the response direction is then drained to
/// completion, so a reply already in flight is delivered rather than truncated.
pub async fn relay<C, S>(client: C, server: S) -> Result<(), OrbitError>
where
    C: AsyncRead + AsyncWrite,
    S: AsyncRead + AsyncWrite,
{
    let (mut client_read, mut client_write) = tokio::io::split(client);
    let (mut server_read, mut server_write) = tokio::io::split(server);

    let to_server = async {
        let copied = tokio::io::copy(&mut client_read, &mut server_write).await;
        // Half-close regardless of how the copy ended: a server blocked on a
        // read would otherwise hold the session open after the client is gone.
        let _ = server_write.shutdown().await;
        copied
    };
    let to_client = async {
        let copied = tokio::io::copy(&mut server_read, &mut client_write).await;
        let _ = client_write.flush().await;
        copied
    };

    tokio::pin!(to_server, to_client);
    let outcome = tokio::select! {
        result = &mut to_client => result.map(|_| ()),
        result = &mut to_server => match result {
            Ok(_) => (&mut to_client).await.map(|_| ()),
            Err(error) => Err(error),
        },
    };
    outcome.map_err(|error| OrbitError::Io(format!("mcp relay: {error}")))
}

/// Relay this process's stdio to `server`, presenting an ordinary stdio MCP
/// server to whatever launched us.
pub async fn relay_stdio<S>(server: S) -> Result<(), OrbitError>
where
    S: AsyncRead + AsyncWrite,
{
    relay(
        tokio::io::join(tokio::io::stdin(), tokio::io::stdout()),
        server,
    )
    .await
}

#[cfg(test)]
#[path = "tests/relay.rs"]
mod tests;
