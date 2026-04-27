//! charta — an MCP (Model Context Protocol) server that exposes SCXML state-chart
//! tooling (validation, Mermaid visualisation, and source-code generation) to
//! MCP-aware agents over stdio.
//!
//! The binary itself is a thin runtime wrapper: it constructs the [`Charta`]
//! handler, attaches it to the stdio transport, and waits for the MCP session
//! to terminate. All the real work lives in [`router`] and [`tools`].

use anyhow::Result;
use rmcp::{ServiceExt, transport::io::stdio};

// Module declarations. `router` holds the MCP `ServerHandler` implementation
// and tool definitions; `tools` holds the typed request/response payload
// structs and the shared `ToolError` type.
mod router;
mod tools;

use crate::router::Charta;

/// Program entry point.
///
/// Spins up a multi-threaded Tokio runtime (via the `#[tokio::main]` macro),
/// binds the default [`Charta`] handler to MCP-over-stdio, and blocks until
/// the peer disconnects. Any error during setup or while waiting bubbles up
/// as an `anyhow::Error` and exits the process non-zero.
#[tokio::main]
async fn main() -> Result<()> {
    // `serve(stdio())` performs the MCP handshake on stdin/stdout and returns
    // a running service handle; `waiting()` then blocks until the peer closes
    // the connection (typical MCP client lifecycle).
    Charta::default().serve(stdio()).await?.waiting().await?;
    Ok(())
}
