//! Synthetic fixture: `implementors-benchsink-with-blanket`
//!
//! Tests graph's `implementors` query on a trait that has:
//!   - direct impls
//!   - a blanket impl
//!   - a feature-gated impl (gated by a `#[cfg(...)]` attribute)
//!
//! Target trait: `BenchAuditSink` with method `record` (NOT `emit` — that
//! collides with production `AuditSink::emit` and would pollute results
//! across fixtures; see METHOD.md §"Synthetic-vs-production name isolation").
//!
//! Expected answer (4 implementors): the type names below.
//! Distractor: `BenchUnrelatedType` does NOT implement `BenchAuditSink`.
//!
//! This file is parsed by the orbit knowledge-graph indexer via the narrow
//! `.orbitignore` negation. It is not part of any cargo crate.

pub trait BenchAuditSink {
    fn record(&self, payload: &str);
}

// =========================================================================
// Direct impls — both should appear in the answer.
// =========================================================================

pub struct BenchNullSink;

impl BenchAuditSink for BenchNullSink {
    fn record(&self, _payload: &str) {
        // No-op. Mirrors the production NullSink shape.
    }
}

pub struct BenchInMemorySink {
    pub events: Vec<String>,
}

impl BenchAuditSink for BenchInMemorySink {
    fn record(&self, payload: &str) {
        // Illustrative; not actually mutating because we keep the file
        // syntactically simple and don't depend on `&mut self` semantics.
        let _ = payload;
    }
}

// =========================================================================
// Blanket impl — should appear in the answer.
// `impl<T: BenchAuditSink> BenchAuditSink for BenchWrapper<T>` makes any
// `BenchWrapper<T>` an implementor wherever T is.
// =========================================================================

pub struct BenchWrapper<T> {
    pub inner: T,
}

impl<T: BenchAuditSink> BenchAuditSink for BenchWrapper<T> {
    fn record(&self, payload: &str) {
        self.inner.record(payload);
    }
}

// =========================================================================
// Feature-gated impl — should appear in the answer.
// `#[cfg(feature = "bench_extra_sinks")]` is a compile-time gate; the
// orbit-knowledge parser sees source AST regardless of cfg evaluation, so
// this impl is expected to be indexed.
// =========================================================================

#[cfg(feature = "bench_extra_sinks")]
pub struct BenchFeatureSink;

#[cfg(feature = "bench_extra_sinks")]
impl BenchAuditSink for BenchFeatureSink {
    fn record(&self, payload: &str) {
        let _ = payload;
    }
}

// =========================================================================
// Distractor: a struct that does NOT implement BenchAuditSink. It must NOT
// appear in the answer even though it lives in the same module.
// =========================================================================

pub struct BenchUnrelatedType {
    pub data: String,
}
