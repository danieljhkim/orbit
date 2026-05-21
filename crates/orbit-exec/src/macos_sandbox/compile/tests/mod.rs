#![allow(missing_docs)]

//! Per-concern unit and integration tests for SBPL profile compilation.
//! Logic tests exercise the pure emission rules and env handling.
//! Integration tests (macOS only) exercise the compiled profiles against the
//! real sandbox-exec kernel enforcement.

mod logic;

#[cfg(target_os = "macos")]
mod integration;
