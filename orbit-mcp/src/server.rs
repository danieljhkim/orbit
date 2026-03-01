use std::io::{BufReader, stdin, stdout};

use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::dispatch::dispatch_tool;
use crate::protocol::{
    ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND, ERR_PARSE, JSONRPC_VERSION,
    JsonRpcRequest, JsonRpcResponse, ToolsCallParams,
};
use crate::registry::mcp_tools;
use crate::transport::{read_framed_json, write_framed_json};

struct RequestOutcome {
    response: Option<JsonRpcResponse>,
    should_exit: bool,
}

pub fn serve_stdio(runtime: &OrbitRuntime) -> Result<(), OrbitError> {
    let mut reader = BufReader::new(stdin().lock());
    let mut writer = stdout().lock();
    let mut shutdown_requested = false;

    loop {
        let Some(frame) = read_framed_json(&mut reader)? else {
            break;
        };

        let request = match serde_json::from_slice::<JsonRpcRequest>(&frame) {
            Ok(request) => request,
            Err(err) => {
                let response = JsonRpcResponse::error(
                    Value::Null,
                    ERR_PARSE,
                    format!("failed to parse JSON-RPC request: {err}"),
                );
                write_framed_json(&mut writer, &response)?;
                continue;
            }
        };

        let outcome = handle_request(runtime, request, &mut shutdown_requested);

        if let Some(response) = outcome.response {
            write_framed_json(&mut writer, &response)?;
        }

        if outcome.should_exit {
            break;
        }
    }

    Ok(())
}

fn handle_request(
    runtime: &OrbitRuntime,
    request: JsonRpcRequest,
    shutdown_requested: &mut bool,
) -> RequestOutcome {
    let id = request.id.clone();

    if request
        .jsonrpc
        .as_deref()
        .is_some_and(|value| value != JSONRPC_VERSION)
    {
        return RequestOutcome {
            response: id.map(|value| {
                JsonRpcResponse::error(value, ERR_INVALID_REQUEST, "jsonrpc must be exactly '2.0'")
            }),
            should_exit: false,
        };
    }

    match request.method.as_str() {
        "initialize" => {
            let result = json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "orbit-mcp",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            });
            RequestOutcome {
                response: id.map(|value| JsonRpcResponse::success(value, result)),
                should_exit: false,
            }
        }
        "notifications/initialized" => RequestOutcome {
            response: None,
            should_exit: false,
        },
        "tools/list" => {
            let tools = mcp_tools();
            RequestOutcome {
                response: id.map(|value| {
                    JsonRpcResponse::success(
                        value,
                        json!({
                            "tools": tools,
                        }),
                    )
                }),
                should_exit: false,
            }
        }
        "tools/call" => {
            let Some(id_value) = id else {
                return RequestOutcome {
                    response: Some(JsonRpcResponse::error(
                        Value::Null,
                        ERR_INVALID_REQUEST,
                        "tools/call requires a request id",
                    )),
                    should_exit: false,
                };
            };

            let params_value = request.params.unwrap_or_else(|| json!({}));
            let parsed = serde_json::from_value::<ToolsCallParams>(params_value);
            let params = match parsed {
                Ok(value) => value,
                Err(err) => {
                    return RequestOutcome {
                        response: Some(JsonRpcResponse::error(
                            id_value,
                            ERR_INVALID_PARAMS,
                            format!("invalid tools/call params: {err}"),
                        )),
                        should_exit: false,
                    };
                }
            };

            let envelope = dispatch_tool(runtime, &params.name, params.arguments);
            let text = serde_json::to_string(&envelope).unwrap_or_else(|_| {
                "{\"ok\":false,\"error\":{\"code\":\"SERIALIZATION_FAILED\",\"message\":\"failed to serialize envelope\"}}".to_string()
            });
            let result = json!({
                "content": [
                    {
                        "type": "text",
                        "text": text,
                    }
                ],
                "structuredContent": envelope,
                "isError": !envelope.ok,
            });

            RequestOutcome {
                response: Some(JsonRpcResponse::success(id_value, result)),
                should_exit: false,
            }
        }
        "shutdown" => {
            *shutdown_requested = true;
            RequestOutcome {
                response: id.map(|value| JsonRpcResponse::success(value, json!({}))),
                should_exit: false,
            }
        }
        "exit" => RequestOutcome {
            response: None,
            should_exit: *shutdown_requested,
        },
        other => RequestOutcome {
            response: id.map(|value| {
                JsonRpcResponse::error(
                    value,
                    ERR_METHOD_NOT_FOUND,
                    format!("unknown method '{other}'"),
                )
            }),
            should_exit: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::protocol::JsonRpcRequest;

    use super::handle_request;

    fn runtime_with_identity() -> orbit_core::OrbitRuntime {
        let dir = tempdir().expect("tempdir");
        let data_root = dir.path().join(".orbit");
        fs::create_dir_all(&data_root).expect("data root");

        let identity_root = dir.path().join("identities");
        fs::create_dir_all(&identity_root).expect("identity root");
        fs::write(
            identity_root.join("linus.yaml"),
            r#"identity:
  name: linus
  display_name: Linus
  role: leader
"#,
        )
        .expect("identity file");

        fs::write(
            data_root.join("config.toml"),
            format!(
                "[identity]\nroot = \"{}\"\n",
                identity_root.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .expect("config");

        orbit_core::OrbitRuntime::from_data_root(&data_root).expect("runtime")
    }

    #[test]
    fn initialize_returns_server_info() {
        let runtime = runtime_with_identity();
        let mut shutdown = false;
        let outcome = handle_request(
            &runtime,
            JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!(1)),
                method: "initialize".to_string(),
                params: None,
            },
            &mut shutdown,
        );

        let response = outcome.response.expect("response");
        let result = response.result.expect("result");
        assert_eq!(result["serverInfo"]["name"], "orbit-mcp");
    }

    #[test]
    fn tools_list_returns_entries() {
        let runtime = runtime_with_identity();
        let mut shutdown = false;
        let outcome = handle_request(
            &runtime,
            JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!(2)),
                method: "tools/list".to_string(),
                params: None,
            },
            &mut shutdown,
        );

        let response = outcome.response.expect("response");
        let result = response.result.expect("result");
        let tools = result["tools"].as_array().expect("tools array");
        assert!(tools.iter().any(|tool| tool["name"] == "orbit.task.add"));
    }
}
