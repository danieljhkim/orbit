use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orbit_core::OrbitError;
use tokio::sync::Notify;

use super::super::{check_bindable_host, drain_with_grace_period};

#[test]
fn allows_ipv4_loopback() {
    let host = IpAddr::V4(Ipv4Addr::LOCALHOST);
    assert!(check_bindable_host(host, 7878).is_ok());
}

#[test]
fn allows_ipv6_loopback() {
    let host = IpAddr::V6(Ipv6Addr::LOCALHOST);
    assert!(check_bindable_host(host, 7878).is_ok());
}

#[test]
fn allows_127_0_0_x_range() {
    // The whole 127.0.0.0/8 block is loopback.
    let host = IpAddr::V4(Ipv4Addr::new(127, 5, 5, 5));
    assert!(check_bindable_host(host, 7878).is_ok());
}

#[test]
fn rejects_unspecified_address() {
    // `--host 0.0.0.0` is the exact exposure the guard exists to block.
    let host = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let err = check_bindable_host(host, 7878).expect_err("0.0.0.0 must be rejected");
    assert!(matches!(err, OrbitError::InvalidInput(_)));
}

#[test]
fn rejects_lan_address() {
    let host = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50));
    let err = check_bindable_host(host, 7878).expect_err("LAN address must be rejected");
    assert!(matches!(err, OrbitError::InvalidInput(_)));
}

// ORB-11255: `drain_with_grace_period` is the seam that lets the grace-period
// regression run in milliseconds instead of the real 10s `SHUTDOWN_GRACE_PERIOD`
// (see `crates/orbit-cli/tests/web_serve_shutdown.rs` for the real-server
// smoke test that exercises the same behavior end to end).

/// A never-completing "server" future, standing in for `axum::serve(..)`
/// while a shutdown signal never arrives.
fn pending_drain() -> impl std::future::Future<Output = std::io::Result<()>> {
    std::future::pending()
}

#[tokio::test]
async fn grace_period_does_not_start_before_shutdown_is_signaled() {
    let notify = Arc::new(Notify::new());
    let grace_period = Duration::from_millis(80);

    // Nobody ever calls `notify.notify_one()`, i.e. no shutdown was
    // requested. Waiting well past `grace_period` must not resolve the
    // future -- this is the exact PR1328 regression: a timeout that starts
    // counting down when serving begins, not when shutdown is requested,
    // would fire here and tear down a healthy server.
    let result = tokio::time::timeout(
        grace_period * 4,
        drain_with_grace_period(pending_drain(), notify, grace_period),
    )
    .await;
    assert!(
        result.is_err(),
        "drain_with_grace_period resolved without a shutdown signal ever firing"
    );
}

#[tokio::test]
async fn grace_period_starts_only_once_shutdown_is_signaled() {
    let notify = Arc::new(Notify::new());
    let grace_period = Duration::from_millis(80);
    let notify_for_signal = Arc::clone(&notify);

    let start = Instant::now();
    tokio::spawn(async move {
        // Simulate the delay between serving starting and a signal arriving:
        // if the grace-period clock started at process boot (the bug), this
        // delay would already have exhausted most or all of the deadline.
        tokio::time::sleep(grace_period * 2).await;
        notify_for_signal.notify_one();
    });

    let result = drain_with_grace_period(pending_drain(), notify, grace_period).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "expected the grace-period backstop to fire");
    assert!(
        elapsed >= grace_period * 2,
        "grace period elapsed before the shutdown signal fired: {elapsed:?}"
    );
    assert!(
        elapsed < grace_period * 4,
        "grace period took far longer than signal_delay + grace_period: {elapsed:?}"
    );
}

#[tokio::test]
async fn drain_completing_first_wins_even_after_shutdown_is_signaled() {
    let notify = Arc::new(Notify::new());
    notify.notify_one();

    let drain = async { Ok(()) };
    let result = drain_with_grace_period(drain, notify, Duration::from_secs(10)).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn early_signal_before_first_poll_is_not_a_lost_wakeup() {
    // Regression for a real race the reviewer flagged: in production, `drain`
    // is polled as part of the `select!` inside `drain_with_grace_period` and
    // can resolve the shutdown signal (calling the notify) before
    // `grace_elapsed`'s `notified().await` has ever been polled for the first
    // time. `Notify::notify_waiters` only wakes *already-registered* waiters
    // and would silently drop a notification that arrives this early,
    // leaving the grace timer never started -- an uncooperative drain would
    // then hang forever, reintroducing the exact ORB-11246 bug this whole
    // mechanism exists to prevent. `notify_one` stores a permit instead, so
    // this must resolve (via the grace timeout) rather than hang.
    let notify = Arc::new(Notify::new());
    notify.notify_one(); // fires before `drain_with_grace_period` even exists

    let grace_period = Duration::from_millis(80);
    let start = Instant::now();
    let result = tokio::time::timeout(
        grace_period * 4,
        drain_with_grace_period(pending_drain(), notify, grace_period),
    )
    .await;

    assert!(
        matches!(result, Ok(Ok(()))),
        "an early notification must not be a lost wakeup: {result:?}"
    );
    assert!(
        start.elapsed() < grace_period * 2,
        "grace timeout should fire close to immediately since the signal \
         already arrived before polling began: {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn drain_error_propagates_as_execution_error() {
    let notify = Arc::new(Notify::new());
    let drain = async { Err(std::io::Error::other("boom")) };
    let err = drain_with_grace_period(drain, notify, Duration::from_secs(10))
        .await
        .expect_err("serve error must propagate");
    assert!(matches!(err, OrbitError::Execution(msg) if msg.contains("boom")));
}
