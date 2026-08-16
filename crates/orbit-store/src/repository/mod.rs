//! Live repositories that coordinate persistence drivers.

pub(crate) mod checkout_projection;
mod file_backends;
pub(crate) mod friction;
pub(crate) mod layered_policy;
pub(crate) mod sqlite_backends;
pub(crate) mod task;
pub(crate) mod token_scoreboard;

#[cfg(test)]
mod tests_file_backends;
