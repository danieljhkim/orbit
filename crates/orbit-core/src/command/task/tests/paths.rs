//! ORB-10475: `normalize_workspace_path` error coverage. The field is a
//! repository-relative/absolute filesystem path (never a logical/bridge
//! workspace id), and its errors must say so with a concrete example.

use orbit_common::types::OrbitError;
use tempfile::tempdir;

use crate::command::task::paths::normalize_workspace_path;

fn expect_invalid_input(result: Result<Option<String>, OrbitError>) -> String {
    match result {
        Err(OrbitError::InvalidInput(message)) => message,
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn no_workspace_input_resolves_to_none() {
    let repo_root = tempdir().expect("create repo root");

    let resolved =
        normalize_workspace_path(repo_root.path(), None).expect("missing workspace is optional");

    assert_eq!(resolved, None);
}

#[test]
fn repository_root_path_is_accepted() {
    let repo_root = tempdir().expect("create repo root");

    let resolved = normalize_workspace_path(repo_root.path(), Some("."))
        .expect("repo root path is a valid workspace");

    let canonical_repo_root = repo_root
        .path()
        .canonicalize()
        .expect("canonicalize repo root");
    assert_eq!(
        resolved,
        Some(canonical_repo_root.to_string_lossy().into_owned())
    );
}

#[test]
fn absolute_repository_path_is_accepted() {
    let repo_root = tempdir().expect("create repo root");
    let canonical_repo_root = repo_root
        .path()
        .canonicalize()
        .expect("canonicalize repo root");

    let resolved = normalize_workspace_path(
        repo_root.path(),
        Some(canonical_repo_root.to_str().unwrap()),
    )
    .expect("absolute repo path is a valid workspace");

    assert_eq!(
        resolved,
        Some(canonical_repo_root.to_string_lossy().into_owned())
    );
}

#[test]
fn logical_workspace_id_is_rejected_with_a_path_form_error() {
    let repo_root = tempdir().expect("create repo root");

    // `ws_orbit` reads like a bridge/broker logical workspace id; it is not a
    // directory under the repo root, so it must fail — but the failure must
    // explain that a *path* was expected, not merely that the directory is
    // missing.
    let message =
        expect_invalid_input(normalize_workspace_path(repo_root.path(), Some("ws_orbit")));

    assert!(
        message.contains("filesystem path"),
        "error must state the accepted form is a filesystem path: {message}"
    );
    assert!(
        message.contains("logical workspace id"),
        "error must call out that a logical/bridge id is not accepted: {message}"
    );
    assert!(
        !message.to_lowercase().contains("workspace id must"),
        "error must never describe a logical id as the required form: {message}"
    );
}

#[test]
fn out_of_repository_path_is_rejected_naming_the_repository() {
    let repo_root = tempdir().expect("create repo root");
    let outside = tempdir().expect("create outside dir");
    let canonical_outside = outside.path().canonicalize().expect("canonicalize outside");

    let message = expect_invalid_input(normalize_workspace_path(
        repo_root.path(),
        Some(canonical_outside.to_str().unwrap()),
    ));

    assert!(
        message.contains("inside repository"),
        "error must name that the path must stay inside the repository: {message}"
    );
    let canonical_repo_root = repo_root
        .path()
        .canonicalize()
        .expect("canonicalize repo root");
    assert!(
        message.contains(canonical_repo_root.to_string_lossy().as_ref()),
        "error must cite the repository root it must stay inside: {message}"
    );
}

#[test]
fn path_to_a_file_is_rejected_as_not_a_directory() {
    let repo_root = tempdir().expect("create repo root");
    let file_path = repo_root.path().join("not-a-dir.txt");
    std::fs::write(&file_path, b"content").expect("write file");

    let message = expect_invalid_input(normalize_workspace_path(
        repo_root.path(),
        Some("not-a-dir.txt"),
    ));

    assert!(
        message.contains("directory"),
        "error must state a directory is required: {message}"
    );
}
