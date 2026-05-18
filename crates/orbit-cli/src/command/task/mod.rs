mod add;
pub(crate) mod artifact;
pub mod artifacts;
mod command;
mod lifecycle;
mod lint;
mod list;
pub(crate) mod output;
mod prune;
mod review;
mod show;
mod templates;
mod update;

pub use command::{TaskCommand, TaskSubcommand};
// Re-export retained after ORB-00146 (web dashboard moved); the symbol was
// consumed by the dashboard API and is now unused in CLI proper.
#[allow(unused_imports)]
pub(crate) use list::task_locks_json;
