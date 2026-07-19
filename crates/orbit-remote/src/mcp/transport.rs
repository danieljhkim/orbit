//! Connector-private MCP context and registration composition.

use std::sync::Arc;

use orbit_common::types::{
    McpTransport, OrbitError, SPOKE_REGISTRATION_METHOD_V1, SpokeRegistrationRequestV1,
    SpokeRegistrationResultV1, ToolSessionContext,
};
use orbit_mcp::{
    McpCallContextResolver, McpCustomRequestError, McpCustomRequestHandler, McpRequestKind,
};
use serde_json::{Map, Value};

use super::hub::HubMcpHost;
use super::hub_client::REMOTE_SESSION_META_KEY;

/// Resolves connector-owned remote identity while retaining server-owned grants.
pub(super) struct RemoteCallContextResolver;

impl McpCallContextResolver for RemoteCallContextResolver {
    fn resolve(
        &self,
        trusted_context: &ToolSessionContext,
        request: &McpRequestKind,
        transport_metadata: &Map<String, Value>,
    ) -> Result<ToolSessionContext, OrbitError> {
        let Some(remote) = remote_session_context_from_metadata(transport_metadata)? else {
            let message = match request {
                McpRequestKind::Custom { method } if method == SPOKE_REGISTRATION_METHOD_V1 => {
                    "private spoke registration requires connector-owned remote session metadata"
                }
                _ => "hub tool calls require connector-owned remote session metadata",
            };
            return Err(OrbitError::InvalidInput(message.to_string()));
        };
        if remote.transport != Some(McpTransport::SshMcp) {
            return Err(OrbitError::InvalidInput(
                "hub remote session metadata must declare ssh-mcp transport".to_string(),
            ));
        }
        if remote.caller_machine_id.is_none()
            || remote.caller_host_id.is_none()
            || remote.origin_session_id.is_none()
            || remote.mcp_call_id.is_none()
        {
            return Err(OrbitError::InvalidInput(
                "hub remote session metadata requires caller identity and call correlation"
                    .to_string(),
            ));
        }
        if remote.process_machine_id.is_some() || remote.process_host_id.is_some() {
            return Err(OrbitError::InvalidInput(
                "hub remote session metadata may not claim process identity".to_string(),
            ));
        }
        let mut remote = remote;
        remote.effective_capabilities = trusted_context.effective_capabilities.clone();
        remote.leased_run = None;
        Ok(remote)
    }
}

/// Typed custom handler for the connector-private spoke bootstrap method.
pub(super) struct SpokeRegistrationHandler {
    host: Arc<HubMcpHost>,
}

impl SpokeRegistrationHandler {
    pub(super) fn new(host: Arc<HubMcpHost>) -> Self {
        Self { host }
    }
}

impl McpCustomRequestHandler for SpokeRegistrationHandler {
    fn recognizes(&self, method: &str) -> bool {
        method == SPOKE_REGISTRATION_METHOD_V1
    }

    fn worker_label(&self) -> &'static str {
        "private spoke registration"
    }

    fn call(
        &self,
        _method: &str,
        params: Option<Value>,
        session_context: ToolSessionContext,
    ) -> Result<Value, McpCustomRequestError> {
        let params = params.ok_or_else(|| {
            McpCustomRequestError::invalid_params("private spoke registration requires parameters")
        })?;
        let registration: SpokeRegistrationRequestV1 =
            serde_json::from_value(params).map_err(|error| {
                McpCustomRequestError::invalid_params(format!(
                    "invalid private spoke registration payload: {error}"
                ))
            })?;
        if let Err(error) = registration.validate() {
            return serialize_registration_result(SpokeRegistrationResultV1::rejected(&error));
        }
        let result = match self
            .host
            .private_register_spoke(registration, session_context)
        {
            Ok(result) => result,
            Err(error) => SpokeRegistrationResultV1::rejected(&error),
        };
        result.validate().map_err(|error| {
            McpCustomRequestError::internal(format!(
                "invalid private spoke registration result: {error}"
            ))
        })?;
        serialize_registration_result(result)
    }
}

