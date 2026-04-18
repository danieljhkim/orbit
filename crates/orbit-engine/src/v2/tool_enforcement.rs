use std::sync::{Arc, Mutex};

use orbit_agent::loop_engine::audit::{AuditSink, LoopAuditEvent};
use orbit_types::v2::{V2AuditEventKind, tool_allowed};

use super::audit_writer::V2AuditWriter;

/// Decision emitted when enforcement fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnforcementDecision {
    Allowed,
    Denied { tool_name: String, reason: String },
}

/// AuditSink wrapper that enforces a tool allowlist at the Orbit layer.
///
/// Usage: Build the inner sink (e.g. JsonlFileSink / InMemorySink), wrap it
/// with an EnforcedAuditSink at construction, and pass the wrapper into
/// AgentLoop::run via the audit parameter. The wrapper intercepts
/// ToolCallRequested events, checks the name against the allowlist, and (on
/// deny) substitutes a PolicyDenial event and signals the caller to
/// terminate.
pub struct EnforcedAuditSink {
    inner: Arc<dyn AuditSink>,
    allowlist: Vec<String>,
    writer: Arc<V2AuditWriter>,
    run_id: String,
    session_id: String,
    tripped: Mutex<Option<EnforcementDecision>>,
}

impl EnforcedAuditSink {
    pub fn new(
        inner: Arc<dyn AuditSink>,
        allowlist: Vec<String>,
        writer: Arc<V2AuditWriter>,
        run_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            allowlist,
            writer,
            run_id: run_id.into(),
            session_id: session_id.into(),
            tripped: Mutex::new(None),
        }
    }

    pub fn tripped(&self) -> Option<EnforcementDecision> {
        self.tripped.lock().expect("tripped mutex").clone()
    }
}

impl AuditSink for EnforcedAuditSink {
    fn emit(&self, event: &LoopAuditEvent) {
        // Two paths can trigger the denial:
        //
        // 1. `AgentLoop` enforces the allowlist internally BEFORE dispatching
        //    a `ToolCallRequested` event and emits its own `PolicyDenial`.
        //    When we see a `PolicyDenial`, we mirror it into a §7
        //    `tool.denied` envelope event so the higher-level trail records
        //    the policy outcome alongside the loop-level event.
        //
        // 2. A caller that bypasses the loop's own check could still emit a
        //    `ToolCallRequested` for a tool we shouldn't allow — we catch
        //    that here too and synthesize both a §7 `tool.denied` envelope
        //    event and a loop-level `PolicyDenial`.
        match event {
            LoopAuditEvent::PolicyDenial {
                tool_name, reason, ..
            } => {
                let _ = self.writer.emit(V2AuditEventKind::ToolDenied {
                    tool_name: tool_name.clone(),
                    reason: reason.clone(),
                });
                *self.tripped.lock().expect("tripped mutex") = Some(EnforcementDecision::Denied {
                    tool_name: tool_name.clone(),
                    reason: reason.clone(),
                });
                self.inner.emit(event);
            }
            LoopAuditEvent::ToolCallRequested { tool_name, .. }
                if !tool_allowed(tool_name, &self.allowlist) =>
            {
                let reason = format!("tool `{tool_name}` not in allowlist");
                let _ = self.writer.emit(V2AuditEventKind::ToolDenied {
                    tool_name: tool_name.clone(),
                    reason: reason.clone(),
                });
                self.inner.emit(event);
                let denial = LoopAuditEvent::PolicyDenial {
                    ts: chrono::Utc::now(),
                    run_id: self.run_id.clone(),
                    session_id: self.session_id.clone(),
                    iteration: 0,
                    tool_name: tool_name.clone(),
                    reason: reason.clone(),
                };
                self.inner.emit(&denial);
                *self.tripped.lock().expect("tripped mutex") = Some(EnforcementDecision::Denied {
                    tool_name: tool_name.clone(),
                    reason,
                });
            }
            _ => self.inner.emit(event),
        }
    }

    fn write_blob(&self, content: &[u8]) -> String {
        self.inner.write_blob(content)
    }
}
