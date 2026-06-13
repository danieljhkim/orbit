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
fn write_redacts_secret_patterns_in_non_utf8_blob() {
    let temp = tempdir().expect("tempdir");
    let store = BlobStore::new(temp.path());
    let secret = b"nonutf-secret-token";
    let mut raw = b"stdout prefix\nAuthorization: Bearer ".to_vec();
    raw.extend_from_slice(secret);
    raw.extend_from_slice(b"\ninvalid byte follows: ");
    raw.push(0xff);

    let hash = store.write(&raw).expect("write blob");
    let stored = store.read(&hash).expect("read blob");
    let stored_text = String::from_utf8(stored.clone()).expect("stored lossy utf8");
    let expected = redact_all(&String::from_utf8_lossy(&raw));

    assert!(!stored.windows(secret.len()).any(|window| window == secret));
    assert!(!stored_text.contains("nonutf-secret-token"));
    assert!(stored_text.contains("Authorization: [REDACTED_AUTH]"));
    assert_eq!(hash, sha256_hex(expected.as_bytes()));
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

#[cfg(unix)]
#[test]
fn write_creates_private_blob_file_and_dirs() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("audit").join("blobs");
    let store = BlobStore::new(&root);

    let hash = store.write(b"audit payload").expect("write blob");
    let shard_dir = root.join(&hash[..2]);
    let blob_path = shard_dir.join(&hash);

    assert_eq!(mode(&blob_path), 0o600);
    assert_eq!(mode(&root), 0o700);
    assert_eq!(mode(&shard_dir), 0o700);
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

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}
