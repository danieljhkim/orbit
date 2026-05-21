//! Shared file-based lock store for graph node editing.
//!
//! Locks live at `.orbit/knowledge/graph_locks.json` and are visible across all
//! tasks/agents.

use std::collections::{HashMap, hash_map::Entry};
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::KnowledgeError;
use crate::io::write_text_atomic_durable;

static SELECTOR_LOCKS: OnceLock<Mutex<HashMap<PathBuf, HeldSelectorLock>>> = OnceLock::new();

/// A lock held on a graph node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeLock {
    pub owner: String,
    pub task_id: Option<String>,
    pub locked_at: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
}

/// Shared lock store backed by a JSON file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LockStore {
    /// selector string → lock
    pub locks: HashMap<String, NodeLock>,
}

impl LockStore {
    /// Load the lock store from disk. Returns empty store if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self, KnowledgeError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .map_err(|e| KnowledgeError::io(format!("read lock store: {e}")))?;
        serde_json::from_str(&raw)
            .map_err(|e| KnowledgeError::invalid_data(format!("parse lock store: {e}")))
    }

    /// Persist the lock store to disk atomically (write tmp + rename).
    pub fn save(&self, path: &Path) -> Result<(), KnowledgeError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| KnowledgeError::io(format!("mkdir for lock store: {e}")))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| KnowledgeError::invalid_data(format!("serialize lock store: {e}")))?;
        write_text_atomic_durable(path, &format!("{json}\n"))
            .map_err(|e| KnowledgeError::io(format!("write lock store: {e}")))?;
        Ok(())
    }

    /// Acquire a lock. Fails if locked by a different owner.
    pub fn lock(
        &mut self,
        selector: &str,
        owner: &str,
        task_id: Option<&str>,
        reason: &str,
        owner_pid: u32,
    ) -> Result<(), LockError> {
        if let Some(existing) = self.locks.get(selector)
            && existing.owner == owner
            && existing.owner_pid == Some(owner_pid)
        {
            return Ok(());
        }
        self.locks.insert(
            selector.to_string(),
            NodeLock {
                owner: owner.to_string(),
                task_id: task_id.map(str::to_string),
                locked_at: Utc::now().to_rfc3339(),
                reason: reason.to_string(),
                owner_pid: Some(owner_pid),
            },
        );
        Ok(())
    }

    /// Release a lock. Only the owner can unlock.
    pub fn unlock(&mut self, selector: &str, owner: &str, owner_pid: u32) -> Result<(), LockError> {
        if let Some(existing) = self.locks.get(selector)
            && (existing.owner != owner || existing.owner_pid != Some(owner_pid))
        {
            return Err(LockError::Contention {
                selector: selector.to_string(),
                held_by: existing.owner.clone(),
                locked_at: existing.locked_at.clone(),
            });
        }
        self.locks.remove(selector);
        Ok(())
    }

    /// Check if a selector is locked by someone other than `owner`.
    pub fn check(&self, selector: &str, owner: &str, owner_pid: u32) -> Result<(), LockError> {
        if let Some(existing) = self.locks.get(selector)
            && (existing.owner != owner || existing.owner_pid != Some(owner_pid))
        {
            return Err(LockError::Contention {
                selector: selector.to_string(),
                held_by: existing.owner.clone(),
                locked_at: existing.locked_at.clone(),
            });
        }
        Ok(())
    }

    /// Get the lock for a selector, if any.
    pub fn get(&self, selector: &str) -> Option<&NodeLock> {
        self.locks.get(selector)
    }
}

/// Held graph lock selectors that are released on drop, including unwind.
pub struct GraphLockGuard {
    knowledge_dir: PathBuf,
    owner: String,
    owner_pid: u32,
    selectors: Vec<String>,
    released: bool,
}

impl GraphLockGuard {
    pub fn acquire(
        knowledge_dir: &Path,
        owner: &str,
        task_id: Option<&str>,
        reason: &str,
        selectors: &[String],
    ) -> Result<Self, KnowledgeError> {
        let lock_path = lock_store_path(knowledge_dir);
        let owner_pid = std::process::id();
        let _sentinel = lock_store_sentinel(&lock_path)?;
        let mut store = LockStore::load(&lock_path)?;
        let mut acquired_selectors: Vec<String> = Vec::new();
        for selector in selectors {
            if let Err(error) = acquire_selector_lock(knowledge_dir, selector, owner, &store) {
                for acquired_selector in &acquired_selectors {
                    let _ = release_selector_lock(knowledge_dir, acquired_selector, owner);
                }
                return Err(error);
            }
            acquired_selectors.push(selector.clone());
        }
        let save_result: Result<(), LockError> = (|| {
            for selector in selectors {
                store.lock(selector, owner, task_id, reason, owner_pid)?;
            }
            Ok(())
        })();
        if let Err(error) = save_result {
            for selector in selectors {
                let _ = release_selector_lock(knowledge_dir, selector, owner);
            }
            return Err(KnowledgeError::invalid_data(error.to_string()));
        }
        if let Err(error) = store.save(&lock_path) {
            for selector in selectors {
                let _ = release_selector_lock(knowledge_dir, selector, owner);
            }
            return Err(error);
        }

        Ok(Self {
            knowledge_dir: knowledge_dir.to_path_buf(),
            owner: owner.to_string(),
            owner_pid,
            selectors: selectors.to_vec(),
            released: false,
        })
    }

