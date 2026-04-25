//! Synthetic fixture: `construct-vs-match-benchevent-distinct`
//!
//! Tests whether graph distinguishes CONSTRUCTION sites of an enum variant
//! from PATTERN MATCH sites of the same variant.
//!
//! Target variant: `BenchAuditEvent::CallReturned` (variant name distinct
//! from production `LoopAuditEvent::ToolCallResult`; see METHOD.md
//! §"Synthetic-vs-production name isolation").
//!
//! External construction patterns probed (all four MUST be in the answer):
//!   1. Direct struct-literal — `BenchAuditEvent::CallReturned { .. }`
//!   2. Builder helper call-site — `BenchAuditEvent::call_returned(...)`
//!   3. Nested constructor — wrapped inside another type's variant
//!   4. Imported variant — `use ...::CallReturned; CallReturned { .. }`
//!
//! Distractors (must NOT be in the answer):
//!   - The builder helper method definition itself
//!   - Pure `match` arm
//!   - `if let` destructuring
//!   - Type-only mention in a function signature

pub enum BenchAuditEvent {
    CallReturned { tool: String, ok: bool },
    Other,
}

impl BenchAuditEvent {
    /// Builder helper that constructs a `CallReturned` variant. Pattern #2.
    pub fn call_returned(tool: &str, ok: bool) -> Self {
        Self::CallReturned {
            tool: tool.into(),
            ok,
        }
    }
}

/// Wrapper enum used to demonstrate the "nested constructor" pattern.
pub enum WrappedEvent {
    Inner(BenchAuditEvent),
    Empty,
}

// =========================================================================
// CONSTRUCTOR sites — answer set (4 functions).
// =========================================================================

// Pattern #1: direct struct-literal construction.
pub fn construct_direct() -> BenchAuditEvent {
    BenchAuditEvent::CallReturned {
        tool: "alpha".to_string(),
        ok: true,
    }
}

// Pattern #2: external builder-helper call-site. The helper implementation
// itself is not in the answer set.
pub fn construct_via_builder() -> BenchAuditEvent {
    BenchAuditEvent::call_returned("beta", false)
}

// Pattern #3: nested constructor inside another variant.
pub fn construct_nested() -> WrappedEvent {
    WrappedEvent::Inner(BenchAuditEvent::CallReturned {
        tool: "gamma".into(),
        ok: true,
    })
}

// Pattern #4: imported variant.
pub fn construct_imported() -> BenchAuditEvent {
    use BenchAuditEvent::CallReturned;
    CallReturned {
        tool: "delta".into(),
        ok: false,
    }
}

// =========================================================================
// DISTRACTORS — must NOT be in the answer.
// =========================================================================

// Pure pattern match. Reads the variant; does not construct it.
pub fn match_call_returned(e: &BenchAuditEvent) -> bool {
    match e {
        BenchAuditEvent::CallReturned { ok, .. } => *ok,
        BenchAuditEvent::Other => false,
    }
}

// `if let` destructuring. Reads the variant; does not construct it.
pub fn if_let_call_returned(e: &BenchAuditEvent) -> Option<String> {
    if let BenchAuditEvent::CallReturned { tool, .. } = e {
        Some(tool.clone())
    } else {
        None
    }
}

// Type-only mention in a signature. Does not construct anything.
pub fn type_only_distractor(_e: BenchAuditEvent) -> u32 {
    0
}
