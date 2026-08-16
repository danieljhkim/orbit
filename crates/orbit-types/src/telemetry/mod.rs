//! Domain contracts for this Orbit types module.

mod audit_actor;
mod audit_event;
mod error;
mod invocation;
mod metrics;
mod pricing;
pub use error::TelemetryError;

#[cfg(test)]
mod tests;

pub use audit_actor::{
    ACTOR_ALIAS_MAP_VERSION, ActorKind, CanonicalActor, canonical_actor_for_role_label,
};
pub use audit_event::{AuditEvent, AuditEventStatus, AuditStats};
pub use invocation::{InvocationTrace, TokenUsage, ToolCallTrace};
pub use metrics::MetricsEntry;
pub use pricing::{InputTokenBasis, PriceRow, cost_from_rows, normalize_token_usage_from_rows};
