//! Synthetic fixture: `generic-dispatch-concrete-impl`
//!
//! Tests whether graph can identify which concrete impl of a trait method
//! actually runs at a generic call-site. This is a precision-gap probe:
//! resolving the answer requires type-level reasoning about
//! monomorphization, which graph's name-based BFS cannot do (per
//! `2_design.md` §"Reference resolution").
//!
//! Target call-site: `site_alpha`, which calls `dispatch_with(BenchProcessA)`.
//! The fixture asks: which impl of `BenchProcess::process` runs at that site?
//!
//! Expected answer: `BenchProcessA::process` (because `site_alpha`
//! instantiates the generic with `BenchProcessA`).
//!
//! Distractors:
//!   - `BenchProcessB::process` — runs at `site_beta`, NOT `site_alpha`.
//!   - `BenchProcessC` — has no `BenchProcess` impl at all.
//!
//! Predicted graph behaviour: graph's `callers`/`refs` will surface
//! `BenchProcessA::process`, `BenchProcessB::process`, and `dispatch_with`
//! as related symbols, but cannot disambiguate which impl runs at which
//! call-site. The answer requires reading source.

pub trait BenchProcess {
    fn process(&self) -> u32;
}

// =========================================================================
// Two concrete impls of BenchProcess. Only one runs at the target call-site.
// =========================================================================

pub struct BenchProcessA;

impl BenchProcess for BenchProcessA {
    fn process(&self) -> u32 {
        // Distinct return value lets the test's ground truth be observed
        // by reading the source.
        1
    }
}

pub struct BenchProcessB;

impl BenchProcess for BenchProcessB {
    fn process(&self) -> u32 {
        2
    }
}

// =========================================================================
// Distractor: a struct that doesn't implement BenchProcess.
// =========================================================================

pub struct BenchProcessC;

// No `impl BenchProcess for BenchProcessC` — intentional. Distractor for the
// "list all impls of BenchProcess" question that an agent might over-include.

// =========================================================================
// Generic dispatcher and its call-sites.
// =========================================================================

pub fn dispatch_with<T: BenchProcess>(t: T) -> u32 {
    t.process()
}

/// THE TARGET CALL-SITE. The fixture asks which impl of `process` runs here.
/// Answer: `BenchProcessA::process` (because the generic is instantiated
/// with `BenchProcessA`).
pub fn site_alpha() -> u32 {
    dispatch_with(BenchProcessA)
}

/// Different call-site, different concrete type. Distractor for the target
/// question — `site_alpha` does NOT dispatch through this site.
pub fn site_beta() -> u32 {
    dispatch_with(BenchProcessB)
}
