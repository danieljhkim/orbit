//! HTTP agent loop, sibling to the CLI-based [`AgentRuntime`](crate::AgentRuntime).
//!
//! This module provides the provider-agnostic primitives that drive
//! HTTP/SDK-first agent execution:
//!
//! - [`AgentLoop`] — runs the send/parse/dispatch/repeat cycle with explicit
//!   guardrails and tool-allowlist enforcement.
//! - [`Session`] — a resumable, in-process conversation handle. Messages are
//!   replayed in full on each turn; prompt caching is how this stays cheap.
//! - [`LoopTransport`] — sibling trait to `AgentRuntime`, implemented per
//!   provider wire format. The shapes diverge enough (one-shot command
//!   descriptor vs. iterative conversation driver) that forcing unification
//!   would over-generalize, so they coexist.
//! - [`AuditSink`] / [`LoopAuditEvent`] — complete structured audit coverage
//!   for every HTTP request/response, tool call, iteration boundary, and
//!   session lifecycle event. Verbatim bodies live behind a
//!   content-addressed [`BlobStore`] that redacts at write time.
//!
//! Sessions are process-local by design; cross-process persistence is a
//! separate layer on top if ever needed.

pub mod agent_loop;
pub mod audit;
pub mod replay_transport;
pub mod session;
pub mod tool_dispatch;
pub mod transport;

pub use replay_transport::{ReplayTransport, ReplayTurn};

pub use agent_loop::{
    AgentLoop, AgentLoopConfig, AgentLoopError, IterationTrace, LoopOutcome, TerminateReason,
};
pub use audit::{
    AuditSink, BlobStore, InMemorySink, JsonlFileSink, LoopAuditEvent, NullSink,
    RedactionMiddleware, UsageSnapshot,
};
pub use session::Session;
pub use transport::{
    CacheHint, ContentBlock, LoopTransport, Message, MessageRole, StopReason, ToolSpec,
    TransportError, TurnRequest, TurnResponse, TurnUsage,
};
