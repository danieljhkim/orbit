#![deny(clippy::print_stderr, clippy::print_stdout)]
//! Slim embedding client surface for Orbit semantic indexing.
//!
//! This crate intentionally contains no inference backend. The main `orbit`
//! binary links this crate, locates the separately installed companion binary,
//! and speaks a small JSON-Lines RPC protocol over stdio.

use std::env;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

pub const DEFAULT_MODEL: &str = "bge-small";
pub const INSTALL_REMEDIATION: &str = "Semantic search not enabled. Run `orbit semantic install` to download the inference companion.";

pub trait Embedder: Send + Sync {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    fn max_input_tokens(&self) -> usize;
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, OrbitError>;
    fn token_count(&self, text: &str) -> Result<usize, OrbitError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpec {
    pub alias: &'static str,
    pub fastembed_name: &'static str,
    pub dim: usize,
    pub max_input_tokens: usize,
}

impl ModelSpec {
    pub fn parse(value: &str) -> Result<Self, OrbitError> {
        let normalized = value.trim().to_ascii_lowercase();
        supported_models()
            .iter()
            .copied()
            .find(|model| {
                model.alias == normalized || model.fastembed_name.eq_ignore_ascii_case(value.trim())
            })
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "unsupported semantic model `{value}`; expected one of: {}",
                    supported_models()
                        .iter()
                        .map(|model| model.alias)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
    }
}

pub fn default_model() -> ModelSpec {
    ModelSpec::parse(DEFAULT_MODEL).expect("default semantic model is supported")
}

pub fn supported_models() -> &'static [ModelSpec] {
    &[
        ModelSpec {
            alias: "bge-small",
            fastembed_name: "BGESmallENV15",
            dim: 384,
            max_input_tokens: 512,
        },
        ModelSpec {
            alias: "minilm-l6",
            fastembed_name: "AllMiniLML6V2",
            dim: 384,
            max_input_tokens: 512,
        },
        ModelSpec {
            alias: "nomic-v1.5",
            fastembed_name: "NomicEmbedTextV15",
            dim: 768,
            max_input_tokens: 8192,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum RpcRequest {
    Info { id: u64 },
    Embed { id: u64, texts: Vec<String> },
    TokenCount { id: u64, text: String },
    Exit { id: u64 },
}

impl RpcRequest {
    pub fn id(&self) -> u64 {
        match self {
            Self::Info { id }
            | Self::Embed { id, .. }
            | Self::TokenCount { id, .. }
            | Self::Exit { id } => *id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RpcResponse {
    Result { id: u64, result: RpcResult },
    Error { id: u64, error: RpcError },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RpcResult {
    Info {
        model_id: String,
        dim: usize,
        max_input_tokens: usize,
        version: Option<String>,
    },
    Embed {
        vectors: Vec<Vec<f32>>,
    },
    TokenCount {
        tokens: usize,
    },
    Exit {
        ok: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct NoopEmbedder {
    model_id: String,
    dim: usize,
    max_input_tokens: usize,
}

impl NoopEmbedder {
    pub fn new(model_id: impl Into<String>, dim: usize, max_input_tokens: usize) -> Self {
        Self {
            model_id: model_id.into(),
            dim,
            max_input_tokens,
        }
    }

    pub fn small() -> Self {
        Self::new("noop", 3, 512)
    }
}

impl Default for NoopEmbedder {
    fn default() -> Self {
        Self::small()
    }
}

impl Embedder for NoopEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn max_input_tokens(&self) -> usize {
        self.max_input_tokens
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, OrbitError> {
        Ok(texts
            .iter()
            .map(|text| noop_vector(text, self.dim))
            .collect())
    }

    fn token_count(&self, text: &str) -> Result<usize, OrbitError> {
        Ok(text.split_whitespace().count().max(1))
    }
}

fn noop_vector(text: &str, dim: usize) -> Vec<f32> {
    let mut state = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x100000001b3);
    }

    let mut values = Vec::with_capacity(dim);
    let mut next = state;
    for _ in 0..dim {
        next ^= next << 13;
        next ^= next >> 7;
        next ^= next << 17;
        let scaled = (next as f64 / u64::MAX as f64) as f32;
        values.push((scaled * 2.0) - 1.0);
    }
    normalize(values)
}

fn normalize(mut values: Vec<f32>) -> Vec<f32> {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut values {
            *value /= norm;
        }
    }
    values
}

#[derive(Debug, Clone)]
pub struct CompanionPaths {
    pub root: PathBuf,
    pub bin_dir: PathBuf,
    pub models_dir: PathBuf,
    pub active_model_path: PathBuf,
}

impl CompanionPaths {
    pub fn default_under_home() -> Result<Self, OrbitError> {
        let root = home_dir()
            .ok_or_else(|| OrbitError::InvalidInput("HOME/USERPROFILE is not set".to_string()))?
            .join(".orbit")
            .join("embed");
        Ok(Self::new(root))
    }

    pub fn new(root: PathBuf) -> Self {
        Self {
            bin_dir: root.join("bin"),
            models_dir: root.join("models"),
            active_model_path: root.join("active-model"),
            root,
        }
    }

    pub fn companion_path(&self) -> PathBuf {
        self.bin_dir.join(platform_companion_filename())
    }

    pub fn model_dir(&self, model_id: &str) -> PathBuf {
        self.models_dir.join(model_id)
    }
}

pub fn platform_companion_filename() -> String {
    if cfg!(windows) {
        format!("orbit-embed-companion-{}.exe", platform_id())
    } else {
        format!("orbit-embed-companion-{}", platform_id())
    }
}

pub fn platform_id() -> &'static str {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => "macos-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("linux", "x86_64") => "linux-x86_64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => "unknown",
    }
}

pub fn locate_companion() -> Result<PathBuf, OrbitError> {
    if let Ok(path) = env::var("ORBIT_EMBED_COMPANION") {
        let path = PathBuf::from(path);
        if is_executable_file(&path) {
            return Ok(path);
        }
    }

    if let Ok(paths) = CompanionPaths::default_under_home() {
        let standard = paths.companion_path();
        if is_executable_file(&standard) {
            return Ok(standard);
        }
    }

    for name in path_candidate_names() {
        if let Some(path) = find_on_path(&name) {
            return Ok(path);
        }
    }

    Err(OrbitError::CompanionNotInstalled(
        INSTALL_REMEDIATION.to_string(),
    ))
}

fn path_candidate_names() -> Vec<OsString> {
    let mut names = vec![OsString::from("orbit-embed-companion")];
    names.push(OsString::from(platform_companion_filename()));
    if cfg!(windows) {
        names.push(OsString::from("orbit-embed-companion.exe"));
    }
    names
}

fn find_on_path(name: &OsString) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn home_dir() -> Option<PathBuf> {
    env::var("HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var("USERPROFILE")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        })
}

pub struct SubprocessEmbedder {
    model_id: String,
    dim: usize,
    max_input_tokens: usize,
    next_id: AtomicU64,
    io: Mutex<ChildIo>,
}

struct ChildIo {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl SubprocessEmbedder {
    pub fn new() -> Result<Self, OrbitError> {
        Self::with_model(DEFAULT_MODEL)
    }

    pub fn with_model(model: &str) -> Result<Self, OrbitError> {
        Self::with_path_and_model(locate_companion()?, model)
    }

    pub fn with_path_and_model(path: PathBuf, model: &str) -> Result<Self, OrbitError> {
        let mut child = Command::new(&path)
            .arg("--model")
            .arg(model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                OrbitError::Execution(format!(
                    "failed to spawn embedding companion '{}': {error}",
                    path.display()
                ))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| OrbitError::Execution("companion stdin unavailable".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OrbitError::Execution("companion stdout unavailable".to_string()))?;
        let mut embedder = Self {
            model_id: String::new(),
            dim: 0,
            max_input_tokens: 0,
            next_id: AtomicU64::new(1),
            io: Mutex::new(ChildIo {
                child,
                stdin,
                stdout: BufReader::new(stdout),
            }),
        };
        let info = embedder.request(RpcRequest::Info { id: 0 })?;
        let RpcResult::Info {
            model_id,
            dim,
            max_input_tokens,
            ..
        } = info
        else {
            return Err(OrbitError::AgentProtocolViolation(
                "companion returned non-info response to info request".to_string(),
            ));
        };
        embedder.model_id = model_id;
        embedder.dim = dim;
        embedder.max_input_tokens = max_input_tokens;
        Ok(embedder)
    }

    fn request(&self, request: RpcRequest) -> Result<RpcResult, OrbitError> {
        let request = match request {
            RpcRequest::Info { id: 0 } => RpcRequest::Info { id: 1 },
            RpcRequest::Info { .. } => RpcRequest::Info {
                id: self.next_request_id(),
            },
            RpcRequest::Embed { texts, .. } => RpcRequest::Embed {
                id: self.next_request_id(),
                texts,
            },
            RpcRequest::TokenCount { text, .. } => RpcRequest::TokenCount {
                id: self.next_request_id(),
                text,
            },
            RpcRequest::Exit { .. } => RpcRequest::Exit {
                id: self.next_request_id(),
            },
        };
        let id = request.id();
        let mut io = self
            .io
            .lock()
            .map_err(|error| OrbitError::Execution(format!("companion mutex poisoned: {error}")))?;
        let line = serde_json::to_string(&request)
            .map_err(|error| OrbitError::Execution(error.to_string()))?;
        io.stdin
            .write_all(line.as_bytes())
            .and_then(|_| io.stdin.write_all(b"\n"))
            .and_then(|_| io.stdin.flush())
            .map_err(|error| {
                OrbitError::Execution(format!("failed to write companion RPC: {error}"))
            })?;

        let mut response_line = String::new();
        let read = io.stdout.read_line(&mut response_line).map_err(|error| {
            OrbitError::Execution(format!("failed to read companion RPC: {error}"))
        })?;
        if read == 0 {
            return Err(OrbitError::AgentProtocolViolation(
                "embedding companion exited before sending a response".to_string(),
            ));
        }
        let response: RpcResponse = serde_json::from_str(&response_line)
            .map_err(|error| OrbitError::AgentProtocolViolation(error.to_string()))?;
        match response {
            RpcResponse::Result {
                id: response_id,
                result,
            } if response_id == id => Ok(result),
            RpcResponse::Error {
                id: response_id,
                error,
            } if response_id == id => Err(OrbitError::Execution(format!(
                "embedding companion {}: {}",
                error.code, error.message
            ))),
            other => Err(OrbitError::AgentProtocolViolation(format!(
                "companion response id mismatch for request {id}: {other:?}"
            ))),
        }
    }

    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

impl Embedder for SubprocessEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn max_input_tokens(&self) -> usize {
        self.max_input_tokens
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, OrbitError> {
        let result = self.request(RpcRequest::Embed {
            id: 0,
            texts: texts.iter().map(|text| (*text).to_string()).collect(),
        })?;
        match result {
            RpcResult::Embed { vectors } => Ok(vectors),
            _ => Err(OrbitError::AgentProtocolViolation(
                "companion returned non-embed response to embed request".to_string(),
            )),
        }
    }

    fn token_count(&self, text: &str) -> Result<usize, OrbitError> {
        let result = self.request(RpcRequest::TokenCount {
            id: 0,
            text: text.to_string(),
        })?;
        match result {
            RpcResult::TokenCount { tokens } => Ok(tokens),
            _ => Err(OrbitError::AgentProtocolViolation(
                "companion returned non-token_count response".to_string(),
            )),
        }
    }
}

impl Drop for SubprocessEmbedder {
    fn drop(&mut self) {
        let Ok(mut io) = self.io.lock() else {
            return;
        };
        if let Ok(line) = serde_json::to_string(&RpcRequest::Exit { id: 9_999_999 }) {
            let _ = io.stdin.write_all(line.as_bytes());
            let _ = io.stdin.write_all(b"\n");
            let _ = io.stdin.flush();
            let mut response = String::new();
            let _ = io.stdout.read_line(&mut response);
        }
        let _ = io.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_embedder_is_deterministic_and_normalized() {
        let embedder = NoopEmbedder::small();
        let vectors = embedder.embed(&["alpha", "alpha", "beta"]).unwrap();
        assert_eq!(vectors[0], vectors[1]);
        assert_ne!(vectors[0], vectors[2]);
        let norm = vectors[0]
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        assert!((norm - 1.0).abs() < 0.0001);
    }

    #[test]
    fn model_aliases_parse() {
        assert_eq!(ModelSpec::parse("bge-small").unwrap().dim, 384);
        assert_eq!(
            ModelSpec::parse("NomicEmbedTextV15").unwrap().alias,
            "nomic-v1.5"
        );
        assert!(ModelSpec::parse("unknown").is_err());
    }
}
