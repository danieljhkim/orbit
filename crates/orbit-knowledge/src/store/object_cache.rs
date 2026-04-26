use std::fmt;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, MutexGuard};

use lru::LruCache;
use serde_json::Value;

pub const DEFAULT_OBJECT_CACHE_CAPACITY: usize = 10_000;
pub const DEFAULT_BLOB_CACHE_CAPACITY: usize = 2_000;

#[derive(Clone)]
pub struct GraphObjectCache {
    inner: Arc<GraphObjectCacheInner>,
}

struct GraphObjectCacheInner {
    objects: Mutex<LruCache<String, Value>>,
    blobs: Mutex<LruCache<String, String>>,
}

impl GraphObjectCache {
    pub fn new() -> Self {
        Self::with_capacities(
            NonZeroUsize::new(DEFAULT_OBJECT_CACHE_CAPACITY)
                .expect("default object cache capacity is non-zero"),
            NonZeroUsize::new(DEFAULT_BLOB_CACHE_CAPACITY)
                .expect("default blob cache capacity is non-zero"),
        )
    }

    pub fn with_capacities(object_capacity: NonZeroUsize, blob_capacity: NonZeroUsize) -> Self {
        Self {
            inner: Arc::new(GraphObjectCacheInner {
                objects: Mutex::new(LruCache::new(object_capacity)),
                blobs: Mutex::new(LruCache::new(blob_capacity)),
            }),
        }
    }

    pub fn object_capacity(&self) -> usize {
        self.objects().cap().get()
    }

    pub fn blob_capacity(&self) -> usize {
        self.blobs().cap().get()
    }

    pub fn object_len(&self) -> usize {
        self.objects().len()
    }

    pub fn blob_len(&self) -> usize {
        self.blobs().len()
    }

    pub(super) fn get_object(&self, object_hash: &str) -> Option<Value> {
        self.objects().get(object_hash).cloned()
    }

    pub(super) fn insert_object(&self, object_hash: String, value: Value) {
        self.objects().put(object_hash, value);
    }

    pub(super) fn get_blob(&self, blob_hash: &str) -> Option<String> {
        self.blobs().get(blob_hash).cloned()
    }

    pub(super) fn insert_blob(&self, blob_hash: String, source: String) {
        self.blobs().put(blob_hash, source);
    }

    fn objects(&self) -> MutexGuard<'_, LruCache<String, Value>> {
        self.inner
            .objects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn blobs(&self) -> MutexGuard<'_, LruCache<String, String>> {
        self.inner
            .blobs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for GraphObjectCache {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for GraphObjectCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphObjectCache")
            .field("object_capacity", &self.object_capacity())
            .field("object_len", &self.object_len())
            .field("blob_capacity", &self.blob_capacity())
            .field("blob_len", &self.blob_len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use serde_json::json;

    use super::GraphObjectCache;

    #[test]
    fn capacity_bounds_are_enforced_by_lru_cache() {
        let cache = GraphObjectCache::with_capacities(nonzero(2), nonzero(1));

        for index in 0..5 {
            cache.insert_object(format!("object-{index}"), json!({ "index": index }));
            assert!(cache.object_len() <= cache.object_capacity());
        }

        for index in 0..4 {
            cache.insert_blob(format!("blob-{index}"), format!("source {index}"));
            assert!(cache.blob_len() <= cache.blob_capacity());
        }

        assert_eq!(cache.object_len(), 2);
        assert_eq!(cache.blob_len(), 1);
    }

    fn nonzero(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).expect("test capacity is non-zero")
    }
}
