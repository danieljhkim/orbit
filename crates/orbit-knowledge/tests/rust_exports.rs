use std::fs;
use std::path::Path;
use std::process::Command;

use orbit_knowledge::graph::object_store::RefName;
use orbit_knowledge::pipeline::context::BuildConfig;
use orbit_knowledge::pipeline::run_build;
use orbit_knowledge::{KnowledgeStore, Selector};
use tempfile::TempDir;

#[test]
fn indexes_public_rust_reexport_chain_into_file_exports() -> Result<(), Box<dyn std::error::Error>>
{
    let repo = TempDir::new()?;
    init_repo(repo.path())?;
    write_file(repo.path(), "a/src/lib.rs", "pub struct Foo;\n")?;
    write_file(repo.path(), "b/src/lib.rs", "pub use a::Foo;\n")?;
    write_file(repo.path(), "c/src/lib.rs", "pub use b::Foo;\n")?;
    commit_all(repo.path(), "seed re-export chain")?;

    let knowledge_dir = TempDir::new()?;
    let build_ref = RefName::new("main")?;
    let ctx = run_build(BuildConfig {
        repo_path: repo.path().to_path_buf(),
        output_dir: knowledge_dir.path().to_path_buf(),
        incremental: false,
        ref_name: Some(build_ref.clone()),
    })?;

    for path in ["a/src/lib.rs", "b/src/lib.rs", "c/src/lib.rs"] {
        let file = ctx
            .graph
            .files
            .iter()
            .find(|file| file.base.location == path)
            .unwrap_or_else(|| panic!("missing graph file {path}"));
        assert!(
            file.exports.contains(&"Foo".to_string()),
            "expected {path} exports to include Foo, got {:?}",
            file.exports
        );
    }

    let store = KnowledgeStore::open(knowledge_dir.path(), &build_ref, None, None)?;
    for path in ["a/src/lib.rs", "b/src/lib.rs", "c/src/lib.rs"] {
        let selector: Selector = format!("file:{path}").parse()?;
        let pack = store.pack(&[selector])?;
        let exports = pack.entries[0]
            .exports
            .as_ref()
            .unwrap_or_else(|| panic!("missing pack exports for {path}"));
        assert!(
            exports.contains(&"Foo".to_string()),
            "expected pack exports for {path} to include Foo, got {exports:?}"
        );
    }

    Ok(())
}

fn init_repo(repo: &Path) -> Result<(), Box<dyn std::error::Error>> {
    git(repo, &["init", "-q", "--initial-branch=main"])?;
    git(repo, &["config", "user.name", "Orbit Tests"])?;
    git(repo, &["config", "user.email", "orbit-tests@example.com"])?;
    git(repo, &["config", "commit.gpgsign", "false"])?;
    Ok(())
}

fn write_file(repo: &Path, rel: &str, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn commit_all(repo: &Path, message: &str) -> Result<(), Box<dyn std::error::Error>> {
    git(repo, &["add", "-A"])?;
    git(repo, &["commit", "-q", "-m", message])?;
    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("git").args(args).current_dir(repo).status()?;
    assert!(
        status.success(),
        "git {:?} failed in {}",
        args,
        repo.display()
    );
    Ok(())
}
