//! Command implementations for all Orbit CLI subcommands.
//!
//! Each sub-module (task, job, activity, skill, audit, tool, init)
//! provides the data types and logic for one command group. Commands are
//! executed via the `Execute` trait, which receives an `&OrbitRuntime` and
//! produces an `OrbitError` on failure.
//!
//! The `init` module is special: it also provides `execute_without_runtime`
//! for bootstrapping a new Orbit root before a runtime can be constructed.
//! Default YAML assets (e.g., sample skills, config templates) are embedded
//! at compile time via `include_str!` and seeded to disk on first `orbit init`.

use std::borrow::Cow;
use std::path::Path;

use orbit_common::types::OrbitError;
use orbit_common::utility::fs::write_text_with_parent;

/// Audit identity used for system-initiated (non-agent) mutations.
/// `pub` because the direct v2 activity runner moved to `orbit-cmd`
/// [ORB-10016] and stamps the same identity.
pub const SYSTEM_AUDIT_IDENTITY: &str = "system";

/// Seed every `(name, content)` pair in `files` as `<dir>/<name>.yaml`,
/// skipping entries that already exist unless `overwrite` is set. `render`
/// maps each embedded asset's raw content to what actually gets written —
/// activity and job seeding pass content through unchanged; routine seeding
/// uses it for placeholder substitution and fail-closed validation.
pub(crate) fn seed_embedded_assets<'a>(
    dir: &Path,
    files: &'a [(&'a str, &'a str)],
    overwrite: bool,
    mut render: impl FnMut(&'a str, &'a str) -> Result<Cow<'a, str>, OrbitError>,
) -> Result<usize, OrbitError> {
    let mut count = 0usize;
    for (name, content) in files {
        let path = dir.join(format!("{name}.yaml"));
        if !overwrite && path.exists() {
            continue;
        }
        let rendered = render(name, content)?;
        write_text_with_parent(&path, &rendered)?;
        count += 1;
    }
    Ok(count)
}

pub(crate) mod activity;
pub mod audit_event;
pub mod backend_resolver;
pub(crate) mod docs;
pub(crate) mod executor;
pub mod gc;
pub(crate) mod id_allocation;
pub mod init;
pub mod job;
pub mod learning;
pub(crate) mod learning_authoring;
pub(crate) mod policy;
pub(crate) mod routine;
pub(crate) mod search;
pub mod semantic;
pub mod skill;
pub mod task;
pub mod task_migration;
pub mod tool;
pub(crate) mod workflow;

#[cfg(test)]
mod tests;
