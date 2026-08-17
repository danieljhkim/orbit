use std::io;
use std::path::Path;

use super::OrbitError;

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
fn from_write_io_names_path_for_readonly_filesystem_kind() {
    let path = Path::new("/tmp/.orbit/tasks/ORB-00000/plan.md");
    let error = OrbitError::from_write_io(
        path,
        io::Error::new(io::ErrorKind::ReadOnlyFilesystem, "Read-only file system"),
    );
    match error {
        OrbitError::Io(message) => {
            assert_sandbox_write_message(&message, &path.display().to_string());
            assert!(message.contains("Read-only file system"), "{message}");
        }
        other => panic!("expected Io, got {other}"),
    }
}

#[test]
fn from_write_io_names_path_for_permission_denied() {
    let path = Path::new("/readonly/.orbit/frictions/tags.yaml");
    let error = OrbitError::from_write_io(
        path,
        io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"),
    );
    match error {
        OrbitError::Io(message) => {
            assert_sandbox_write_message(&message, &path.display().to_string())
        }
        other => panic!("expected Io, got {other}"),
    }
}

#[cfg(unix)]
#[test]
fn from_write_io_classifies_raw_erofs_and_eacces() {
    let path = Path::new("/mnt/ro/.orbit/state/tasks/ORB-00000/envelope.yaml");
    for errno in [libc::EROFS, libc::EACCES] {
        let error = OrbitError::from_write_io(path, io::Error::from_raw_os_error(errno));
        match error {
            OrbitError::Io(message) => {
                assert_sandbox_write_message(&message, &path.display().to_string())
            }
            other => panic!("expected Io for errno {errno}, got {other}"),
        }
    }
}

#[test]
fn from_write_io_leaves_other_io_failures_bare() {
    let path = Path::new("/tmp/.orbit/tasks/ORB-00000/plan.md");
    let error = OrbitError::from_write_io(
        path,
        io::Error::new(io::ErrorKind::StorageFull, "no space left"),
    );
    match error {
        OrbitError::Io(message) => {
            assert_eq!(message, "no space left");
            assert!(!message.contains("sandbox or environment"), "{message}");
        }
        other => panic!("expected Io, got {other}"),
    }
}
