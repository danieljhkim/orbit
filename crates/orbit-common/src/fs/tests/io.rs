use std::io;

use tempfile::TempDir;

use crate::OrbitError;
use crate::fs::io::with_exclusive_file_lock;

fn assert_sandbox_write_message(message: &str, path: &str) {
    assert!(
        message.contains(path),
        "expected path `{path}` in `{message}`"
    );
    assert!(
        message.contains("is not writable"),
        "expected writable attribution in `{message}`"
    );
    assert!(
        message.contains("sandbox or environment"),
        "expected sandbox/environment hint in `{message}`"
    );
    assert!(
        message.contains("not an Orbit store defect"),
        "expected store-defect negation in `{message}`"
    );
}

#[test]
fn exclusive_lock_runs_op_and_releases() {
    let temp = TempDir::new().expect("tempdir");
    let target = temp.path().join("task.yaml");
    let ran = with_exclusive_file_lock(&target, "task artifact v2", || {
        assert!(temp.path().join(".task.yaml.lock").exists());
        Ok::<_, io::Error>(true)
    })
    .expect("lock should succeed");
    assert!(ran);
}

#[test]
fn exclusive_lock_non_access_failure_stays_labeled() {
    let temp = TempDir::new().expect("tempdir");
    let blocker = temp.path().join("not-a-dir");
    std::fs::write(&blocker, b"x").expect("write blocker");
    let target = blocker.join("task.yaml");
    let err = with_exclusive_file_lock::<(), io::Error, _>(&target, "task artifact v2", || Ok(()))
        .expect_err("parent file must fail lock acquisition");
    let message = err.to_string();
    assert!(
        !message.contains("sandbox or environment"),
        "non-access failures must stay unlabeled: {message}"
    );
}

#[cfg(unix)]
#[test]
fn exclusive_lock_open_on_readonly_dir_names_path_and_hints_sandbox() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().expect("tempdir");
    let dir = temp.path().join("bundle");
    std::fs::create_dir(&dir).expect("mkdir");
    let target = dir.join("task.yaml");
    let lock_path = dir.join(".task.yaml.lock");

    let mut perms = std::fs::metadata(&dir).expect("meta").permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(&dir, perms).expect("chmod -w");

    struct Restore<'a>(&'a std::path::Path);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o755));
        }
    }
    let _restore = Restore(&dir);

    let err = with_exclusive_file_lock::<(), OrbitError, _>(&target, "task artifact v2", || Ok(()))
        .expect_err("lock open must fail on a read-only directory");
    match err {
        OrbitError::Io(message) => {
            assert_sandbox_write_message(&message, &lock_path.display().to_string());
            assert!(
                !message.contains("open task artifact v2 lock"),
                "classified access errors must not use the bare lock-open wrap: {message}"
            );
        }
        other => panic!("expected Io, got {other}"),
    }
}

/// ORB-10988: nesting the same lock path on one thread must re-enter, not
/// deadlock. `flock(2)` belongs to the open file description, so the inner
/// call's fresh descriptor would otherwise block on the outer call's lock
/// forever. The runtime relies on this to hold a task lock across a
/// read-modify-write whose inner store writes lock the same file.
#[test]
fn exclusive_lock_is_reentrant_within_a_thread() {
    let temp = TempDir::new().expect("tempdir");
    let target = temp.path().join("bundle").join("task.yaml");

    let depth = with_exclusive_file_lock::<usize, io::Error, _>(&target, "outer", || {
        with_exclusive_file_lock::<usize, io::Error, _>(&target, "inner", || {
            with_exclusive_file_lock::<usize, io::Error, _>(&target, "innermost", || Ok(3))
        })
    })
    .expect("nested locks must re-enter");

    assert_eq!(depth, 3);
}

/// Re-entry must not leak the held-path bookkeeping: once the outer call
/// returns — even by unwinding — the next acquisition has to lock for real, or
/// a later caller would silently run unlocked.
#[test]
fn exclusive_lock_releases_reentrancy_bookkeeping_after_a_panic() {
    let temp = TempDir::new().expect("tempdir");
    let target = temp.path().join("bundle").join("task.yaml");

    let panicked = std::panic::catch_unwind(|| {
        let _ = with_exclusive_file_lock::<(), io::Error, _>(&target, "outer", || {
            panic!("body blew up while holding the lock");
        });
    });
    assert!(panicked.is_err(), "the panic must propagate");

    // A different thread can only take the lock if the first release actually
    // happened, and this thread can only re-lock if its held set was cleared.
    let taken = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                with_exclusive_file_lock::<bool, io::Error, _>(&target, "other thread", || Ok(true))
            })
            .join()
            .expect("thread joined")
    })
    .expect("lock after panic");
    assert!(taken);
    with_exclusive_file_lock::<(), io::Error, _>(&target, "same thread again", || Ok(()))
        .expect("re-lock on the original thread");
}

/// Re-entrancy has to survive reaching the same lock file by a second route.
/// Orbit's checkout projection links a workspace path at the canonical task
/// bundle, so a nested lock arriving via the link must recognize the outer
/// lock taken via the canonical path rather than deadlock on it.
#[cfg(unix)]
#[test]
fn exclusive_lock_is_reentrant_across_a_symlinked_route() {
    let temp = TempDir::new().expect("tempdir");
    let canonical = temp.path().join("canonical");
    std::fs::create_dir(&canonical).expect("mkdir");
    let linked = temp.path().join("projection");
    std::os::unix::fs::symlink(&canonical, &linked).expect("symlink");

    let reached = with_exclusive_file_lock::<bool, io::Error, _>(
        &canonical.join("task.yaml"),
        "canonical route",
        || {
            with_exclusive_file_lock::<bool, io::Error, _>(
                &linked.join("task.yaml"),
                "projected route",
                || Ok(true),
            )
        },
    )
    .expect("the projected route must re-enter the canonical lock");
    assert!(reached);
}
