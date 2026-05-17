//! Search index / reindex / latency tests split per ORB-00116.

use std::time::Instant;

use orbit_common::types::LearningScope;

use super::store::LearningFileStore;
use super::test_support::{create_params, store_with_index};
use crate::backend::{LearningCreateParams, LearningSearchParams};

#[test]
#[ignore]
fn learning_search_latency_p50_under_10ms_at_500_records() {
    let (_dir, store) = store_with_index();

    // Seed 500 active learnings with varied scopes.
    let path_pool = [
        "crates/orbit-engine/**/perf*.rs",
        "crates/orbit-knowledge/**/*.rs",
        "crates/orbit-tools/**/handlers/*.rs",
        "benchmarks/**/*.rs",
        "docs/**/*.md",
    ];
    let tag_pool = ["performance", "knowledge", "tools", "bench", "docs"];

    for i in 0..500 {
        let path = path_pool[i % path_pool.len()].to_string();
        let tag = tag_pool[i % tag_pool.len()].to_string();
        store
            .create_learning(LearningCreateParams {
                summary: format!("Learning {i}"),
                scope: LearningScope {
                    paths: vec![path],
                    tags: vec![tag],
                    ..Default::default()
                },
                body: String::new(),
                evidence: Vec::new(),
                created_by: Some("bench".to_string()),
                priority: None,
            })
            .expect("seed");
    }

    // 1000 search calls against a representative path.
    let mut durations_ns: Vec<u128> = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let start = Instant::now();
        let _ = store
            .search_learnings(LearningSearchParams {
                path: Some("crates/orbit-engine/perf_runner.rs".to_string()),
                limit: Some(5),
                ..Default::default()
            })
            .expect("search");
        durations_ns.push(start.elapsed().as_nanos());
    }
    durations_ns.sort_unstable();
    let p = |q: f64| -> u128 {
        let idx = ((durations_ns.len() as f64) * q).floor() as usize;
        durations_ns[idx.min(durations_ns.len() - 1)]
    };
    let p50_ns = p(0.50);
    let p95_ns = p(0.95);
    let p99_ns = p(0.99);
    let p50_ms = (p50_ns as f64) / 1_000_000.0;
    let p95_ms = (p95_ns as f64) / 1_000_000.0;
    let p99_ms = (p99_ns as f64) / 1_000_000.0;
    // Print methodology + raw numbers to stdout. This bench is gated as
    // `#[ignore]` so default `cargo test` skips it; run explicitly with
    // `cargo test -p orbit-store --release --lib learning_search_latency
    // -- --ignored --nocapture`.
    #[allow(clippy::print_stdout)]
    {
        println!(
            "learning_search_latency: 500 records, 1000 calls, target path=crates/orbit-engine/perf_runner.rs"
        );
        println!("learning_search_latency: p50={p50_ms:.3}ms p95={p95_ms:.3}ms p99={p99_ms:.3}ms");
    }
    assert!(
        p50_ms < 10.0,
        "median search latency must be < 10ms; got {p50_ms:.3}ms (p95={p95_ms:.3}ms p99={p99_ms:.3}ms)"
    );
}
