//! Unit tests for `subprocess` retry hygiene (ORB-10006) — sibling layout.

use crate::subprocess::{retry_backoff_bound_ms, spawn_error_is_permanent};

#[test]
fn spawn_error_classification_table() {
    use std::io::{Error, ErrorKind};
    // Deterministic spawn failures are permanent; resource exhaustion and
    // anything unrecognized stays transient (conservative toward retrying).
    let table = [
        (ErrorKind::NotFound, true),
        (ErrorKind::PermissionDenied, true),
        (ErrorKind::WouldBlock, false),  // EAGAIN
        (ErrorKind::OutOfMemory, false), // ENOMEM
        (ErrorKind::Interrupted, false),
        (ErrorKind::Other, false),
    ];
    for (kind, expect_permanent) in table {
        assert_eq!(
            spawn_error_is_permanent(&Error::new(kind, "boom")),
            expect_permanent,
            "kind {kind:?} misclassified"
        );
    }
}

#[test]
fn retry_backoff_bound_doubles_and_saturates_at_cap() {
    let first = retry_backoff_bound_ms(1);
    let second = retry_backoff_bound_ms(2);
    assert_eq!(second, first * 2, "bound must double per attempt");
    let mut previous = 0;
    for attempt in 1..12 {
        let bound = retry_backoff_bound_ms(attempt);
        assert!(bound >= previous, "bound shrank at attempt {attempt}");
        previous = bound;
    }
    assert_eq!(previous, retry_backoff_bound_ms(30), "bound must saturate");
}

#[cfg(unix)]
mod fake_companion {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use crate::embedder::Embedder;
    use crate::subprocess::SubprocessEmbedder;

    /// Shared JSON-Lines dispatcher for the fake companion. `$EMBED_BODY`
    /// is spliced per test to control embed behavior.
    fn write_companion_script(dir: &Path, embed_body: &str) -> PathBuf {
        let path = dir.join("fake-companion.sh");
        let script = format!(
            r#"#!/bin/sh
while read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"info"'*)
      printf '{{"id":%s,"result":{{"model_id":"fake","dim":2,"max_input_tokens":16,"version":null}}}}\n' "$id" ;;
    *'"method":"embed"'*)
      {embed_body} ;;
    *'"method":"exit"'*)
      printf '{{"id":%s,"result":{{"ok":true}}}}\n' "$id"; exit 0 ;;
    *)
      printf '{{"id":%s,"error":{{"code":"bad_request","message":"unknown method"}}}}\n' "$id" ;;
  esac
done
"#
        );
        std::fs::write(&path, script).expect("write fake companion");
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod fake companion");
        path
    }

    #[test]
    fn transport_failure_respawns_companion_and_recovers() {
        // First-generation companion dies on the first embed request (EOF
        // mid-RPC — a transient transport failure). The embedder must
        // respawn it and replay the request; the second generation answers.
        let temp = tempfile::tempdir().expect("tempdir");
        let marker = temp.path().join("crashed-once");
        let embed_body = format!(
            r#"if [ ! -f "{marker}" ]; then touch "{marker}"; exit 1; fi
      printf '{{"id":%s,"result":{{"vectors":[[0.5,0.25]]}}}}\n' "$id""#,
            marker = marker.display()
        );
        let script = write_companion_script(temp.path(), &embed_body);

        let embedder =
            SubprocessEmbedder::with_path_and_model(script, "fake").expect("construct embedder");
        let vectors = embedder
            .embed(&["hello"])
            .expect("embed must succeed after respawn");
        assert_eq!(vectors, vec![vec![0.5, 0.25]]);
        assert!(marker.exists(), "first generation should have crashed");
    }

    #[test]
    fn companion_reported_error_is_permanent_and_not_retried() {
        // A companion-reported RPC error is deterministic — the embedder
        // must surface it immediately without burning respawn attempts.
        let temp = tempfile::tempdir().expect("tempdir");
        let log = temp.path().join("embed-requests.log");
        let embed_body = format!(
            r#"printf 'x' >> "{log}"
      printf '{{"id":%s,"error":{{"code":"input_too_large","message":"nope"}}}}\n' "$id""#,
            log = log.display()
        );
        let script = write_companion_script(temp.path(), &embed_body);

        let embedder =
            SubprocessEmbedder::with_path_and_model(script, "fake").expect("construct embedder");
        let err = embedder
            .embed(&["hello"])
            .expect_err("companion error must surface");
        assert!(
            err.to_string().contains("input_too_large"),
            "error should carry the companion code: {err}"
        );
        let attempts = std::fs::read(&log).expect("embed log").len();
        assert_eq!(attempts, 1, "permanent RPC error must not be retried");
    }

    #[test]
    fn missing_companion_binary_fails_without_retry_delay() {
        let started = std::time::Instant::now();
        let err = match SubprocessEmbedder::with_path_and_model(
            PathBuf::from("/nonexistent/orbit-fake-companion"),
            "fake",
        ) {
            Ok(_) => panic!("missing binary must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("/nonexistent/orbit-fake-companion"),
            "error should name the binary: {err}"
        );
        // Permanent classification skips the backoff sleeps entirely.
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "ENOENT should fail fast, not retry"
        );
    }
}
