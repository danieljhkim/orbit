use tempfile::tempdir;

use super::super::test_support::{ENV_LOCK, EnvVarGuard};
use super::*;

#[test]
fn resolve_workspace_layout_skips_global_home_orbit_during_walk_up() {
    let _lock = ENV_LOCK.lock().expect("lock env");
    let home = tempdir().expect("home tempdir");
    let global_orbit = home.path().join(".orbit");
    std::fs::create_dir_all(&global_orbit).expect("seed global orbit");
    let nested = home.path().join("uninitialized-project");
    std::fs::create_dir_all(&nested).expect("create nested cwd");
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str().to_os_string());

    let err = resolve_workspace_layout_for_cwd(&nested)
        .expect_err("walk-up to $HOME/.orbit should fail");

    assert!(matches!(
        err,
        OrbitError::InvalidInput(message)
            if message.contains("not inside an initialized Orbit workspace")
    ));
}
