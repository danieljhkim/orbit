#![allow(missing_docs)]

use super::super::*;

use std::num::NonZeroUsize;

use serde_json::json;

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
