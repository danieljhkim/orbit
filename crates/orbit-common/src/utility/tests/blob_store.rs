#![allow(missing_docs)]

use std::ffi::OsString;

use sha2::{Digest, Sha256};
use tempfile::tempdir;

use super::super::blob_store::BlobStore;
use super::super::redaction::{PatternRedactor, redact_all};

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this test uses a dedicated variable name and restores the
        // previous value on drop.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: see EnvVarGuard::set.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[test]
fn write_hashes_and_stores_env_redacted_bytes() {
    let temp = tempdir().expect("tempdir");
    let secret = "live-audit-blob-secret-value";
    let _guard = EnvVarGuard::set("ORBIT_BLOB_STORE_TEST_TOKEN", secret);
    let store = BlobStore::new(temp.path());
    let raw = format!("stdout contains {secret}\nAuthorization: Bearer pattern-secret-token\n");

    let hash = store.write(raw.as_bytes()).expect("write blob");
    let stored = store.read(&hash).expect("read blob");
    let stored_text = String::from_utf8(stored).expect("stored utf8");

    assert!(!stored_text.contains(secret));
    assert!(!stored_text.contains("pattern-secret-token"));
    assert!(stored_text.contains("[REDACTED_ENV]"));
    assert!(stored_text.contains("[REDACTED_AUTH]"));
    assert_eq!(hash, sha256_hex(redact_all(&raw).as_bytes()));
}

#[test]
fn caller_redaction_cannot_weaken_default_redaction() {
    let temp = tempdir().expect("tempdir");
    let secret = "live-audit-blob-empty-redactor-secret";
    let _guard = EnvVarGuard::set("ORBIT_BLOB_EMPTY_REDACTOR_TOKEN", secret);
    let store = BlobStore::new(temp.path()).with_redaction(PatternRedactor::empty());
    let raw = format!("{secret}\n{{\"api_key\":\"json-secret\"}}\n");

    let hash = store.write(raw.as_bytes()).expect("write blob");
    let stored = store.read(&hash).expect("read blob");
    let stored_text = String::from_utf8(stored).expect("stored utf8");

    assert!(!stored_text.contains(secret));
    assert!(!stored_text.contains("json-secret"));
    assert!(stored_text.contains("[REDACTED_ENV]"));
    assert!(stored_text.contains("[REDACTED_AUTH]"));
    assert_eq!(hash, sha256_hex(redact_all(&raw).as_bytes()));
}

#[test]
fn caller_redaction_can_add_stronger_patterns() {
    let temp = tempdir().expect("tempdir");
    let store = BlobStore::new(temp.path()).with_redaction(PatternRedactor::with_argv_secrets());
    let raw = "argv accidentally contains sk-short\n";

    let hash = store.write(raw.as_bytes()).expect("write blob");
    let stored = store.read(&hash).expect("read blob");
    let stored_text = String::from_utf8(stored).expect("stored utf8");
    let expected = PatternRedactor::with_argv_secrets().apply_str(&redact_all(raw));

    assert!(!stored_text.contains("sk-short"));
    assert!(stored_text.contains("[REDACTED_API_KEY]"));
    assert_eq!(hash, sha256_hex(expected.as_bytes()));
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
