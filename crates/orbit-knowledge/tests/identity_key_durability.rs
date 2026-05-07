use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use orbit_knowledge::graph::nodes::LeafNode;
use orbit_knowledge::graph::object_store::RefName;
use orbit_knowledge::pipeline;
use orbit_knowledge::pipeline::context::{BuildConfig, PipelineContext};
use tempfile::TempDir;

#[test]
fn identity_key_survives_rename_rebuild() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = IdentityFixture::new()?;
    fixture.write_file("a.rs", "pub fn foo() -> u32 {\n    1\n}\n")?;
    fixture.commit_all("seed rename fixture")?;

    let before = fixture.build(false)?;
    let before_key = identity_key_for(&before, "foo");

    fixture.rename_file("a.rs", "b.rs")?;
    let after = fixture.build(true)?;
    let after_key = identity_key_for(&after, "foo");

    assert_eq!(
        after_key, before_key,
        "rename invariant violated: identity_key for foo changed after a.rs was renamed to b.rs"
    );
    Ok(())
}

#[test]
fn identity_key_survives_move_rebuild() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = IdentityFixture::new()?;
    fixture.write_file(
        "src/a.rs",
        "pub fn foo() -> u32 {\n    1\n}\n\npub fn bar() -> u32 {\n    2\n}\n",
    )?;
    fixture.commit_all("seed move fixture")?;

    let before = fixture.build(false)?;
    let before_keys = identity_keys_by_name(&before);

    fixture.rename_file("src/a.rs", "src/sub/a.rs")?;
    let after = fixture.build(true)?;
    let after_keys = identity_keys_by_name(&after);

    assert_eq!(
        after_keys, before_keys,
        "move invariant violated: identity_key values changed after src/a.rs was moved to src/sub/a.rs"
    );
    Ok(())
}

#[test]
fn identity_key_survives_content_edit_rebuild() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = IdentityFixture::new()?;
    fixture.write_file(
        "a.rs",
        "pub fn foo() -> u32 {\n    1\n}\n\npub fn bar() -> u32 {\n    2\n}\n",
    )?;
    fixture.commit_all("seed content edit fixture")?;

    let before = fixture.build(false)?;
    let before_keys = identity_keys_by_name(&before);

    fixture.write_file(
        "a.rs",
        "pub fn foo() -> u32 {\n    40 + 2\n}\n\npub fn bar() -> u32 {\n    2\n}\n",
    )?;
    let after = fixture.build(true)?;
    let after_keys = identity_keys_by_name(&after);

    assert_eq!(
        after_keys.get("foo"),
        before_keys.get("foo"),
        "content_edit invariant violated: identity_key for foo changed after editing its body without changing its signature"
    );
    assert_eq!(
        after_keys.get("bar"),
        before_keys.get("bar"),
        "content_edit invariant violated: identity_key for bar changed when only foo's body was edited"
    );
    Ok(())
}

#[test]
fn identity_key_survives_delete_recreate_rebuild() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = IdentityFixture::new()?;
    let source = "pub fn foo() -> u32 {\n    1\n}\n";
    fixture.write_file("a.rs", source)?;
    fixture.commit_all("seed delete recreate fixture")?;

    let before = fixture.build(false)?;
    let before_key = identity_key_for(&before, "foo");

    fixture.remove_file("a.rs")?;
    let deleted = fixture.build(true)?;
    assert!(
        deleted
            .graph
            .leaves
            .iter()
            .all(|leaf| leaf.base.name != "foo"),
        "delete_recreate invariant setup failed: foo still existed after deleting a.rs and rebuilding"
    );

    fixture.write_file("a.rs", source)?;
    let after = fixture.build(true)?;
    let after_key = identity_key_for(&after, "foo");

    assert_eq!(
        after_key, before_key,
        "delete_recreate invariant violated: identity_key for foo changed after deleting and recreating a.rs with the same content"
    );
    Ok(())
}

#[test]
fn identity_key_survives_signature_change_rebuild() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = IdentityFixture::new()?;
    fixture.write_file("a.rs", "pub fn foo(x: u32) -> u32 {\n    x + 1\n}\n")?;
    fixture.commit_all("seed signature change fixture")?;

    let before = fixture.build(false)?;
    let before_key = identity_key_for(&before, "foo");

    fixture.write_file("a.rs", "pub fn foo(x: u64) -> u64 {\n    x + 1\n}\n")?;
    let after = fixture.build(true)?;
    let after_key = identity_key_for(&after, "foo");

    assert_eq!(
        after_key, before_key,
        "signature_change current-behavior invariant violated: identity_key for foo changed after changing its Rust function signature"
    );
    Ok(())
}

struct IdentityFixture {
    repo: TempDir,
    knowledge: TempDir,
    ref_name: RefName,
}

impl IdentityFixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let repo = TempDir::new()?;
        git(repo.path(), &["init", "-q", "--initial-branch=main"])?;
        git(repo.path(), &["config", "user.name", "Orbit Tests"])?;
        git(
            repo.path(),
            &["config", "user.email", "orbit-tests@example.com"],
        )?;
        git(repo.path(), &["config", "commit.gpgsign", "false"])?;
        fs::write(repo.path().join("README.md"), "identity fixture\n")?;

        Ok(Self {
            repo,
            knowledge: TempDir::new()?,
            ref_name: RefName::new("main")?,
        })
    }

    fn build(&self, incremental: bool) -> Result<PipelineContext, Box<dyn std::error::Error>> {
        Ok(pipeline::run_build(BuildConfig {
            repo_path: self.repo.path().to_path_buf(),
            output_dir: self.knowledge.path().to_path_buf(),
            incremental,
            ref_name: Some(self.ref_name.clone()),
        })?)
    }

    fn write_file(&self, rel: &str, content: &str) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.repo.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    fn rename_file(&self, from: &str, to: &str) -> Result<(), Box<dyn std::error::Error>> {
        let to_path = self.repo.path().join(to);
        if let Some(parent) = to_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(self.repo.path().join(from), to_path)?;
        Ok(())
    }

    fn remove_file(&self, rel: &str) -> Result<(), Box<dyn std::error::Error>> {
        fs::remove_file(self.repo.path().join(rel))?;
        Ok(())
    }

    fn commit_all(&self, message: &str) -> Result<(), Box<dyn std::error::Error>> {
        git(self.repo.path(), &["add", "-A"])?;
        git(self.repo.path(), &["commit", "-q", "-m", message])?;
        Ok(())
    }
}

fn identity_key_for(ctx: &PipelineContext, name: &str) -> String {
    leaf_by_name(ctx, name).base.identity_key.clone()
}

fn identity_keys_by_name(ctx: &PipelineContext) -> BTreeMap<String, String> {
    ctx.graph
        .leaves
        .iter()
        .map(|leaf| (leaf.base.name.clone(), leaf.base.identity_key.clone()))
        .collect()
}

fn leaf_by_name<'a>(ctx: &'a PipelineContext, name: &str) -> &'a LeafNode {
    ctx.graph
        .leaves
        .iter()
        .find(|leaf| leaf.base.name == name)
        .unwrap_or_else(|| {
            let names = ctx
                .graph
                .leaves
                .iter()
                .map(|leaf| leaf.base.name.as_str())
                .collect::<Vec<_>>();
            panic!("missing leaf `{name}`; graph leaves were {names:?}")
        })
}

fn git(repo: &Path, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git").args(args).current_dir(repo).output()?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed in '{}': {}",
            args.join(" "),
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(())
}
