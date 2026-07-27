//! Gemini-compatible HTTP transport.
//!
//! Implements the [`LoopTransport`](crate::loop_engine::LoopTransport) trait
//! against the Google Generative Language `generateContent` API, including
//! `cachedContents` support for large histories.

mod transport;
mod wire;

#[cfg(test)]
mod tests;

pub use transport::GeminiHttpTransport;
