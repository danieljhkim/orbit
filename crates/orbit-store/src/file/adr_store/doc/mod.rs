use orbit_common::types::Adr;
use serde::{Deserialize, Serialize};

/// On-disk shape of an ADR record (the contents of `adr.yaml`).
///
/// Wraps an in-memory [`Adr`] with the persisted `schema_version` field so that
/// future schema bumps can migrate older files without changing the in-memory
/// type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct AdrFileDocument {
    pub(super) schema_version: u8,
    #[serde(flatten)]
    pub(super) adr: Adr,
}

#[cfg(test)]
mod tests;
