//! Regression coverage for legacy `models:` executor definitions.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use orbit_store::global_executor_def_store;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
struct CaptureMakeWriter(Arc<Mutex<Vec<u8>>>);

struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl<'a> MakeWriter<'a> for CaptureMakeWriter {
    type Writer = CaptureWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CaptureWriter(Arc::clone(&self.0))
    }
}

impl Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("capture writer lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_warnings<F, T>(f: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(CaptureMakeWriter(Arc::clone(&buffer)))
        .with_max_level(LevelFilter::WARN)
        .with_target(true)
        .with_ansi(false)
        .without_time()
        .finish();
    let result = tracing::subscriber::with_default(subscriber, f);
    let logs = String::from_utf8(buffer.lock().expect("capture buffer lock").clone())
        .expect("captured logs are utf8");
    (result, logs)
}

#[test]
fn loading_legacy_models_fixture_warns_once_per_def() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let store = global_executor_def_store(fixtures);

    let (defs, logs) = capture_warnings(|| store.list_executor_defs().expect("load fixtures"));

    assert_eq!(defs.len(), 1);
    let pair = defs[0]
        .model_pair_override()
        .expect("legacy models become model_pair_override");
    assert_eq!(pair.strong, "gemini-3.1-pro");
    assert_eq!(pair.weak, "gemini-3-flash");
    assert_eq!(
        logs.matches("deprecated `models` key").count(),
        1,
        "expected one deprecation warning, got: {logs}"
    );
    assert!(
        logs.contains("orbit.executor.def"),
        "warning should use executor def target: {logs}"
    );
}
