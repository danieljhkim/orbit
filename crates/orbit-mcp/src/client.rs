//! Generic asynchronous MCP client for an injected byte-stream transport.

use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientInfo, ClientRequest, CustomRequest, Meta,
    ServerResult,
};
use rmcp::service::{PeerRequestOptions, RoleClient, RunningService, ServiceError};
use serde_json::{Map, Value};
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpClientInitialization {
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpToolResponse {
    pub content: Vec<Value>,
    pub structured_content: Option<Value>,
    pub is_error: Option<bool>,
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpClientRequestError {
    PreHandoff {
        message: String,
    },
    PostHandoff {
        message: String,
    },
    Protocol {
        code: i32,
        message: String,
        data: Option<Value>,
    },
    UnexpectedResponse {
        message: String,
    },
}

impl McpClientRequestError {
    fn pre_handoff(message: impl Into<String>) -> Self {
        Self::PreHandoff {
            message: message.into(),
        }
    }

    fn message(&self) -> String {
        match self {
            Self::PreHandoff { message }
            | Self::PostHandoff { message }
            | Self::UnexpectedResponse { message } => message.clone(),
            Self::Protocol {
                code,
                message,
                data: _,
            } => format!("MCP protocol error {code}: {message}"),
        }
    }
}

impl std::fmt::Display for McpClientRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for McpClientRequestError {}

pub struct RawOrbitMcpClient {
    service: RunningService<RoleClient, ClientInfo>,
    initialization: McpClientInitialization,
}

impl RawOrbitMcpClient {
    pub async fn connect<R, W>(
        read: R,
        write: W,
        initialize_timeout: Duration,
    ) -> Result<Self, McpClientRequestError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let service = tokio::time::timeout(
            initialize_timeout,
            ClientInfo::default().serve((read, write)),
        )
        .await
        .map_err(|_| {
            McpClientRequestError::pre_handoff(format!(
                "MCP initialize exceeded {} ms",
                initialize_timeout.as_millis()
            ))
        })?
        .map_err(|error| {
            McpClientRequestError::pre_handoff(format!("MCP initialize failed: {error}"))
        })?;
        let initialization = service.peer_info().map_or(
            McpClientInitialization {
                server_name: None,
                server_version: None,
                instructions: None,
            },
            |info| McpClientInitialization {
                server_name: Some(info.server_info.name.clone()),
                server_version: Some(info.server_info.version.clone()),
                instructions: info.instructions.clone(),
            },
        );
        Ok(Self {
            service,
            initialization,
        })
    }

    pub fn initialization(&self) -> &McpClientInitialization {
        &self.initialization
    }

    pub fn is_closed(&self) -> bool {
        self.service.is_closed()
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Map<String, Value>,
        transport_metadata: Map<String, Value>,
        request_timeout: Duration,
    ) -> Result<McpToolResponse, McpClientRequestError> {
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(arguments);
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let response = self
            .send_request(request, transport_metadata, request_timeout)
            .await?;
        let ServerResult::CallToolResult(result) = response else {
            return Err(McpClientRequestError::UnexpectedResponse {
                message: "server returned a non-tool result".to_string(),
            });
        };
        let content = result
            .content
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| McpClientRequestError::UnexpectedResponse {
                message: format!("serialize MCP tool content: {error}"),
            })?;
        Ok(McpToolResponse {
            content,
            structured_content: result.structured_content,
            is_error: result.is_error,
            metadata: result.meta.map(|meta| meta.0),
        })
    }

    pub async fn custom_request(
        &self,
        method: &str,
        params: Option<Value>,
        transport_metadata: Map<String, Value>,
        request_timeout: Duration,
    ) -> Result<Value, McpClientRequestError> {
        let request = ClientRequest::CustomRequest(CustomRequest::new(method, params));
        let response = self
            .send_request(request, transport_metadata, request_timeout)
            .await?;
        let ServerResult::CustomResult(result) = response else {
            return Err(McpClientRequestError::UnexpectedResponse {
                message: "server returned a non-custom result".to_string(),
            });
        };
        Ok(result.0)
    }

    async fn send_request(
        &self,
        request: ClientRequest,
        transport_metadata: Map<String, Value>,
        request_timeout: Duration,
    ) -> Result<ServerResult, McpClientRequestError> {
        let mut options = PeerRequestOptions::default();
        options.timeout = Some(request_timeout);
        if !transport_metadata.is_empty() {
            options.meta = Some(Meta(transport_metadata));
        }
        let handle = self
            .service
            .peer()
            .send_request_with_option(request, options)
            .await
            .map_err(|error| McpClientRequestError::PreHandoff {
                message: error.to_string(),
            })?;
        match handle.await_response().await {
            Ok(response) => Ok(response),
            Err(ServiceError::McpError(error)) => Err(McpClientRequestError::Protocol {
                code: error.code.0,
                message: error.message.into_owned(),
                data: error.data,
            }),
            Err(error) => Err(McpClientRequestError::PostHandoff {
                message: error.to_string(),
            }),
        }
    }

    pub async fn close(&mut self, timeout: Duration) -> Result<(), McpClientRequestError> {
        self.service
            .close_with_timeout(timeout)
            .await
            .map_err(|error| McpClientRequestError::PostHandoff {
                message: error.to_string(),
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use orbit_common::types::{
        McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, OrbitError, ToolSchema,
        ToolSessionContext,
    };
    use rmcp::ServiceExt;
    use serde_json::{Map, Value, json};
    use tokio::io::duplex;

    use super::*;
    use crate::{
        McpCallContextResolver, McpCustomRequestError, McpCustomRequestHandler, McpHost,
        McpRequestKind, McpResultDecoration, McpResultDecorationFuture, McpResultDecorator,
        McpServerComposition, McpServerMetadata, OrbitToolServer,
    };

    struct GenericHost {
        definitions: Vec<McpToolDefinition>,
    }

    impl McpHost for GenericHost {
        fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
            Ok(self.definitions.clone())
        }

        fn call_tool(
            &self,
            name: &str,
            input: Value,
            context: ToolSessionContext,
        ) -> Result<Value, OrbitError> {
            Ok(json!({
                "name": name,
                "input": input,
                "workspace": context.workspace,
                "mcp_call_id": context.mcp_call_id,
            }))
        }
    }

    struct MetadataContextResolver;

    impl McpCallContextResolver for MetadataContextResolver {
        fn resolve(
            &self,
            trusted_context: &ToolSessionContext,
            _request: &McpRequestKind,
            transport_metadata: &Map<String, Value>,
        ) -> Result<ToolSessionContext, OrbitError> {
            let mut context = trusted_context.clone();
            context.workspace = transport_metadata
                .get("workspace")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            context.mcp_call_id = Some("mcall-generic".to_string());
            Ok(context)
        }
    }

    struct ContextResultDecorator;

    impl McpResultDecorator for ContextResultDecorator {
        fn decorate(&self, call: McpResultDecoration) -> McpResultDecorationFuture<'_> {
            Box::pin(async move {
                Ok(json!({
                    "decorated": call.output,
                    "call_workspace": call.call_context.workspace,
                    "server_workspace": call.server_context.workspace,
                }))
            })
        }
    }

    struct EchoCustomRequest;

    impl McpCustomRequestHandler for EchoCustomRequest {
        fn recognizes(&self, method: &str) -> bool {
            method == "demo/custom"
        }

        fn call(
            &self,
            method: &str,
            params: Option<Value>,
            session_context: ToolSessionContext,
        ) -> Result<Value, McpCustomRequestError> {
            Ok(json!({
                "method": method,
                "params": params,
                "workspace": session_context.workspace,
                "mcp_call_id": session_context.mcp_call_id,
            }))
        }
    }

    #[tokio::test]
    async fn raw_client_round_trips_generic_composition_without_owner_contract() {
        let definition = McpToolDefinition::new(
            ToolSchema {
                name: "demo.generic".to_string(),
                description: "Generic raw-client fixture".to_string(),
                parameters: Vec::new(),
                builtin: false,
            },
            McpToolPolicy::agent_and_operator(McpToolPlacement::Owner),
        )
        .expect("definition");
        let host: Arc<dyn McpHost> = Arc::new(GenericHost {
            definitions: vec![definition],
        });
        let composition = McpServerComposition::new()
            .with_call_context_resolver(Arc::new(MetadataContextResolver))
            .with_result_decorator(Arc::new(ContextResultDecorator))
            .with_custom_request_handler(Arc::new(EchoCustomRequest))
            .with_metadata(
                McpServerMetadata::default().with_instructions("opaque-test-instructions"),
            );
        let mut trusted = ToolSessionContext::trusted_local(None, None, None);
        trusted.effective_capabilities = BTreeSet::from([McpCapability::Agent]);
        trusted.workspace = Some("server-workspace".to_string());
        let server = OrbitToolServer::new_with_context_and_composition(host, trusted, composition);
        let (server_io, client_io) = duplex(64 * 1024);
        tokio::spawn(async move {
            if let Ok(running) = server.serve(server_io).await {
                let _ = running.waiting().await;
            }
        });
        let (read, write) = tokio::io::split(client_io);
        let mut client = RawOrbitMcpClient::connect(read, write, Duration::from_secs(1))
            .await
            .expect("raw connect");

        assert_eq!(
            client.initialization().instructions.as_deref(),
            Some("opaque-test-instructions")
        );
        let metadata = Map::from_iter([(
            "workspace".to_string(),
            Value::String("metadata-workspace".to_string()),
        )]);
        let tool = client
            .call_tool(
                "demo_generic",
                Map::from_iter([("value".to_string(), json!(7))]),
                metadata.clone(),
                Duration::from_secs(1),
            )
            .await
            .expect("raw tool call");
        assert_eq!(
            tool.structured_content
                .as_ref()
                .expect("structured content")["call_workspace"],
            "metadata-workspace"
        );
        assert_eq!(
            tool.structured_content
                .as_ref()
                .expect("structured content")["server_workspace"],
            Value::Null
        );
        assert_eq!(
            tool.structured_content
                .as_ref()
                .expect("structured content")["decorated"]["mcp_call_id"],
            "mcall-generic"
        );

        let custom = client
            .custom_request(
                "demo/custom",
                Some(json!({ "value": 9 })),
                metadata,
                Duration::from_secs(1),
            )
            .await
            .expect("raw custom request");
        assert_eq!(custom["workspace"], "metadata-workspace");
        assert_eq!(custom["params"]["value"], 9);
        assert_eq!(custom["mcp_call_id"], "mcall-generic");

        client
            .close(Duration::from_secs(1))
            .await
            .expect("raw close");
        assert!(client.is_closed());
    }
}
