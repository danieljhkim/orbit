//! Pure task-bundle file persistence: codecs, atomic publication, and bundle
//! lock-sentinel mechanics. Registry and checkout projection coordination live
//! in the task repository.

pub(crate) mod bundle_io;
mod lock;
mod migrations;
mod types;

pub(crate) use bundle_io::{
    append_jsonl_row, cleanup_partial_bundle_best_effort, read_bundle_at, write_bundle_at,
    write_bundle_with_artifacts_at,
};
pub(crate) use lock::{remove_task_bundle_lock_sentinel, task_bundle_lock_sentinel_path};
pub(crate) use types::{TaskBundleV2, TaskDocumentV2};
