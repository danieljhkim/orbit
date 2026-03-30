use std::path::Path;
use std::process::Command;

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::RuntimeHost;

use super::input::input_string_field;

pub(super) fn verify_batch<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let workspace_path = match input_string_field(input, "workspace_path") {
        Some(p) => p,
        None => {
            let repo_root_str = host.repo_root()?;
            let repo_root = Path::new(&repo_root_str);
            let worktree = super::parallel::resolve_shared_worktree_path(repo_root)?;
            worktree.to_string_lossy().to_string()
        }
    };

    let workspace = super::input::canonicalize_existing_dir(&workspace_path, "workspace_path")?;

    // Run cargo build
    let build_output = Command::new("cargo")
        .arg("build")
        .current_dir(&workspace)
        .output()
        .map_err(|e| OrbitError::Execution(format!("failed to spawn cargo build: {e}")))?;

    let build_stdout = String::from_utf8_lossy(&build_output.stdout).to_string();
    let build_stderr = String::from_utf8_lossy(&build_output.stderr).to_string();

    if !build_output.status.success() {
        let exit_code = build_output.status.code().unwrap_or(1);
        return Err(OrbitError::Execution(format!(
            "verify_batch: cargo build failed (exit_code={exit_code})\nstdout:\n{build_stdout}\nstderr:\n{build_stderr}"
        )));
    }

    // Run cargo test
    let test_output = Command::new("cargo")
        .arg("test")
        .current_dir(&workspace)
        .output()
        .map_err(|e| OrbitError::Execution(format!("failed to spawn cargo test: {e}")))?;

    let test_stdout = String::from_utf8_lossy(&test_output.stdout).to_string();
    let test_stderr = String::from_utf8_lossy(&test_output.stderr).to_string();
    let test_exit_code = test_output.status.code().unwrap_or(1);

    let stdout = format!("{build_stdout}{test_stdout}");
    let stderr = format!("{build_stderr}{test_stderr}");

    if !test_output.status.success() {
        return Err(OrbitError::Execution(format!(
            "verify_batch: cargo test failed (exit_code={test_exit_code})\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )));
    }

    Ok(json!({
        "passed": true,
        "exit_code": test_exit_code,
        "stdout": stdout,
        "stderr": stderr,
    }))
}
