#![deny(clippy::print_stderr, clippy::print_stdout)]
// Internal MCP kernel surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! Generic Model Context Protocol transport kernel for caller-supplied tool,
//! schema, context, result-decoration, and custom-request compositions.
//!
//! # Role
//! Depends only on `orbit-common` for Orbit-neutral wire types. Feature and
//! domain policy belongs in higher-level composition crates.
//!
//! # Transport
//! Stdio and TCP are supported. The protocol handler performs no IO, so the
//! two transports share framing, dispatch, and capability filtering unchanged;
//! they differ only in how a byte stream arrives and in how many sessions one
//! server process owns. HTTP/SSE/streamable-http remain follow-up work.
//! Authenticating a network endpoint is the deployment's concern, not this
//! crate's — see [`McpTcpServer`].

mod adapter;
mod client;
mod error;
mod session;
mod tcp;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use orbit_common::types::{
    McpToolDefinition, NotFoundKind, OrbitError, ToolParam, ToolSessionContext, audit_execution_id,
};
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;
use serde_json::{Map, Value};

pub use adapter::OrbitToolServer;
pub use client::{
    McpClientInitialization, McpClientRequestError, McpToolResponse, RawOrbitMcpClient,
};
pub use session::McpSessionFactory;
pub use tcp::{McpTcpServer, serve_tcp_with_context_and_composition};

/// Complete JSON input schema advertised for one MCP tool.
pub type McpInputSchema = Map<String, Value>;

/// Encode Orbit's compatibility input schema for one canonical tool.
///
/// Higher-level feature crates may instead provide a complete schema through
/// [`McpToolExtension::input_schema`].
pub fn encode_mcp_input_schema(tool_name: &str, params: &[ToolParam]) -> McpInputSchema {
    adapter::encode_mcp_input_schema(tool_name, params)
}

/// Encode an input schema with a caller-owned enum-value resolver.
///
/// This keeps JSON-schema framing in the generic MCP kernel while allowing a
/// feature-owned extension to keep its enum metadata adjacent to its tool
/// definitions.
pub fn encode_mcp_input_schema_with_enum_values<F>(
    tool_name: &str,
    params: &[ToolParam],
    enum_values: F,
) -> McpInputSchema
where
    F: Fn(&str, &str) -> Option<&'static [&'static str]>,
{
    adapter::encode_mcp_input_schema_with_enum_values(tool_name, params, enum_values)
}

/// Resolves the complete wire input schema for host-owned MCP definitions.
///
/// The kernel default is deliberately structural: feature crates may attach
/// schema-adjacent enum metadata without teaching this transport crate any
/// domain-specific tool names.
pub trait McpInputSchemaResolver: Send + Sync + 'static {
    fn input_schema(&self, definition: &McpToolDefinition) -> Result<McpInputSchema, OrbitError>;
}

/// Structural schema resolver used by generic MCP compositions.
#[derive(Debug, Default)]
pub struct StructuralMcpInputSchemaResolver;

impl McpInputSchemaResolver for StructuralMcpInputSchemaResolver {
    fn input_schema(&self, definition: &McpToolDefinition) -> Result<McpInputSchema, OrbitError> {
        Ok(encode_mcp_input_schema(
            &definition.schema.name,
            &definition.schema.parameters,
        ))
    }
}

/// An in-process MCP tool implementation composed into [`OrbitToolServer`].
///
/// Extensions own their canonical definitions and name recognition. Calls
/// still cross [`McpHost::call_in_process_tool`] before this implementation is
/// invoked, so composing an extension does not bypass host policy or auditing.
pub trait McpToolExtension: Send + Sync + 'static {
    fn definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError>;

    /// Return whether this extension owns the canonical tool name.
    ///
    /// Recognition is deliberately independent of advertisement. A hidden or
    /// disabled extension can therefore keep guessed calls on the host's
    /// audited in-process denial path.
    fn recognizes(&self, name: &str) -> bool;

    fn call(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError>;

    /// Encode the complete input schema for one definition owned by this
    /// extension. The compatibility default preserves the kernel's existing
    /// schema encoding; feature extensions may override it completely.
    fn input_schema(&self, definition: &McpToolDefinition) -> Result<McpInputSchema, OrbitError> {
        Ok(encode_mcp_input_schema(
            &definition.schema.name,
            &definition.schema.parameters,
        ))
    }

    /// Preserve implementation-specific diagnostics after the host boundary
    /// returns a call failure. Generic extensions need not emit anything.
    fn report_call_failure(&self, _name: &str, _error: &OrbitError) {}
}

