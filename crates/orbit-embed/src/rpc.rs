//! JSON-Lines RPC envelope shared between the orbit binary and the
//! `orbit-embed-companion` subprocess. The protocol is deliberately small:
//! `info`, `embed`, `token_count`, `exit`. Both sides serialize via serde.

use serde::{Deserialize, Serialize};

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
