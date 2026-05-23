#![allow(missing_docs)]

mod run {
    #![allow(missing_docs)]

    use std::sync::{Arc, Mutex};

    use orbit_common::types::OrbitError;
    use orbit_common::types::activity_job::OnDenial;
    use orbit_tools::{
        OrbitBuiltinAction, OrbitTaskScope, OrbitToolHost, ReservationOwnerContext, ToolContext,
        ToolRegistry,
    };
    use serde_json::{Value, json};

    use super::super::super::agent_loop::*;
    use super::super::super::audit::NullSink;
    use super::super::super::session::Session;
    use super::super::super::{
        ContentBlock, LoopTransport, MessageRole, StopReason, TransportError, TurnRequest,
        TurnResponse, TurnUsage,
    };

    #[derive(Default)]
    struct RecordingTransport {
        advertised: Mutex<Vec<Vec<String>>>,
        calls: Mutex<usize>,
    }

    impl RecordingTransport {
        fn advertised(&self) -> Vec<Vec<String>> {
            self.advertised.lock().expect("advertised mutex").clone()
        }
    }

    impl LoopTransport for RecordingTransport {
        fn provider(&self) -> &str {
            "test"
        }

        fn model(&self) -> &str {
            "test-model"
        }

        fn send_turn(&self, req: &TurnRequest<'_>) -> Result<TurnResponse, TransportError> {
            self.advertised
                .lock()
                .expect("advertised mutex")
                .push(req.tools.iter().map(|tool| tool.name.clone()).collect());

            let mut calls = self.calls.lock().expect("calls mutex");
            let call_index = *calls;
            *calls += 1;

            let (content, stop_reason) = if call_index == 0 {
                (
                    vec![ContentBlock::ToolUse {
                        id: "call-1".to_string(),
                        name: "orbit.task.show".to_string(),
                        input: json!({ "id": "T-test" }),
                    }],
                    StopReason::ToolUse,
                )
            } else {
                (
                    vec![ContentBlock::Text {
                        text: "done".to_string(),
                    }],
                    StopReason::EndTurn,
                )
            };

            Ok(TurnResponse {
                content,
                stop_reason,
                usage: TurnUsage::default(),
                raw_request_body: Vec::new(),
                raw_response_body: Vec::new(),
                endpoint: String::new(),
                http_status: 200,
            })
        }
    }

    #[derive(Default)]
    struct DenialContinueTransport {
        calls: Mutex<usize>,
    }

    impl LoopTransport for DenialContinueTransport {
        fn provider(&self) -> &str {
            "test"
        }

        fn model(&self) -> &str {
            "test-model"
        }

        fn send_turn(&self, req: &TurnRequest<'_>) -> Result<TurnResponse, TransportError> {
            let mut calls = self.calls.lock().expect("calls mutex");
            let call_index = *calls;
            *calls += 1;

            let (content, stop_reason) = if call_index == 0 {
                (
                    vec![ContentBlock::ToolUse {
                        id: "denied-1".to_string(),
                        name: "fs.delete".to_string(),
                        input: json!({ "path": "/tmp/blocked.txt" }),
                    }],
                    StopReason::ToolUse,
                )
            } else {
                let last_message = req.messages.last().expect("tool result user message");
                assert_eq!(last_message.role, MessageRole::User);
                let [
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    },
                ] = last_message.content.as_slice()
                else {
                    panic!("expected one tool_result block");
                };
                assert_eq!(tool_use_id, "denied-1");
                assert!(*is_error);
                let payload: Value =
                    serde_json::from_str(content).expect("denial tool result is json");
                assert_eq!(payload["error"]["code"], "tool_denied");
                assert_eq!(payload["tool_name"], "fs.delete");
                assert_eq!(payload["tool_use_id"], "denied-1");

                (
                    vec![ContentBlock::Text {
                        text: "done".to_string(),
                    }],
                    StopReason::EndTurn,
                )
            };

            Ok(TurnResponse {
                content,
                stop_reason,
                usage: TurnUsage::default(),
                raw_request_body: Vec::new(),
                raw_response_body: Vec::new(),
                endpoint: String::new(),
                http_status: 200,
            })
        }
    }

    struct FakeOrbitHost;

    impl OrbitToolHost for FakeOrbitHost {
        fn execute(
            &self,
            action: OrbitBuiltinAction,
            input: Value,
            _agent: Option<String>,
            _model: Option<String>,
            _reservation_owner: Option<ReservationOwnerContext>,
        ) -> Result<Value, OrbitError> {
            assert_eq!(action, OrbitBuiltinAction::TaskShow);
            assert_eq!(input["id"], "T-test");
            Ok(json!({ "id": "T-test" }))
        }

        fn task_scope(&self) -> OrbitTaskScope {
            OrbitTaskScope {
                orbit_root: None,
                task_id: Some("T-test".to_string()),
                run_id: None,
            }
        }
    }

    #[test]
    fn wildcard_allowlist_advertises_and_executes_task_show() {
        let mut session = Session::new("test", "test-model", "", None);
        let cfg = AgentLoopConfig::new_for_run("run-test")
            .with_allowlist(vec!["orbit.task.*".to_string()])
            .with_max_iterations(3);
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        let tool_ctx = ToolContext {
            allowed_tools: vec!["orbit.task.*".to_string()],
            orbit_host: Some(Arc::new(FakeOrbitHost)),
            ..Default::default()
        };
        let transport = RecordingTransport::default();
        let sink = NullSink;

        let outcome = AgentLoop::run(
            &mut session,
            &cfg,
            &transport,
            &registry,
            &tool_ctx,
            &sink,
            "show the task",
        )
        .expect("wildcard should allow orbit.task.show");

        assert_eq!(outcome.final_message, "done");
        assert!(
            outcome
                .trace
                .iter()
                .all(|iteration| iteration.policy_denials.is_empty())
        );
        assert!(
            transport
                .advertised()
                .first()
                .expect("first request")
                .iter()
                .any(|name| name == "orbit.task.show")
        );
    }

    #[test]
    fn continue_on_denial_returns_structured_tool_result_error() {
        let mut session = Session::new("test", "test-model", "", None);
        let cfg = AgentLoopConfig::new_for_run("run-test")
            .with_advertised_tools(vec!["fs.delete".to_string()])
            .with_on_denial(OnDenial::Continue)
            .with_max_iterations(3);
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        let tool_ctx = ToolContext::default();
        let transport = DenialContinueTransport::default();
        let sink = NullSink;

        let outcome = AgentLoop::run(
            &mut session,
            &cfg,
            &transport,
            &registry,
            &tool_ctx,
            &sink,
            "try deleting",
        )
        .expect("continue should feed denial back to model");

        assert_eq!(outcome.final_message, "done");
        assert_eq!(outcome.trace.len(), 2);
        assert_eq!(
            outcome.trace[0].policy_denials,
            vec!["fs.delete".to_string()]
        );
    }
}
