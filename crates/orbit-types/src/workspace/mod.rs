//! Domain contracts for this Orbit types module.

mod error;
mod registry;
pub use error::WorkspaceError;

#[cfg(test)]
mod tests;

pub use registry::{
    WORKSPACE_REGISTRY_SCHEMA_VERSION, Workspace, WorkspaceCheckout, WorkspaceCheckoutRole,
    WorkspacePaths, WorkspaceRegistry, WorkspaceStatus,
};