/// Identifies the request whose trusted per-call context is being resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpRequestKind {
    Tool { name: String },
    Custom { method: String },
}

/// Resolves one trusted per-call context from server-owned state and opaque
/// transport metadata.
pub trait McpCallContextResolver: Send + Sync + 'static {
    fn resolve(
        &self,
        trusted_context: &ToolSessionContext,
        request: &McpRequestKind,
        transport_metadata: &Map<String, Value>,
    ) -> Result<ToolSessionContext, OrbitError>;
}

/// Default resolver for ordinary local MCP sessions.
#[derive(Debug, Default)]
pub struct LocalMcpCallContextResolver;

impl McpCallContextResolver for LocalMcpCallContextResolver {
    fn resolve(
        &self,
        trusted_context: &ToolSessionContext,
        _request: &McpRequestKind,
        _transport_metadata: &Map<String, Value>,
    ) -> Result<ToolSessionContext, OrbitError> {
        let mut context = trusted_context.clone();
        context.mcp_call_id = Some(audit_execution_id("mcall"));
        Ok(context)
    }
}

/// Error classification returned by an in-process custom MCP request handler.
#[derive(Debug, Clone, PartialEq)]
pub enum McpCustomRequestError {
    MethodNotFound,
    InvalidParams {
        message: String,
        data: Option<Value>,
    },
    Internal {
        message: String,
        data: Option<Value>,
    },
}

impl McpCustomRequestError {
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::InvalidParams {
            message: message.into(),
            data: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            data: None,
        }
    }
}

/// Handles one family of non-tool MCP requests.
pub trait McpCustomRequestHandler: Send + Sync + 'static {
    fn recognizes(&self, method: &str) -> bool;

    /// Stable label used when the blocking handler worker itself fails.
    fn worker_label(&self) -> &'static str {
        "MCP custom request"
    }

    fn call(
        &self,
        method: &str,
        params: Option<Value>,
        session_context: ToolSessionContext,
    ) -> Result<Value, McpCustomRequestError>;
}

/// Successful tool-call data presented to a post-call result decorator.
#[derive(Debug, Clone)]
pub struct McpResultDecoration {
    pub canonical_name: String,
    pub input: Value,
    pub output: Value,
    pub call_context: ToolSessionContext,
    pub server_context: ToolSessionContext,
}

/// Stable adapter-level error returned by a result decorator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpResultDecorationError {
    message: String,
}

impl McpResultDecorationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for McpResultDecorationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for McpResultDecorationError {}

pub type McpResultDecorationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Value, McpResultDecorationError>> + Send + 'a>>;

/// Decorates a successful tool result before MCP structured framing.
pub trait McpResultDecorator: Send + Sync + 'static {
    fn decorate(&self, call: McpResultDecoration) -> McpResultDecorationFuture<'_>;
}

/// Opaque initialize metadata supplied by server composition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpServerMetadata {
    instructions: Option<String>,
}

impl McpServerMetadata {
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }
}

/// Complete generic composition for one [`OrbitToolServer`].
#[derive(Clone)]
pub struct McpServerComposition {
    tool_extensions: Vec<McpToolExtensionRegistration>,
    result_decorators: Vec<Arc<dyn McpResultDecorator>>,
    call_context_resolver: Arc<dyn McpCallContextResolver>,
    input_schema_resolver: Arc<dyn McpInputSchemaResolver>,
    custom_request_handlers: Vec<Arc<dyn McpCustomRequestHandler>>,
    metadata: McpServerMetadata,
}

