use std::env;

pub const OUTPUT_TRUNCATED_MARKER: &[u8] = b"\n[orbit: output truncated after capture limit]\n";

#[derive(Debug, Clone)]
pub struct BoundedOutputCapture {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}

impl BoundedOutputCapture {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            truncated: false,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> bool {
        if self.truncated {
            return false;
        }

        let remaining = self.limit.saturating_sub(self.bytes.len());
        if chunk.len() <= remaining {
            self.bytes.extend_from_slice(chunk);
            return false;
        }

        if remaining > 0 {
            self.bytes.extend_from_slice(&chunk[..remaining]);
        }
        self.bytes.extend_from_slice(OUTPUT_TRUNCATED_MARKER);
        self.truncated = true;
        true
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

pub fn capture_limit_from_env(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|limit| *limit > 0)
        .unwrap_or(default)
}
