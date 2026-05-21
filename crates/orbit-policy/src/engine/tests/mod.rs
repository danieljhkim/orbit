#![allow(missing_docs)]

//! Boundary tests for `PolicyEngine::check`. These guard the global
//! `denyRead` / `denyModify` last-match-wins semantics, the unknown-profile
//! error path, and matched_rule observability for audit attribution.
//! See task T20260509-7.

use chrono::Utc;
use orbit_common::types::policy_def::FsProfile;
use std::collections::HashMap;

use orbit_common::types::{FsOperation, OrbitError, PolicyDef};

mod check;
mod errors;
mod overrides;

/// Shared test fixture builder. Constructs a minimal `PolicyDef` for
/// exercising `PolicyEngine::check` against profile rules and global denies.
pub(crate) fn make_def(
    deny_read: Vec<&str>,
    deny_modify: Vec<&str>,
    profiles: &[(&str, &[&str], &[&str])],
) -> PolicyDef {
    let mut fs_profiles = HashMap::new();
    for (name, read, modify) in profiles {
        fs_profiles.insert(
            (*name).to_string(),
            FsProfile {
                read: read.iter().map(|s| (*s).to_string()).collect(),
                modify: modify.iter().map(|s| (*s).to_string()).collect(),
            },
        );
    }
    PolicyDef {
        name: "test".to_string(),
        description: None,
        deny_read: deny_read.into_iter().map(String::from).collect(),
        deny_modify: deny_modify.into_iter().map(String::from).collect(),
        fs_profiles,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}
