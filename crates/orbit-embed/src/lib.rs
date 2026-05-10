#![deny(clippy::print_stderr, clippy::print_stdout)]
//! Slim embedding client surface for Orbit semantic indexing.
//!
//! This crate intentionally contains no inference backend. The main `orbit`
//! binary links this crate, locates the separately installed companion binary,
//! and speaks a small JSON-Lines RPC protocol over stdio.
//!
//! Module layout — start with the entry point that matches your need:
//!
//! - [`embedder`] — the [`Embedder`] trait + [`ModelSpec`] catalog. Read first
//!   if you're integrating a new caller; everything else is downstream of this.
//! - [`rpc`] — the JSON-Lines protocol shared with `orbit-embed-companion`.
//! - [`companion`] — discovery of the installed companion binary.
//! - [`noop`] — a deterministic test fake that needs no companion subprocess.
//! - [`subprocess`] — the production [`Embedder`] impl that talks to the
//!   companion over stdio.
//! - [`vector`] — workspace-local SQLite storage for embeddings + FTS5 rows.
//! - [`commands`] — install / uninstall / reindex / stats command surface.

pub mod commands;
pub mod companion;
pub mod embedder;
pub mod noop;
pub mod rpc;
pub mod subprocess;
pub mod vector;

pub use companion::{
    CompanionPaths, INSTALL_REMEDIATION, locate_companion, platform_companion_filename, platform_id,
};
pub use embedder::{DEFAULT_MODEL, Embedder, ModelSpec, default_model, supported_models};
pub use noop::NoopEmbedder;
pub use rpc::{RpcError, RpcRequest, RpcResponse, RpcResult};
pub use subprocess::SubprocessEmbedder;
