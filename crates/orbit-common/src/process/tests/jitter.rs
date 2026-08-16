//! Unit tests for `utility::jitter` — sibling layout under tests/.

use crate::process::jitter::JitterRng;

#[test]
fn full_jitter_stays_within_inclusive_bound() {
    let mut rng = JitterRng::from_seed(42);
    for bound in [0u64, 1, 2, 100, 1_000, 60_000, u64::MAX] {
        for _ in 0..256 {
            let sample = rng.full_jitter(bound);
            assert!(sample <= bound, "sample {sample} exceeded bound {bound}");
        }
    }
}

#[test]
fn full_jitter_zero_bound_is_always_zero() {
    let mut rng = JitterRng::from_seed(7);
    for _ in 0..64 {
        assert_eq!(rng.full_jitter(0), 0);
    }
}

#[test]
fn full_jitter_is_not_degenerate() {
    // Over many draws against a wide bound, at least two distinct values
    // must appear — guards against a broken generator returning a constant.
    let mut rng = JitterRng::from_seed(1234);
    let mut values = std::collections::HashSet::new();
    for _ in 0..64 {
        values.insert(rng.full_jitter(1_000_000));
    }
    assert!(values.len() > 1, "jitter draws were constant: {values:?}");
}

#[test]
fn from_seed_is_deterministic() {
    let mut a = JitterRng::from_seed(99);
    let mut b = JitterRng::from_seed(99);
    for _ in 0..16 {
        assert_eq!(a.next_u64(), b.next_u64());
    }
}

#[test]
fn zero_seed_does_not_stick_at_zero() {
    // xorshift64* has a fixed point at state 0; the constructor must dodge it.
    let mut rng = JitterRng::from_seed(0);
    let draws: Vec<u64> = (0..8).map(|_| rng.next_u64()).collect();
    assert!(
        draws.iter().any(|v| *v != 0),
        "zero seed produced all zeros"
    );
}

#[test]
fn seeded_salts_produce_distinct_streams() {
    // Two runs with different salts should (overwhelmingly) diverge. Time
    // and pid feed the seed too, so equality is possible in theory but a
    // shared 64-draw prefix indicates the salt is being ignored.
    let mut a = JitterRng::seeded("run-a");
    let mut b = JitterRng::seeded("run-b");
    let diverged = (0..64).any(|_| a.next_u64() != b.next_u64());
    assert!(diverged, "distinct salts produced identical streams");
}