    pub fn release(&mut self) -> Result<(), KnowledgeError> {
        if self.released {
            return Ok(());
        }

        let lock_path = lock_store_path(&self.knowledge_dir);
        let _sentinel = lock_store_sentinel(&lock_path)?;
        let mut store = LockStore::load(&lock_path)?;
        for selector in &self.selectors {
            let fully_released = release_selector_lock(&self.knowledge_dir, selector, &self.owner)?;
            if fully_released {
                store
                    .unlock(selector, &self.owner, self.owner_pid)
                    .map_err(|error| KnowledgeError::invalid_data(error.to_string()))?;
            }
        }
        store.save(&lock_path)?;
        self.released = true;
        Ok(())
    }
}

impl Drop for GraphLockGuard {
    fn drop(&mut self) {
        if self.released {
            return;
        }

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Err(error) = self.release() {
                tracing::warn!(
                    target: "orbit.knowledge.lock",
                    owner = self.owner.as_str(),
                    error = %error,
                    "failed to release graph locks",
                );
            }
        }));
    }
}

/// Error returned when a lock operation fails due to contention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum LockError {
    Contention {
        selector: String,
        held_by: String,
        locked_at: String,
    },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Contention {
                selector,
                held_by,
                locked_at,
            } => write!(f, "`{selector}` is locked by `{held_by}` since {locked_at}"),
        }
    }
}

/// Convenience: load → operate → save in one call.
pub fn with_lock_store<F, T>(lock_path: &Path, f: F) -> Result<T, KnowledgeError>
where
    F: FnOnce(&mut LockStore) -> Result<T, LockError>,
{
    let _sentinel = lock_store_sentinel(lock_path)?;
    let mut store = LockStore::load(lock_path)?;
    let result = f(&mut store).map_err(|e| KnowledgeError::invalid_data(e.to_string()))?;
    store.save(lock_path)?;
    Ok(result)
}

/// Default lock file path within the shared knowledge directory.
pub fn lock_store_path(knowledge_dir: &Path) -> PathBuf {
    knowledge_dir.join("graph_locks.json")
}

fn selector_lock_path(knowledge_dir: &Path, selector: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(selector.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    knowledge_dir
        .join(".graph_lock_leases")
        .join(format!("{digest}.lock"))
}

fn lock_store_sentinel(lock_path: &Path) -> Result<File, KnowledgeError> {
    let sentinel_path = lock_path.with_extension("json.lock");
    if let Some(parent) = sentinel_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| KnowledgeError::io(format!("mkdir for lock sentinel: {error}")))?;
    }

    let sentinel = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&sentinel_path)
        .map_err(|error| KnowledgeError::io(format!("open lock sentinel: {error}")))?;
    sentinel
        .lock_exclusive()
        .map_err(|error| KnowledgeError::io(format!("lock sentinel: {error}")))?;
    Ok(sentinel)
}

#[derive(Debug)]
struct HeldSelectorLock {
    owner: String,
    ref_count: usize,
    _file: File,
}

fn selector_lock_registry() -> &'static Mutex<HashMap<PathBuf, HeldSelectorLock>> {
    SELECTOR_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn acquire_selector_lock(
    knowledge_dir: &Path,
    selector: &str,
    owner: &str,
    store: &LockStore,
) -> Result<(), KnowledgeError> {
    let path = selector_lock_path(knowledge_dir, selector);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| KnowledgeError::io(format!("mkdir for selector lease: {error}")))?;
    }

    let mut registry = selector_lock_registry().lock().map_err(|error| {
        KnowledgeError::io(format!("selector lease registry poisoned: {error}"))
    })?;
    match registry.entry(path.clone()) {
        Entry::Occupied(mut entry) => {
            if entry.get().owner != owner {
                return Err(KnowledgeError::invalid_data(
                    contention_error(selector, store).to_string(),
                ));
            }
            entry.get_mut().ref_count += 1;
            Ok(())
        }
        Entry::Vacant(entry) => {
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&path)
                .map_err(|error| KnowledgeError::io(format!("open selector lease: {error}")))?;
            match file.try_lock_exclusive() {
                Ok(()) => {
                    entry.insert(HeldSelectorLock {
                        owner: owner.to_string(),
                        ref_count: 1,
                        _file: file,
                    });
                    Ok(())
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => Err(
                    KnowledgeError::invalid_data(contention_error(selector, store).to_string()),
                ),
                Err(error) => Err(KnowledgeError::io(format!(
                    "lock selector lease '{}': {error}",
                    path.display()
                ))),
            }
        }
    }
}

fn release_selector_lock(
    knowledge_dir: &Path,
    selector: &str,
    owner: &str,
) -> Result<bool, KnowledgeError> {
    let path = selector_lock_path(knowledge_dir, selector);
    let mut registry = selector_lock_registry().lock().map_err(|error| {
        KnowledgeError::io(format!("selector lease registry poisoned: {error}"))
    })?;
    let Entry::Occupied(mut entry) = registry.entry(path) else {
        return Ok(true);
    };

    if entry.get().owner != owner {
        return Err(KnowledgeError::invalid_data(format!(
            "selector lease owner mismatch for `{selector}`: held by `{}`",
            entry.get().owner
        )));
    }

    if entry.get().ref_count > 1 {
        entry.get_mut().ref_count -= 1;
        return Ok(false);
    }

    entry.remove();
    Ok(true)
}

fn contention_error(selector: &str, store: &LockStore) -> LockError {
    let held_by = store
        .get(selector)
        .map(|lock| lock.owner.clone())
        .unwrap_or_else(|| "another active process".to_string());
    let locked_at = store
        .get(selector)
        .map(|lock| lock.locked_at.clone())
        .unwrap_or_else(|| "unknown".to_string());
    LockError::Contention {
        selector: selector.to_string(),
        held_by,
        locked_at,
    }
}

#[cfg(test)]
mod tests;
