// Migrated from file/skill_store.rs per ORB-00231
use tempfile::tempdir;

use super::super::*;

#[test]
fn layered_catalog_uses_merge_by_key_precedence() {
    let workspace = tempdir().expect("workspace tempdir");
    let global = tempdir().expect("global tempdir");

    write_skill(global.path(), "orbit", "global skill");
    write_skill(global.path(), "orbit-graph", "global graph");
    write_skill(workspace.path(), "orbit", "workspace override");

    let catalog =
        SkillCatalog::layered(workspace.path().to_path_buf(), global.path().to_path_buf());

    assert_eq!(catalog.strategy(), ScopeStrategy::MergeByKey);
    assert_eq!(
        catalog
            .load("orbit")
            .expect("load override")
            .sections
            .purpose,
        "workspace override"
    );
    assert_eq!(
        catalog
            .load("orbit-graph")
            .expect("load global fallback")
            .sections
            .purpose,
        "global graph"
    );

    let ids = catalog
        .list()
        .expect("list skills")
        .into_iter()
        .map(|skill| skill.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["orbit", "orbit-graph"]);
}

fn write_skill(root: &Path, id: &str, purpose: &str) {
    let dir = root.join(id);
    fs::create_dir_all(&dir).expect("create skill dir");
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: test skill\n---\n\n# Purpose\n\n{purpose}\n"),
    )
    .expect("write skill");
}
