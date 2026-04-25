//! Synthetic fixture: `macro-expanded-callers`
//!
//! Precision-gap fixture for source-visible calls to a derive-provided method.
//! Graph sees the explicit call-sites in this file, but not the synthetic
//! `Default` impl that `#[derive(Default)]` produces.
//!
//! Target call-sites: every function in this file that calls
//! `BenchDerivedStruct::default()`.
//!
//! Expected answer (3 functions): `site_one`, `site_two`, `site_three`.
//!
//! Distractor: `site_distractor` constructs `BenchDerivedStruct` via
//! explicit field-init syntax and never calls `default()`.
//!
//! NOTE: `Default::default()` is a stdlib method with thousands of
//! production callers. The fixture's prompt MUST scope to this file
//! explicitly (e.g. "in `benchmarks/graph/v4/_fixture_code/macro_expanded_callers.rs`")
//! to avoid pollution from production. Graph's `callers` BFS is by simple
//! name (`default`) and will return huge result sets without the scope.
//!
//! Predicted graph behaviour:
//!   - The 3 call-sites are in source AST → graph SHOULD see them.
//!   - The macro-generated `impl Default for BenchDerivedStruct { fn default(...) }`
//!     is NOT in source AST → graph CANNOT see it.
//!   - So `callers of BenchDerivedStruct::default` should return the 3 sites,
//!     but `implementors of Default` for `BenchDerivedStruct` should NOT
//!     include the derive-generated impl.

#[derive(Default, Debug)]
pub struct BenchDerivedStruct {
    pub name: String,
    pub count: u32,
}

// =========================================================================
// CALL-SITES of `BenchDerivedStruct::default()` — answer set (3 functions).
// =========================================================================

pub fn site_one() -> BenchDerivedStruct {
    BenchDerivedStruct::default()
}

pub fn site_two() -> BenchDerivedStruct {
    BenchDerivedStruct::default()
}

pub fn site_three() -> Option<BenchDerivedStruct> {
    Some(BenchDerivedStruct::default())
}

// =========================================================================
// DISTRACTOR: constructs BenchDerivedStruct via explicit field-init, NOT
// via `default()`. Must NOT be in the answer.
// =========================================================================

pub fn site_distractor() -> BenchDerivedStruct {
    BenchDerivedStruct {
        name: "explicit".to_string(),
        count: 0,
    }
}
