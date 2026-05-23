mod command;
pub mod format;
pub mod tail;

pub use command::LogCommand;

#[cfg(test)]
mod tests;
