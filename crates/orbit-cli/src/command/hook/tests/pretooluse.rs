use super::*;
use std::path::Path;
use std::path::PathBuf;

#[test]
fn tmpdir_falls_back_to_tmp_when_empty() {
    let path = orbit_core::command::learning_hook::state_file_path(
        Path::new("/repo"),
        None,
        Path::new("/tmp"),
        42,
    );
    assert_eq!(path, PathBuf::from("/tmp/orbit-learning-hook-42.json"));
}

#[test]
fn cap_env_constants_match_documented_names() {
    assert_eq!(
        orbit_core::command::learning_hook::ORBIT_LEARNING_PER_CALL_CAP_ENV,
        "ORBIT_LEARNING_PER_CALL_CAP"
    );
    assert_eq!(
        orbit_core::command::learning_hook::ORBIT_LEARNING_SESSION_CAP_ENV,
        "ORBIT_LEARNING_SESSION_CAP"
    );
}
