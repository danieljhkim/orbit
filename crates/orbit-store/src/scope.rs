//! Unified scope resolution (§9, v2 activity-job design).
//!
//! Collapses the per-asset-type resolution logic historically scattered
//! across layered store wrappers into a single canonical abstraction: a
//! [`ScopeStrategy`] enum describing how workspace and global layers compose,
//! a [`ScopedStore`] trait surfacing the two getters, and a single [`resolve`]
//! function implementing the merge logic.
//!
//! ## Phase 2 scope
//!
//! Phase 2 introduces the abstraction and establishes the strategy mapping.
//! The existing layered store wrappers (`LayeredActivityStore`,
//! `LayeredJobStore`, etc.) retain their inline implementations and can
//! migrate to [`resolve`] incrementally. Until every layer calls this module,
//! `rg 'fn.*resolve' crates/orbit-store/` will still show legacy sites;
//! Phase 4 cutover completes the fan-out.
//!
//! ## Design decisions
//!
//! Each asset type maps to exactly one strategy — no per-key overrides,
//! no runtime policy. The mapping mirrors the table in `CLAUDE.md` under
//! `## Scoping Rules`.

/// Strategy describing how workspace and global layers compose for a given
/// asset type. See `CLAUDE.md` for the canonical mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeStrategy {
    /// Workspace layer only; global is ignored. Used for tasks and job runs.
    WorkspaceOnly,
    /// Global provides defaults; workspace entries with matching keys
    /// override. Used for activities, jobs, and skills.
    MergeByKey,
    /// Single authoritative layer (global); workspace is ignored. Used for
    /// the audit trail.
    GlobalOnly,
}

/// Contract surfaced by a scoped store so [`resolve`] can compose its two
/// layers without knowing anything about the concrete backend. Lookups are
/// fallible (the underlying backends can fail for I/O reasons), so getters
/// return `Result<Option<T>, E>` with an error type chosen by the store.
pub trait ScopedStore<T> {
    /// Error type propagated from the underlying backend.
    type Err;
    /// Strategy governing this store's composition.
    fn strategy(&self) -> ScopeStrategy;
    /// Look up the workspace layer. `Ok(None)` when not present.
    fn get_workspace(&self, key: &str) -> Result<Option<T>, Self::Err>;
    /// Look up the global layer. `Ok(None)` when not present.
    fn get_global(&self, key: &str) -> Result<Option<T>, Self::Err>;
}

/// Resolve a key to its composed value by applying the store's strategy
/// uniformly. This is the single canonical resolution site — callers route
/// every lookup through this function rather than reimplementing the merge
/// logic per asset type.
pub fn resolve<T, S: ScopedStore<T>>(store: &S, key: &str) -> Result<Option<T>, S::Err> {
    match store.strategy() {
        ScopeStrategy::WorkspaceOnly => store.get_workspace(key),
        ScopeStrategy::GlobalOnly => store.get_global(key),
        ScopeStrategy::MergeByKey => {
            // Workspace entries with matching keys override global defaults.
            // Callers that need a full merged listing (e.g. `list_all`) walk
            // both layers separately — resolve() is for single-key lookups.
            if let Some(value) = store.get_workspace(key)? {
                return Ok(Some(value));
            }
            store.get_global(key)
        }
    }
}
