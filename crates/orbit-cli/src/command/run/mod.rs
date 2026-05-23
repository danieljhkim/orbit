mod command;
pub mod duel;
mod events;
mod format;
mod history;
pub mod job;
pub mod legacy_logs;
mod logs;
pub mod ship;
mod show;
mod steps;
pub(crate) mod support;
mod trace;

pub use command::{RunCommand, RunSubcommand};
// Re-export retained after ORB-00146 (web dashboard moved); the symbol was
// consumed by the dashboard API and is now unused in CLI proper.
#[allow(unused_imports)]
pub(crate) use job::job_run_to_json;
pub use job::{JobReplayArgs, JobRunArgs, JobRunPipelineWorkerArgs};
pub(crate) use show::{print_legacy_logs_summary, print_run_show};

#[cfg(test)]
mod tests;
