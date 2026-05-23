//! Background worker that drains an mpsc channel of `EmbedJob`s and feeds
//! them into a long-lived `SubprocessEmbedder`. Used by `orbit-core`'s task
//! lifecycle hooks (create / update / delete) so that mutations enqueue
//! best-effort indexing without blocking the caller. Failures log at debug
//! and never propagate.

use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread;

use orbit_common::types::Task;

use crate::SubprocessEmbedder;

use super::store::VectorStore;

const EMBED_BATCH_SIZE: usize = 16;

#[derive(Debug, Clone)]
pub struct EmbedJob {
    pub task: Task,
    pub force: bool,
}

#[derive(Clone)]
pub struct EmbedWorker {
    sender: SyncSender<EmbedJob>,
}

impl EmbedWorker {
    pub fn start(store: VectorStore) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<EmbedJob>(128);
        thread::spawn(move || {
            let mut embedder: Option<SubprocessEmbedder> = None;
            while let Ok(first) = receiver.recv() {
                let mut batch = vec![first];
                while batch.len() < EMBED_BATCH_SIZE {
                    match receiver.try_recv() {
                        Ok(job) => batch.push(job),
                        Err(_) => break,
                    }
                }
                if embedder.is_none() {
                    match SubprocessEmbedder::quiet_with_model(crate::DEFAULT_MODEL) {
                        Ok(value) => embedder = Some(value),
                        Err(error) => {
                            orbit_common::tracing::debug!(
                                target: "orbit.search.indexer",
                                error = %error,
                                "semantic indexing skipped because embedder initialization failed",
                            );
                            continue;
                        }
                    }
                }
                let Some(active_embedder) = embedder.as_ref() else {
                    continue;
                };
                for job in &batch {
                    if let Err(error) = store.index_task(&job.task, active_embedder, job.force) {
                        orbit_common::tracing::debug!(
                            target: "orbit.search.indexer",
                            task_id = job.task.id.as_str(),
                            error = %error,
                            "semantic indexing failed after task mutation",
                        );
                    }
                }
            }
        });
        Self { sender }
    }

    pub fn enqueue(&self, task: Task) {
        match self.sender.try_send(EmbedJob { task, force: false }) {
            Ok(()) => {}
            Err(TrySendError::Full(job)) => {
                orbit_common::tracing::debug!(
                    target: "orbit.search.indexer",
                    task_id = job.task.id.as_str(),
                    "semantic indexing queue is full; dropping task update",
                );
            }
            Err(TrySendError::Disconnected(_)) => {
                orbit_common::tracing::debug!(
                    target: "orbit.search.indexer",
                    "semantic indexing queue is disconnected; dropping task update",
                );
            }
        }
    }
}
