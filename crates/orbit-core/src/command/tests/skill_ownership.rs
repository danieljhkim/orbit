//! Ownership + retirement metadata tests, covering the classification matrix
//! from ORB-10181 against the *whole generated skill tree*: current generated
//! skills (SKILL.md + resources), retired generated skills, modified SKILL.md,
//! modified resource, added file, removed file, embedded symlinks (files and
//! directories, never followed), same-named user content, broken links, and
//! links targeting outside Orbit roots.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use tempfile::tempdir;

use crate::command::skill_ownership::{
    GeneratedFile, GeneratedSkill, SkillOwnership, classify_install, classify_symlink,
    load_manifest, owned_roots, reconcile_managed_skills, record_link_destinations,
    remove_owned_skill_links, save_manifest,
};

/// A single-file skill (`SKILL.md` only).
fn orbit_v1_solo() -> Vec<(&'static str, &'static str)> {
    vec![(
        "SKILL.md",
        "---\nname: orbit\ndescription: d\n---\n\n# Orbit\n",
    )]
}

/// A skill with a `references/` resource — exercises multi-file / directory
/// coverage that a `SKILL.md`-only hash would miss.
fn orbit_v1() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "SKILL.md",
            "---\nname: orbit\ndescription: d\n---\n\n# Orbit\n",
        ),
        ("references/guide.md", "# Guide\n\nUse orbit.\n"),
    ]
}

/// `orbit_v1` with only the *resource* changed — the `SKILL.md` is byte-identical.
fn orbit_v2_resource_bump() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "SKILL.md",
            "---\nname: orbit\ndescription: d\n---\n\n# Orbit\n",
        ),
        ("references/guide.md", "# Guide\n\nUse orbit, revised.\n"),
    ]
}

fn generated_files(items: &[(&str, &str)]) -> Vec<GeneratedFile> {
    items
        .iter()
        .map(|(rel, content)| GeneratedFile {
            relative_path: rel.to_string(),
            contents: content.as_bytes().to_vec(),
        })
        .collect()
}

fn gen_skill(id: &str, items: &[(&str, &str)]) -> GeneratedSkill {
    GeneratedSkill::from_files(id, Some("1".to_string()), &generated_files(items))
        .expect("fingerprint generated skill")
}

/// Materialize a skill tree at `<root>/<id>/` from `(relative_path, contents)`.
fn write_tree(root: &Path, id: &str, items: &[(&str, &str)]) -> PathBuf {
    let dir = root.join(id);
    fs::create_dir_all(&dir).expect("create skill dir");
    for (rel, content) in items {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, content).expect("write file");
    }
    dir
}

#[test]
fn reconcile_records_current_generated_skills() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    let ga = gen_skill("orbit", &orbit_v1());
    let gb = gen_skill("orbit-task", &orbit_v1_solo());
    reconcile_managed_skills(&skills_root, &[ga.clone(), gb.clone()]).expect("reconcile");

    let manifest = load_manifest(&skills_root).expect("load");
    assert_eq!(manifest.managed.len(), 2);
    let rec = manifest.managed.get("orbit").expect("orbit record");
    assert_eq!(rec.tree_fingerprint, ga.tree_fingerprint);
    assert_eq!(rec.version.as_deref(), Some("1"));
    assert_eq!(rec.owned_root, skills_root);
    assert!(manifest.tombstones.is_empty());
}

#[test]
fn current_generated_tree_with_resources_is_orbit_owned() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1())]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    // The exact generated tree (SKILL.md + references/guide.md) is proven owned.
    let install = write_tree(&skills_root, "orbit", &orbit_v1());
    assert_eq!(
        classify_install(&manifest, &roots, "orbit", &install).expect("classify"),
        SkillOwnership::OrbitOwned
    );
}

#[test]
fn retired_skill_moves_to_tombstone_and_stays_owned_by_fingerprint() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    let ga = gen_skill("orbit", &orbit_v1());
    let gb = gen_skill("orbit-legacy", &orbit_v1());
    reconcile_managed_skills(&skills_root, &[ga.clone(), gb.clone()]).expect("seed both");

    // Drop orbit-legacy from the catalog.
    reconcile_managed_skills(&skills_root, std::slice::from_ref(&ga)).expect("retire legacy");

    let manifest = load_manifest(&skills_root).expect("load");
    assert!(!manifest.managed.contains_key("orbit-legacy"));
    let tomb = manifest
        .tombstones
        .get("orbit-legacy")
        .expect("legacy tombstone");
    assert!(tomb.tree_fingerprints.contains(&gb.tree_fingerprint));
    assert_eq!(tomb.retired_version.as_deref(), Some("1"));

    // A retired generated copy on disk is still provably Orbit-owned by fingerprint.
    let install = write_tree(tmp.path(), "orbit-legacy", &orbit_v1());
    let roots = owned_roots(&skills_root, &manifest);
    assert_eq!(
        classify_install(&manifest, &roots, "orbit-legacy", &install).expect("classify"),
        SkillOwnership::OrbitOwned
    );
}

