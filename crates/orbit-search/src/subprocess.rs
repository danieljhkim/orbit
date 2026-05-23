//! `Embedder` implementation that talks to the installed companion binary
//! over JSON-Lines stdio. The subprocess is kept alive across requests via
//! a `Mutex<ChildIo>`; `Drop` sends `Exit` and reaps the child.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_common::types::OrbitError;

use crate::companion::locate_companion;
use crate::embedder::{DEFAULT_MODEL, Embedder};
use crate::rpc::{RpcRequest, RpcResponse, RpcResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompanionStderr {
    Inherit,
    Suppress,
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
        Self::with_path_model_and_stderr(path, model, CompanionStderr::Inherit)
    }

    pub(crate) fn quiet_with_model(model: &str) -> Result<Self, OrbitError> {
        Self::with_path_model_and_stderr(locate_companion()?, model, CompanionStderr::Suppress)
    }

    fn with_path_model_and_stderr(
        path: PathBuf,
        model: &str,
        stderr: CompanionStderr,
    ) -> Result<Self, OrbitError> {
        let mut child = Command::new(&path)
            .arg("--model")
            .arg(model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr.stdio())
            .spawn()
            .map_err(|error| {
                OrbitError::Execution(format!(
                    "failed to spawn search companion '{}': {error}",
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
                "search companion exited before sending a response".to_string(),
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
                "search companion {}: {}",
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

impl CompanionStderr {
    fn stdio(self) -> Stdio {
        match self {
            Self::Inherit => Stdio::inherit(),
            Self::Suppress => Stdio::null(),
        }
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
