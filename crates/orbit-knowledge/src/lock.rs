//! Shared file-based lock store for graph node editing.
//!
//! Locks live at `.orbit/graph_locks.json` and are visible across all tasks/agents.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::KnowledgeError;

/// A lock held on a graph node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeLock {
    pub owner: String,
    pub task_id: Option<String>,
    pub locked_at: String,
    pub reason: String,
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
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &json)
            .map_err(|e| KnowledgeError::io(format!("write lock store tmp: {e}")))?;
        fs::rename(&tmp, path)
            .map_err(|e| KnowledgeError::io(format!("rename lock store: {e}")))?;
        Ok(())
    }

    /// Acquire a lock. Fails if locked by a different owner.
    pub fn lock(
        &mut self,
        selector: &str,
        owner: &str,
        task_id: Option<&str>,
        reason: &str,
    ) -> Result<(), LockError> {
        if let Some(existing) = self.locks.get(selector) {
            if existing.owner != owner {
                return Err(LockError::Contention {
                    selector: selector.to_string(),
                    held_by: existing.owner.clone(),
                    locked_at: existing.locked_at.clone(),
                });
            }
            // Already locked by same owner — idempotent
            return Ok(());
        }
        self.locks.insert(
            selector.to_string(),
            NodeLock {
                owner: owner.to_string(),
                task_id: task_id.map(str::to_string),
                locked_at: Utc::now().to_rfc3339(),
                reason: reason.to_string(),
            },
        );
        Ok(())
    }

    /// Release a lock. Only the owner can unlock.
    pub fn unlock(&mut self, selector: &str, owner: &str) -> Result<(), LockError> {
        if let Some(existing) = self.locks.get(selector)
            && existing.owner != owner
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
    pub fn check(&self, selector: &str, owner: &str) -> Result<(), LockError> {
        if let Some(existing) = self.locks.get(selector)
            && existing.owner != owner
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
    let mut store = LockStore::load(lock_path)?;
    let result = f(&mut store).map_err(|e| KnowledgeError::invalid_data(e.to_string()))?;
    store.save(lock_path)?;
    Ok(result)
}

/// Default lock file path within an orbit directory.
pub fn lock_store_path(orbit_dir: &Path) -> PathBuf {
    orbit_dir.join("knowledge/graph_locks.json")
}
