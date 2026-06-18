use std::io::{Read, Write};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

use orbit_common::utility::output_capture::{BoundedOutputCapture, capture_limit_from_env};
use orbit_common::utility::redaction::redact_sensitive_env_text;

pub(super) const ORBIT_EXEC_OUTPUT_CAPTURE_LIMIT_ENV: &str =
    "ORBIT_EXEC_OUTPUT_CAPTURE_LIMIT_BYTES";
pub(super) const DEFAULT_OUTPUT_CAPTURE_LIMIT_BYTES: usize = 1024 * 1024;

pub(super) fn output_capture_limit() -> usize {
    capture_limit_from_env(
        ORBIT_EXEC_OUTPUT_CAPTURE_LIMIT_ENV,
        DEFAULT_OUTPUT_CAPTURE_LIMIT_BYTES,
    )
}

pub(super) fn spawn_stdout_drain<R>(
    mut out: R,
    debug: bool,
    limit: usize,
    limit_tx: Sender<&'static str>,
) -> JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut capture = BoundedOutputCapture::new(limit);
        let mut chunk = [0u8; 4096];
        if debug {
            loop {
                match out.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Redact sensitive env values before printing to stderr
                        // so tokens/secrets are never shown in debug output.
                        let raw = String::from_utf8_lossy(&chunk[..n]);
                        let redacted = redact_sensitive_env_text(&raw);
                        let _ = std::io::stderr().write_all(redacted.as_bytes());
                        if capture.push(&chunk[..n]) {
                            let _ = limit_tx.send("stdout");
                            break;
                        }
                    }
                }
            }
        } else {
            loop {
                match out.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if capture.push(&chunk[..n]) {
                            let _ = limit_tx.send("stdout");
                            break;
                        }
                    }
                }
            }
        }
        capture.into_bytes()
    })
}

pub(super) fn spawn_stderr_drain<R>(
    mut err: R,
    debug: bool,
    limit: usize,
    limit_tx: Sender<&'static str>,
) -> JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut capture = BoundedOutputCapture::new(limit);
        let mut chunk = [0u8; 4096];
        if debug {
            loop {
                match err.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let redacted = redact_chunk(&chunk[..n]);
                        let _ = std::io::stderr().write_all(&redacted);
                        if capture.push(&redacted) {
                            let _ = limit_tx.send("stderr");
                            break;
                        }
                    }
                }
            }
        } else {
            loop {
                match err.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if capture.push(&chunk[..n]) {
                            let _ = limit_tx.send("stderr");
                            break;
                        }
                    }
                }
            }
        }
        capture.into_bytes()
    })
}

pub(super) fn spawn_stdin_write<W>(
    mut stdin: W,
    bytes: Vec<u8>,
    result_tx: Sender<Result<(), String>>,
) -> JoinHandle<()>
where
    W: Write + Send + 'static,
{
    thread::spawn(move || {
        let result = stdin
            .write_all(&bytes)
            .map_err(|e| format!("failed to write process stdin: {e}"));
        let _ = result_tx.send(result);
    })
}

fn redact_chunk(chunk: &[u8]) -> Vec<u8> {
    redact_sensitive_env_text(&String::from_utf8_lossy(chunk)).into_bytes()
}
