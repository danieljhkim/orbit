mod redaction;
mod subscriber;

// Shared test helpers and utilities. Visible to child test submodules (subscriber, redaction)
// because they are descendants. These use private items of the parent logging module for
// test setup (e.g. jsonl_layer_at_path, emit_log_init_warning) which is allowed for
// submodules.

use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use serde_json::Value;
use tracing::Dispatch;
use tracing_subscriber::{
    Registry,
    fmt::{self, MakeWriter},
    layer::SubscriberExt,
};

use super::{RedactingFields, emit_log_init_warning, env_filter, jsonl_layer_at_path};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_test_subscriber_at_path<W>(
    default_filter: &str,
    log_path: &Path,
    stderr_writer: W,
    f: impl FnOnce(),
) -> io::Result<()>
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let filter = env_filter(default_filter);
    let stderr_layer = fmt::layer()
        .with_writer(stderr_writer)
        .fmt_fields(RedactingFields::default());
    let (file_layer, guard) = jsonl_layer_at_path(log_path)?;
    let subscriber = Registry::default()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer);
    let dispatch = Dispatch::new(subscriber);
    tracing::dispatcher::with_default(&dispatch, f);
    drop(guard);
    Ok(())
}

fn with_test_subscriber_allowing_file_failure<W>(
    default_filter: &str,
    log_path: &Path,
    stderr_writer: W,
    f: impl FnOnce(),
) -> Option<String>
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let filter = env_filter(default_filter);
    let stderr_layer = fmt::layer()
        .with_writer(stderr_writer)
        .fmt_fields(RedactingFields::default());
    match jsonl_layer_at_path(log_path) {
        Ok((file_layer, guard)) => {
            let subscriber = Registry::default()
                .with(filter)
                .with(stderr_layer)
                .with(file_layer);
            let dispatch = Dispatch::new(subscriber);
            tracing::dispatcher::with_default(&dispatch, f);
            drop(guard);
            None
        }
        Err(err) => {
            let warning = err.to_string();
            let subscriber = Registry::default().with(filter).with(stderr_layer);
            let dispatch = Dispatch::new(subscriber);
            tracing::dispatcher::with_default(&dispatch, || {
                emit_log_init_warning(&warning);
                f();
            });
            Some(warning)
        }
    }
}

fn read_jsonl_values(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .expect("read jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid json line"))
        .collect()
}

#[derive(Clone, Default)]
struct BufferMakeWriter {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl BufferMakeWriter {
    fn buffer(&self) -> Arc<Mutex<Vec<u8>>> {
        Arc::clone(&self.buffer)
    }
}

impl<'writer> MakeWriter<'writer> for BufferMakeWriter {
    type Writer = BufferWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        BufferWriter {
            buffer: Arc::clone(&self.buffer),
        }
    }
}

struct BufferWriter {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer
            .lock()
            .expect("buffer lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: OsString) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}
