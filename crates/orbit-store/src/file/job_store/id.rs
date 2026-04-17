use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};

use super::JobFileStore;

#[derive(Default)]
struct IdGenerationState {
    last_base: String,
    next_suffix: u32,
}

impl JobFileStore {
    pub(super) fn next_job_id(&self) -> String {
        self.next_timestamped_id("job", |candidate| {
            self.job_path(candidate).exists() || self.disabled_job_path(candidate).exists()
        })
    }

    pub(super) fn next_run_id(&self, job_id: &str) -> String {
        self.next_timestamped_id("jrun", |candidate| {
            self.run_id_exists_globally(job_id, candidate)
        })
    }

    pub(super) fn next_timestamped_id<F>(&self, prefix: &str, exists: F) -> String
    where
        F: Fn(&str) -> bool,
    {
        let base = format_timestamped_id(prefix, Utc::now());
        let state = id_generation_state();
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let start_suffix = if state.last_base == base {
            state.next_suffix
        } else {
            1
        };

        for suffix in start_suffix..1024_u32 {
            let candidate = if suffix == 1 {
                base.clone()
            } else {
                format!("{base}-{suffix}")
            };
            if !exists(&candidate) {
                state.last_base = base.clone();
                state.next_suffix = suffix + 1;
                return candidate;
            }
        }

        base
    }

    pub(super) fn run_id_exists_globally(&self, job_id: &str, run_id: &str) -> bool {
        self.run_bundle_dir(job_id, run_id).exists()
            || self.archived_run_bundle_dir(job_id, run_id).exists()
            || self.find_run_path(run_id).ok().flatten().is_some()
            || self.find_archived_run_path(run_id).ok().flatten().is_some()
    }
}

fn format_timestamped_id(prefix: &str, now: DateTime<Utc>) -> String {
    format!("{prefix}-{}", now.format("%Y%m%d-%H%M"))
}

fn id_generation_state() -> &'static Mutex<IdGenerationState> {
    static ID_GENERATION_STATE: OnceLock<Mutex<IdGenerationState>> = OnceLock::new();
    ID_GENERATION_STATE.get_or_init(|| Mutex::new(IdGenerationState::default()))
}