pub(crate) struct McpServerCompositionParts {
    pub(crate) tool_extensions: Vec<McpToolExtensionRegistration>,
    pub(crate) result_decorators: Vec<Arc<dyn McpResultDecorator>>,
    pub(crate) call_context_resolver: Arc<dyn McpCallContextResolver>,
    pub(crate) input_schema_resolver: Arc<dyn McpInputSchemaResolver>,
    pub(crate) custom_request_handlers: Vec<Arc<dyn McpCustomRequestHandler>>,
    pub(crate) metadata: McpServerMetadata,
}

impl McpServerComposition {
    pub fn new() -> Self {
        Self {
            tool_extensions: Vec::new(),
            result_decorators: Vec::new(),
            call_context_resolver: Arc::new(LocalMcpCallContextResolver),
            input_schema_resolver: Arc::new(StructuralMcpInputSchemaResolver),
            custom_request_handlers: Vec::new(),
            metadata: McpServerMetadata::default(),
        }
    }

    pub fn with_tool_extension(mut self, extension: McpToolExtensionRegistration) -> Self {
        self.tool_extensions.push(extension);
        self
    }

    pub fn with_tool_extensions(
        mut self,
        extensions: impl IntoIterator<Item = McpToolExtensionRegistration>,
    ) -> Self {
        self.tool_extensions.extend(extensions);
        self
    }

    pub fn with_result_decorator(mut self, decorator: Arc<dyn McpResultDecorator>) -> Self {
        self.result_decorators.push(decorator);
        self
    }

    pub fn with_call_context_resolver(mut self, resolver: Arc<dyn McpCallContextResolver>) -> Self {
        self.call_context_resolver = resolver;
        self
    }

    pub fn with_input_schema_resolver(mut self, resolver: Arc<dyn McpInputSchemaResolver>) -> Self {
        self.input_schema_resolver = resolver;
        self
    }

    pub fn with_custom_request_handler(
        mut self,
        handler: Arc<dyn McpCustomRequestHandler>,
    ) -> Self {
        self.custom_request_handlers.push(handler);
        self
    }

    pub fn with_metadata(mut self, metadata: McpServerMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub(crate) fn into_parts(self) -> McpServerCompositionParts {
        McpServerCompositionParts {
            tool_extensions: self.tool_extensions,
            result_decorators: self.result_decorators,
            call_context_resolver: self.call_context_resolver,
            input_schema_resolver: self.input_schema_resolver,
            custom_request_handlers: self.custom_request_handlers,
            metadata: self.metadata,
        }
    }
}

impl Default for McpServerComposition {
    fn default() -> Self {
        Self::new()
    }
}

/// Composition metadata for one in-process MCP tool extension.
#[derive(Clone)]
pub struct McpToolExtensionRegistration {
    extension: Arc<dyn McpToolExtension>,
    advertise_definitions: bool,
}

impl McpToolExtensionRegistration {
    /// Register an extension whose definitions are included in `tools/list`.
    pub fn advertised(extension: Arc<dyn McpToolExtension>) -> Self {
        Self {
            extension,
            advertise_definitions: true,
        }
    }

    /// Register name ownership and dispatch without advertising definitions.
    ///
    /// This is the fail-closed form for an implementation disabled by local
    /// composition policy: guessed calls still cross the host's in-process
    /// policy and audit seam instead of falling through to ordinary dispatch.
    pub fn recognition_only(extension: Arc<dyn McpToolExtension>) -> Self {
        Self {
            extension,
            advertise_definitions: false,
        }
    }

    pub(crate) fn extension(&self) -> &Arc<dyn McpToolExtension> {
        &self.extension
    }

    pub(crate) fn advertises_definitions(&self) -> bool {
        self.advertise_definitions
    }
}

/// A pluggable back-end that satisfies MCP `tools/list` and `tools/call`
/// requests.
///
/// `list_mcp_tool_definitions` returns only the tools the host wants exposed,
/// with schema and policy already paired at their canonical definition site.
/// Disabled tools should be filtered out here, not in the adapter.
/// `call_tool` and `call_in_process_tool` must run whatever policy, audit, and
/// sandboxing the host wants applied; the adapter never invokes an in-process
/// implementation without first crossing the latter host seam.
pub trait McpHost: Send + Sync + 'static {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError>;

