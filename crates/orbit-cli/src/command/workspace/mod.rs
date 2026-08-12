mod command;
mod init;
// Fleet registry-backed owner linking is retained for v2 but cannot compile
// into the v1 command graph (ADR-0358).
#[cfg(any())]
mod link;
mod list;
mod remove;
mod role;
mod show;
mod support;
mod teardown;

pub use command::{WorkspaceCommand, WorkspaceSubcommand};

#[cfg(test)]
mod tests;
