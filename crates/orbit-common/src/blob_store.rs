//! Content-addressed blob store.
//!
//! Writes bytes to `{root}/{hash[..2]}/{hash}` keyed by sha256 of the
//! post-redaction content. De-duplicates: if the target path already exists
//! the write is a no-op. Intended for audit/verbatim payload storage where
//! events reference blobs by hash rather than path.
//!
//! Redaction runs at write time via a [`PatternRedactor`]; the stored bytes
//! are already safe, so read-side tooling does not need to re-apply it.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::redaction::PatternRedactor;

pub struct BlobStore {
    root: PathBuf,
    redactor: PatternRedactor,
}

impl BlobStore {
    pub fn new<P: Into<PathBuf>>(root: P) -> Self {
        Self {
            root: root.into(),
            redactor: PatternRedactor::http_default(),
        }
    }

    pub fn with_redaction(mut self, redactor: PatternRedactor) -> Self {
        self.redactor = redactor;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn write(&self, content: &[u8]) -> io::Result<String> {
        let redacted = self.redactor.apply_bytes(content);
        let hash = sha256_hex(&redacted);
        let dir = self.root.join(&hash[..2]);
        fs::create_dir_all(&dir)?;
        let path = dir.join(&hash);
        if !path.exists() {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .or_else(|err| {
                    if err.kind() == io::ErrorKind::AlreadyExists {
                        fs::OpenOptions::new().write(true).open(&path)
                    } else {
                        Err(err)
                    }
                })?;
            f.write_all(&redacted)?;
            f.flush()?;
        }
        Ok(hash)
    }

    pub fn read(&self, sha256: &str) -> io::Result<Vec<u8>> {
        let path = self.root.join(&sha256[..2]).join(sha256);
        fs::read(path)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}