fn serialize_registration_result(
    result: SpokeRegistrationResultV1,
) -> Result<Value, McpCustomRequestError> {
    serde_json::to_value(result).map_err(|error| {
        McpCustomRequestError::internal(format!(
            "serialize private spoke registration result: {error}"
        ))
    })
}

fn remote_session_context_from_metadata(
    metadata: &Map<String, Value>,
) -> Result<Option<ToolSessionContext>, OrbitError> {
    let Some(value) = metadata
        .get("orbit")
        .and_then(|orbit| orbit.get(REMOTE_SESSION_META_KEY))
    else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| {
            OrbitError::InvalidInput(format!("invalid hub remote session metadata: {error}"))
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use orbit_common::types::{McpCapability, McpLeasedRun};
    use orbit_mcp::McpCallContextResolver;
    use serde_json::json;

    use super::*;

    #[test]
    fn hub_calls_fail_closed_without_connector_metadata() {
        let trusted = trusted_agent_context();
        let error = RemoteCallContextResolver
            .resolve(
                &trusted,
                &McpRequestKind::Tool {
                    name: "orbit.task.show".to_string(),
                },
                &Map::new(),
            )
            .expect_err("missing connector metadata must fail closed");
        assert!(
            error
                .to_string()
                .contains("hub tool calls require connector-owned remote session metadata")
        );
    }

    #[test]
    fn connector_identity_is_retained_but_grants_and_lease_are_server_owned() {
        let trusted = trusted_agent_context();
        let mut remote = remote_context();
        remote.effective_capabilities =
            BTreeSet::from([McpCapability::Operator, McpCapability::Runner]);
        remote.leased_run = Some(McpLeasedRun {
            run_id: "forged-run".to_string(),
            lease_id: "forged-lease".to_string(),
        });
        let resolved = RemoteCallContextResolver
            .resolve(
                &trusted,
                &McpRequestKind::Tool {
                    name: "orbit.task.show".to_string(),
                },
                &metadata(remote),
            )
            .expect("valid connector metadata");

        assert_eq!(resolved.caller_machine_id.as_deref(), Some("hm_spoke"));
        assert_eq!(resolved.caller_host_id.as_deref(), Some("spoke"));
        assert_eq!(
            resolved.effective_capabilities,
            BTreeSet::from([McpCapability::Agent])
        );
        assert!(resolved.leased_run.is_none());
    }

    #[test]
    fn connector_metadata_cannot_claim_hub_process_identity() {
        let trusted = trusted_agent_context();
        let mut remote = remote_context();
        remote.process_machine_id = Some("hm_hub".to_string());
        let error = RemoteCallContextResolver
            .resolve(
                &trusted,
                &McpRequestKind::Custom {
                    method: SPOKE_REGISTRATION_METHOD_V1.to_string(),
                },
                &metadata(remote),
            )
            .expect_err("remote process identity is never caller-owned");
        assert!(error.to_string().contains("may not claim process identity"));
    }

    fn trusted_agent_context() -> ToolSessionContext {
        let mut context = ToolSessionContext::trusted_local(
            None,
            Some("hm_hub".to_string()),
            Some("hub".to_string()),
        );
        context.effective_capabilities = BTreeSet::from([McpCapability::Agent]);
        context
    }

    fn remote_context() -> ToolSessionContext {
        ToolSessionContext {
            transport: Some(McpTransport::SshMcp),
            caller_machine_id: Some("hm_spoke".to_string()),
            caller_host_id: Some("spoke".to_string()),
            origin_session_id: Some("session-1".to_string()),
            mcp_call_id: Some("mcall-1".to_string()),
            ..ToolSessionContext::default()
        }
    }

    fn metadata(context: ToolSessionContext) -> Map<String, Value> {
        json!({
            "orbit": {
                (REMOTE_SESSION_META_KEY): context,
            }
        })
        .as_object()
        .expect("metadata object")
        .clone()
    }
}
