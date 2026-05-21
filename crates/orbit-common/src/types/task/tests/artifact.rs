use super::super::TaskArtifact;

#[test]
fn artifact_from_source_defaults_to_file_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("summary.md");
    std::fs::write(&source, "hello\n").expect("write source");

    let artifact = TaskArtifact::from_source_file(&source, None).expect("read artifact source");

    assert_eq!(artifact.path, "summary.md");
    assert_eq!(artifact.text_content(), Some("hello\n"));
    assert_eq!(artifact.media_type, "text/markdown");
}

#[test]
fn artifact_from_source_uses_explicit_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("summary.md");
    std::fs::write(&source, "hello\n").expect("write source");

    let artifact = TaskArtifact::from_source_file(&source, Some("reports/summary.md"))
        .expect("read artifact source");

    assert_eq!(artifact.path, "reports/summary.md");
    assert_eq!(artifact.text_content(), Some("hello\n"));
}

#[test]
fn artifact_from_source_rejects_directories() {
    let dir = tempfile::tempdir().expect("tempdir");

    let error = TaskArtifact::from_source_file(dir.path(), None)
        .unwrap_err()
        .to_string();

    assert!(error.contains("must be a file"));
    assert!(error.contains(dir.path().to_string_lossy().as_ref()));
}

#[test]
fn artifact_from_source_accepts_binary() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("binary.bin");
    std::fs::write(&source, [0xff, 0xfe, 0xfd]).expect("write source");

    let artifact =
        TaskArtifact::from_source_file(&source, None).expect("read binary artifact source");

    assert_eq!(artifact.path, "binary.bin");
    assert_eq!(artifact.content, vec![0xff, 0xfe, 0xfd]);
    assert_eq!(artifact.media_type, "application/octet-stream");
}
