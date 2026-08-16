//! Helpers for constructing command lines that cross a shell boundary.

/// Quote one argument for safe interpolation into a POSIX shell command.
///
/// This returns a single-quoted word. Embedded single quotes are represented
/// by ending the quoted section, emitting an escaped quote, and reopening it.
/// It is intended for boundaries such as SSH, where a command string is parsed
/// by a remote shell.
pub fn quote_posix_arg(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