#[test]
fn resource_change_keeps_previous_fingerprint_as_tombstone() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    let v1 = gen_skill("orbit", &orbit_v1());
    reconcile_managed_skills(&skills_root, std::slice::from_ref(&v1)).expect("seed v1");
    // Only the resource file changed — a SKILL.md-only hash would miss this bump.
    let v2 = gen_skill("orbit", &orbit_v2_resource_bump());
    assert_ne!(v1.tree_fingerprint, v2.tree_fingerprint);
    reconcile_managed_skills(&skills_root, std::slice::from_ref(&v2)).expect("seed v2");

    let manifest = load_manifest(&skills_root).expect("load");
    assert_eq!(
        manifest.managed.get("orbit").unwrap().tree_fingerprint,
        v2.tree_fingerprint
    );
    let tomb = manifest.tombstones.get("orbit").expect("orbit tombstone");
    assert!(tomb.tree_fingerprints.contains(&v1.tree_fingerprint));

    // A pre-upgrade (v1) install still classifies as Orbit-owned via the tombstone.
    let install = write_tree(tmp.path(), "orbit", &orbit_v1());
    let roots = owned_roots(&skills_root, &manifest);
    assert_eq!(
        classify_install(&manifest, &roots, "orbit", &install).expect("classify"),
        SkillOwnership::OrbitOwned
    );
}

#[test]
fn modified_skill_md_is_ambiguous() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1())]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    let install = write_tree(&skills_root, "orbit", &orbit_v1());
    fs::write(
        install.join("SKILL.md"),
        "---\nname: orbit\ndescription: d\n---\n\n# Edited by user\n",
    )
    .expect("edit SKILL.md");
    assert_eq!(
        classify_install(&manifest, &roots, "orbit", &install).expect("classify"),
        SkillOwnership::Ambiguous
    );
}

#[test]
fn modified_resource_is_ambiguous() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1())]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    // SKILL.md untouched; a *generated resource* is edited. The old SKILL.md-only
    // hash claimed this; the whole-tree fingerprint fails closed.
    let install = write_tree(&skills_root, "orbit", &orbit_v1());
    fs::write(install.join("references/guide.md"), "# Guide\n\nEDITED\n").expect("edit resource");
    assert_eq!(
        classify_install(&manifest, &roots, "orbit", &install).expect("classify"),
        SkillOwnership::Ambiguous
    );
}

#[test]
fn added_file_is_ambiguous() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1())]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    // Every generated file is intact, but the user added an extra file.
    let install = write_tree(&skills_root, "orbit", &orbit_v1());
    fs::write(install.join("references/extra.md"), "user content\n").expect("add file");
    assert_eq!(
        classify_install(&manifest, &roots, "orbit", &install).expect("classify"),
        SkillOwnership::Ambiguous
    );
}

#[test]
fn removed_generated_file_is_ambiguous() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1())]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    // A generated resource was deleted — the tree is no longer complete.
    let install = write_tree(&skills_root, "orbit", &orbit_v1());
    fs::remove_file(install.join("references/guide.md")).expect("remove file");
    assert_eq!(
        classify_install(&manifest, &roots, "orbit", &install).expect("classify"),
        SkillOwnership::Ambiguous
    );
}

#[test]
fn embedded_file_symlink_is_not_followed_and_is_ambiguous() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1())]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    // Replace a generated resource with a symlink whose target holds the *exact*
    // generated bytes. Following it would match; refusing to follow must not.
    let external = tmp.path().join("external_guide.md");
    fs::write(&external, "# Guide\n\nUse orbit.\n").expect("write external");
    let install = write_tree(&skills_root, "orbit", &orbit_v1());
    fs::remove_file(install.join("references/guide.md")).expect("remove resource");
    symlink(&external, install.join("references/guide.md")).expect("symlink resource");
    assert_eq!(
        classify_install(&manifest, &roots, "orbit", &install).expect("classify"),
        SkillOwnership::Ambiguous
    );
}

#[test]
fn embedded_directory_symlink_is_not_followed_and_is_ambiguous() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1())]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    // `references/` is a symlink to an external directory with the exact generated
    // resource. The walk must record it as a symlink and never traverse into it.
    let external = tmp.path().join("external_refs");
    fs::create_dir_all(&external).expect("mkdir external");
    fs::write(external.join("guide.md"), "# Guide\n\nUse orbit.\n").expect("write external");
    let install = skills_root.join("orbit");
    fs::create_dir_all(&install).expect("mkdir install");
    fs::write(
        install.join("SKILL.md"),
        "---\nname: orbit\ndescription: d\n---\n\n# Orbit\n",
    )
    .expect("write SKILL.md");
    symlink(&external, install.join("references")).expect("symlink dir");
    assert_eq!(
        classify_install(&manifest, &roots, "orbit", &install).expect("classify"),
        SkillOwnership::Ambiguous
    );
}

