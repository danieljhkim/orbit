//! Durable host and workspace registry persistence.

mod hosts;
mod workspaces;

pub(crate) fn advance_registry_revision(
    conn: &rusqlite::Connection,
) -> Result<(), orbit_common::types::OrbitError> {
    hosts::advance_registry_revision(conn)
}
