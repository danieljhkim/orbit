//! Ownership + retirement metadata tests, covering the classification matrix
//! from ORB-10181: current generated skills, retired generated skills, modified
//! copies, same-named user content, broken links, and links targeting outside
//! Orbit roots.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use tempfile::tempdir;

use crate::command::skill_ownership::{
    GeneratedSkill, SkillOwnership, classify_install, classify_symlink, content_hash,
    load_manifest, owned_roots, reconcile_managed_skills, record_link_destinations,
    remove_owned_skill_links, save_manifest,
};

fn generated(id: &str, body: &str) -> (GeneratedSkill, String) {
    let content = format!("---\nname: {id}\ndescription: d\n---\n\n# {body}\n");
    let g = GeneratedSkill {
        skill_id: id.to_string(),
        content_hash: content_hash(content.as_bytes()),
        version: Some("1".to_string()),
    };
    (g, content)
}

fn write_skill_dir(root: &Path, id: &str, content: &str) -> PathBuf {
    let dir = root.join(id);
    fs::create_dir_all(&dir).expect("create skill dir");
    fs::write(dir.join("SKILL.md"), content).expect("write SKILL.md");
    dir
}

#[test]
fn reconcile_records_current_generated_skills() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    let (ga, _) = generated("orbit", "Orbit");
    let (gb, _) = generated("orbit-task", "Task");
    reconcile_managed_skills(&skills_root, &[ga.clone(), gb.clone()]).expect("reconcile");

    let manifest = load_manifest(&skills_root).expect("load");
    assert_eq!(manifest.managed.len(), 2);
    let rec = manifest.managed.get("orbit").expect("orbit record");
    assert_eq!(rec.content_hash, ga.content_hash);
    assert_eq!(rec.version.as_deref(), Some("1"));
    assert_eq!(rec.owned_root, skills_root);
    assert!(manifest.tombstones.is_empty());
}

#[test]
fn retired_skill_moves_to_tombstone_and_stays_owned_by_hash() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    let (ga, _) = generated("orbit", "Orbit");
    let (gb, gb_content) = generated("orbit-legacy", "Legacy");
    reconcile_managed_skills(&skills_root, &[ga.clone(), gb.clone()]).expect("seed both");

    // Drop orbit-legacy from the catalog.
    reconcile_managed_skills(&skills_root, &[ga.clone()]).expect("retire legacy");

    let manifest = load_manifest(&skills_root).expect("load");
    assert!(!manifest.managed.contains_key("orbit-legacy"));
    let tomb = manifest
        .tombstones
        .get("orbit-legacy")
        .expect("legacy tombstone");
    assert!(tomb.content_hashes.contains(&gb.content_hash));
    assert_eq!(tomb.retired_version.as_deref(), Some("1"));

    // A retired generated copy on disk is still provably Orbit-owned by hash.
    let install = write_skill_dir(tmp.path(), "orbit-legacy", &gb_content);
    let roots = owned_roots(&skills_root, &manifest);
    assert_eq!(
        classify_install(&manifest, &roots, "orbit-legacy", &install).expect("classify"),
        SkillOwnership::OrbitOwned
    );
}

#[test]
fn template_change_keeps_previous_hash_as_tombstone() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    let (v1, v1_content) = generated("orbit", "V1");
    reconcile_managed_skills(&skills_root, &[v1.clone()]).expect("seed v1");
    let (v2, _) = generated("orbit", "V2");
    reconcile_managed_skills(&skills_root, &[v2.clone()]).expect("seed v2");

    let manifest = load_manifest(&skills_root).expect("load");
    // Current hash is v2; v1 hash survives as a known-generated hash.
    assert_eq!(
        manifest.managed.get("orbit").unwrap().content_hash,
        v2.content_hash
    );
    let tomb = manifest.tombstones.get("orbit").expect("orbit tombstone");
    assert!(tomb.content_hashes.contains(&v1.content_hash));

    // A pre-upgrade (v1) install still classifies as Orbit-owned.
    let install = write_skill_dir(tmp.path(), "orbit", &v1_content);
    let roots = owned_roots(&skills_root, &manifest);
    assert_eq!(
        classify_install(&manifest, &roots, "orbit", &install).expect("classify"),
        SkillOwnership::OrbitOwned
    );
}

#[test]
fn modified_generated_skill_is_ambiguous() {
    let tmp = tempdir().expect("tmp");
    let skills_root = tmp.path().join("skills");
    fs::create_dir_all(&skills_root).expect("mkdir");

    let (g, _) = generated("orbit", "Orbit");
    reconcile_managed_skills(&skills_root, &[g]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    // User edited the generated skill — hash no longer matches.
    let install = write_skill_dir(
        tmp.path(),
        "orbit",
        "---\nname: orbit\ndescription: d\n---\n\n# Edited by user\n",
    );
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

    let (g, _) = generated("orbit", "Orbit");
    reconcile_managed_skills(&skills_root, &[g]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    let roots = owned_roots(&skills_root, &manifest);

    // A user directory whose id Orbit never managed.
    let install = write_skill_dir(
        tmp.path(),
        "my-custom-skill",
        "---\nname: my-custom-skill\ndescription: d\n---\n\n# Mine\n",
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
    let target = write_skill_dir(
        &skills_root,
        "orbit",
        "---\nname: orbit\ndescription: d\n---\n\n# Orbit\n",
    );
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
    let elsewhere = write_skill_dir(&tmp.path().join("user"), "orbit", "# user\n");
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
    let owned_target = write_skill_dir(&skills_root, "orbit", "# orbit\n");
    let user_target = write_skill_dir(&tmp.path().join("user"), "mine", "# mine\n");

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
    let (g, _) = generated("orbit", "Orbit");
    reconcile_managed_skills(&skills_root, &[g]).expect("seed");

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
    let (g, _) = generated("orbit", "Orbit");
    reconcile_managed_skills(&skills_root, &[g]).expect("seed");
    let manifest = load_manifest(&skills_root).expect("load");
    save_manifest(&skills_root, &manifest).expect("save");
    let reloaded = load_manifest(&skills_root).expect("reload");
    assert_eq!(manifest, reloaded);
}