#[test]
fn same_named_user_directory_is_unmanaged() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1())]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    // A user directory whose id Orbit never managed.
    let install = write_tree(
        tmp.path(),
        "my-custom-skill",
        &[(
            "SKILL.md",
            "---\nname: my-custom-skill\ndescription: d\n---\n\n# Mine\n",
        )],
    );
    assert_eq!(
        classify_install(&manifest, &roots, "my-custom-skill", &install).expect("classify"),
        SkillOwnership::Unmanaged
    );
}

#[test]
fn symlink_into_owned_root_is_orbit_owned() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    let target = write_tree(&skills_root, "orbit", &orbit_v1_solo());
    let links_dir = tmp.path().join("links");
    fs::create_dir_all(&links_dir).expect("mkdir links");
    let link = links_dir.join("orbit");
    symlink(&target, &link).expect("symlink");

    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);
    assert_eq!(classify_symlink(&link, &roots), SkillOwnership::OrbitOwned);
}

#[test]
fn broken_symlink_into_owned_root_is_orbit_owned() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir skills");
    let links_dir = tmp.path().join("links");
    fs::create_dir_all(&links_dir).expect("mkdir links");
    // Target directory never created / already removed — broken link.
    let link = links_dir.join("orbit");
    symlink(skills_root.join("orbit"), &link).expect("symlink");

    let roots = vec![skills_root.clone()];
    assert_eq!(classify_symlink(&link, &roots), SkillOwnership::OrbitOwned);
}

#[test]
fn symlink_outside_owned_roots_is_ambiguous() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir skills");
    let elsewhere = write_tree(
        &tmp.path().join("user"),
        "orbit",
        &[("SKILL.md", "# user\n")],
    );
    let links_dir = tmp.path().join("links");
    fs::create_dir_all(&links_dir).expect("mkdir links");
    let link = links_dir.join("orbit");
    symlink(&elsewhere, &link).expect("symlink");

    let roots = vec![skills_root.clone()];
    assert_eq!(classify_symlink(&link, &roots), SkillOwnership::Ambiguous);
}

#[test]
fn remove_owned_skill_links_removes_only_proven_links() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    let owned_target = write_tree(&skills_root, "orbit", &[("SKILL.md", "# orbit\n")]);
    let user_target = write_tree(
        &tmp.path().join("user"),
        "mine",
        &[("SKILL.md", "# mine\n")],
    );

    let links_dir = tmp.path().join("links");
    fs::create_dir_all(&links_dir).expect("mkdir links");
    symlink(&owned_target, links_dir.join("orbit")).expect("owned link");
    symlink(&user_target, links_dir.join("mine")).expect("user link");
    // A plain regular file must never be touched.
    fs::write(links_dir.join("README.md"), "keep me").expect("regular file");

    let roots = vec![skills_root.clone()];
    let removed = remove_owned_skill_links(&links_dir, &roots).expect("remove");
    assert_eq!(removed, 1);
    assert!(!links_dir.join("orbit").exists());
    assert!(fs::symlink_metadata(links_dir.join("mine")).is_ok());
    assert!(links_dir.join("README.md").exists());
}

#[test]
fn record_link_destinations_annotates_managed_records_only() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");
    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1_solo())]).expect("seed");

    let managed_link = tmp.path().join(".claude/skills/orbit");
    let unknown_link = tmp.path().join(".claude/skills/unknown");
    record_link_destinations(
        &skills_root,
        &[
            ("orbit".to_string(), managed_link.clone()),
            ("unknown".to_string(), unknown_link),
        ],
    )
    .expect("record");

    let manifest = load_manifest(&skills_root).expect("load");
    let rec = manifest.managed.get("orbit").expect("orbit record");
    assert_eq!(rec.link_destinations, vec![managed_link]);
    // The unknown id was ignored — no phantom record created.
    assert_eq!(manifest.managed.len(), 1);
}

#[test]
fn corrupt_manifest_surfaces_error() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");
    fs::write(skills_root.join(".ownership.json"), "{ not json").expect("write corrupt");
    assert!(load_manifest(&skills_root).is_err());
}

#[test]
fn manifest_round_trips_through_save_load() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");
    reconcile_managed_skills(&skills_root, &[gen_skill("orbit", &orbit_v1())]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    save_manifest(&skills_root, &manifest).expect("save");
    let reloaded = load_manifest(&skills_root).expect("reload");
    assert_eq!(manifest, reloaded);
}
