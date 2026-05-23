//! Shared test helpers and child module declarations for the docs command tests.
//!
//! Helpers extracted from the original monolithic `#[cfg(test)] mod tests` block
//! in docs.rs (pre ORB-00250 split). Each per-submodule test file does
//! `use super::*;` to obtain these + `use super::super::<sibling>::<Item>;` for
//! production items (per test_layout.md and sibling layout convention).

mod add_root;
mod artifact_ref;
mod config;
mod frontmatter;
mod migrate;
mod path_util;
mod search;
mod walk;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use orbit_common::types::OrbitError;
use serde_yaml;

use super::frontmatter::parse_doc_frontmatter_strict;
use super::types::DocFrontmatter;

pub(crate) fn parse_frontmatter(raw: &str) -> Result<DocFrontmatter, OrbitError> {
    parse_doc_frontmatter_strict(Path::new("docs/example.md"), raw)
}

pub(crate) fn yaml_string<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a str> {
    mapping
        .get(serde_yaml::Value::String(key.to_string()))
        .and_then(serde_yaml::Value::as_str)
}

pub(crate) fn apply_patch(root: &Path, diff: &str, dry_run: bool) {
    let mut command = Command::new("patch");
    command.arg("-p0").current_dir(root).stdin(Stdio::piped());
    if dry_run {
        command.arg("--dry-run");
    }
    let mut child = command.spawn().expect("spawn patch");
    child
        .stdin
        .as_mut()
        .expect("patch stdin")
        .write_all(diff.as_bytes())
        .expect("write patch");
    let output = child.wait_with_output().expect("patch output");
    assert!(
        output.status.success(),
        "patch failed\nstdout:\n{}\nstderr:\n{}",
        std::string::String::from_utf8_lossy(&output.stdout),
        std::string::String::from_utf8_lossy(&output.stderr)
    );
}