    /// Validate trusted call identity before the adapter resolves a canonical
    /// tool definition. Hub hosts use this to reject unknown/retired callers
    /// before any registry discovery or domain dispatch.
    fn preflight_tool_call(
        &self,
        _name: &str,
        _session_context: &ToolSessionContext,
    ) -> Result<(), OrbitError> {
        Ok(())
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError>;

    /// Observe a transport-level capability denial. The default preserves the
    /// denial unchanged; audited brokers may record it without executing the
    /// tool or constructing a workspace runtime.
    fn reject_tool_call(
        &self,
        _name: &str,
        _input: &Value,
        _session_context: &ToolSessionContext,
        denial: OrbitError,
    ) -> OrbitError {
        denial
    }

    /// Apply host policy and auditing around a tool implemented inside this
    /// adapter. The secure default rejects the call; hosts must explicitly
    /// admit adapter-owned tools and invoke `dispatch` inside their boundary.
    fn call_in_process_tool(
        &self,
        name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
        _dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
    ) -> Result<Value, OrbitError> {
        Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()))
    }
}

/// Serve the given [`McpHost`] over an MCP stdio transport.
///
/// Runs until the client disconnects or the server encounters a fatal
/// transport error. The function is async and expects to be driven by a tokio
/// runtime (see `tokio::runtime::Runtime::block_on`).
///
/// A stdio process serves exactly one client for its lifetime, so these entry
/// points build one server and keep it. Use [`McpTcpServer`] when more than one
/// client can arrive, since that server must not be shared.
pub async fn serve_stdio(host: Arc<dyn McpHost>) -> Result<(), OrbitError> {
    serve_stdio_with_context(host, ToolSessionContext::trusted_local(None, None, None)).await
}

/// Serve stdio MCP with an explicit in-process extension composition.
pub async fn serve_stdio_with_extensions(
    host: Arc<dyn McpHost>,
    extensions: Vec<McpToolExtensionRegistration>,
) -> Result<(), OrbitError> {
    serve_stdio_with_context_and_extensions(
        host,
        ToolSessionContext::trusted_local(None, None, None),
        extensions,
    )
    .await
}

/// Serve stdio MCP from a complete generic server composition.
pub async fn serve_stdio_with_composition(
    host: Arc<dyn McpHost>,
    composition: McpServerComposition,
) -> Result<(), OrbitError> {
    serve_stdio_with_context_and_composition(
        host,
        ToolSessionContext::trusted_local(None, None, None),
        composition,
    )
    .await
}

/// Serve stdio MCP with adapter-validated trusted context. The external
/// initialize payload can replace only the legacy `workspace` selector.
pub async fn serve_stdio_with_context(
    host: Arc<dyn McpHost>,
    trusted_context: ToolSessionContext,
) -> Result<(), OrbitError> {
    let server = OrbitToolServer::new_with_context(host, trusted_context);
    serve_server(server).await
}

/// Serve stdio MCP with trusted context and an explicit in-process extension
/// composition. This is the construction seam for higher-level brokers such
/// as `orbit-remote`.
pub async fn serve_stdio_with_context_and_extensions(
    host: Arc<dyn McpHost>,
    trusted_context: ToolSessionContext,
    extensions: Vec<McpToolExtensionRegistration>,
) -> Result<(), OrbitError> {
    let server =
        OrbitToolServer::new_with_context_and_extensions(host, trusted_context, extensions);
    serve_server(server).await
}

/// Serve stdio MCP with trusted context and a complete generic server
/// composition.
pub async fn serve_stdio_with_context_and_composition(
    host: Arc<dyn McpHost>,
    trusted_context: ToolSessionContext,
    composition: McpServerComposition,
) -> Result<(), OrbitError> {
    let server =
        OrbitToolServer::new_with_context_and_composition(host, trusted_context, composition);
    serve_server(server).await
}

async fn serve_server(server: OrbitToolServer) -> Result<(), OrbitError> {
    let running = server
        .serve(stdio())
        .await
        .map_err(|err| OrbitError::Execution(format!("mcp serve_stdio start: {err}")))?;
    running
        .waiting()
        .await
        .map_err(|err| OrbitError::Execution(format!("mcp serve_stdio wait: {err}")))?;
    Ok(())
}
