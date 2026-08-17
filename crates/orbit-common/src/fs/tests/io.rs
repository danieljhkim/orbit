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
