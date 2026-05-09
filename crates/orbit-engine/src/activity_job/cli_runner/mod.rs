mod argv;
mod envelope;
mod orchestrator;
mod spawn;
mod supervisor;

#[cfg(all(test, target_os = "macos"))]
mod orchestrator_macos_tests;
#[cfg(test)]
mod orchestrator_tests;
#[cfg(test)]
mod test_support;

pub(super) use envelope::task_id_from_input;
pub use orchestrator::run_cli_backend;
