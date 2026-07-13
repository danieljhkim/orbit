//! `orbit gc skills` collector tests, covering the eligibility matrix from
//! ORB-10187/§3.6: retired generated directories, modified generated content,
//! same-named user content, stale/broken owned links, escaping (foreign) links,
//! current-skill link repair reporting, dry-run/apply parity, and idempotency.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use tempfile::TempDir;

use crate::command::gc::{
    GcClock, GcItemStatus, GcMode, GcOutcome, GcReport, GcRequest, GcScope, execute_gc,
};
use crate::command::skill_gc::SkillsGcCollector;
use crate::command::skill_ownership::{GeneratedFile, GeneratedSkill, reconcile_managed_skills};

struct FakeClock(DateTime<Utc>);

impl GcClock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// The exact generated tree Orbit seeds for a v1 skill: `SKILL.md` + a resource.
fn v1_tree(id: &str) -> Vec<(String, String)> {
    vec![
        (
            "SKILL.md".to_string(),
            format!("---\nname: {id}\ndescription: d\n---\n\n# {id}\n"),
        ),
        (
            "references/guide.md".to_string(),
            format!("# Guide for {id}\n"),
        ),
    ]
}

fn generated(id: &str) -> GeneratedSkill {
    let files: Vec<GeneratedFile> = v1_tree(id)
        .into_iter()
        .map(|(relative_path, contents)| GeneratedFile {
            relative_path,
            contents: contents.into_bytes(),
        })
        .collect();
    GeneratedSkill::from_files(id, Some("1".to_string()), &files).expect("fingerprint")
}

/// Materialize the exact generated tree for `id` at `<root>/<id>/`.
fn write_generated(root: &Path, id: &str) -> PathBuf {
    write_tree(root, id, &v1_tree(id))
}

fn write_tree(root: &Path, id: &str, items: &[(String, String)]) -> PathBuf {
    let dir = root.join(id);
    for (relative, contents) in items {
        let path = dir.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, contents).expect("write");
    }
    dir
}

struct Fixture {
    _temp: TempDir,
    skills_root: PathBuf,
    agents: PathBuf,
    claude: PathBuf,
    global_root: PathBuf,
    state: PathBuf,
    clock: FakeClock,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp");
        let global_root = temp.path().join(".orbit");
        let skills_root = global_root.join("skills");
        fs::create_dir_all(&skills_root).expect("skills root");
        let agents = temp.path().join(".agents").join("skills");
        let claude = temp.path().join(".claude").join("skills");
        Self {
            skills_root,
            agents,
            claude,
            global_root,
            state: temp.path().join("state"),
            clock: FakeClock(
                Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
                    .single()
                    .expect("time"),
            ),
            _temp: temp,
        }
    }

    fn collector(&self) -> SkillsGcCollector {
        SkillsGcCollector::with_roots(
            self.skills_root.clone(),
            vec![self.agents.clone(), self.claude.clone()],
        )
    }

    fn request(&self, apply: bool) -> GcRequest<'_> {
        GcRequest {
            apply,
            scope: GcScope::Global {
                root: self.global_root.clone(),
            },
            retention_override: None,
            global_state_dir: &self.state,
            clock: &self.clock,
        }
    }

    fn link(root: &Path, name: &str, target: &Path) {
        fs::create_dir_all(root).expect("link root");
        symlink(target, root.join(name)).expect("symlink");
    }
}

/// Candidate ids in the (single) skills target report, by item status.
fn item_ids(report: &GcReport, status: GcItemStatus) -> BTreeSet<String> {
    report.targets[0]
        .items
        .iter()
        .filter(|item| item.status == status)
        .map(|item| item.id.clone())
        .collect()
}

/// Map of skip id -> code in the skills target report.
fn skip_codes(report: &GcReport) -> BTreeMap<String, String> {
    report.targets[0]
        .skipped
        .iter()
        .map(|skip| (skip.id.clone(), skip.code.clone()))
        .collect()
}

/// Build the full mixed fixture: two current skills (`orbit`, `orbit-mod`), two
/// retired skills (`orbit-legacy`, `orbit-old`), on-disk generated/modified/user
/// dirs, and a spread of owned/broken/foreign links.
fn mixed_fixture() -> (Fixture, PathBuf) {
    let fx = Fixture::new();

    // Manifest: seed all four, then retire orbit-legacy and orbit-old.
    reconcile_managed_skills(
        &fx.skills_root,
        &[
            generated("orbit"),
            generated("orbit-mod"),
            generated("orbit-legacy"),
            generated("orbit-old"),
        ],
    )
    .expect("seed");
    reconcile_managed_skills(
        &fx.skills_root,
        &[generated("orbit"), generated("orbit-mod")],
    )
    .expect("retire");

    // Generated directories on disk.
    write_generated(&fx.skills_root, "orbit"); // current + intact => healthy
    write_generated(&fx.skills_root, "orbit-legacy"); // retired + intact => remove

    // Current generated skill with a modified resource => modified_generated skip.
    let modified = write_generated(&fx.skills_root, "orbit-mod");
    fs::write(modified.join("references/guide.md"), "# edited by user\n").expect("edit");

    // Same-named user directory Orbit never generated => user_content skip.
    write_tree(
        &fx.skills_root,
        "my-custom",
        &[("SKILL.md".to_string(), "# mine\n".to_string())],
    );

    // Foreign target outside every owned root.
    let outside = fx._temp.path().join("outside");
    fs::create_dir_all(outside.join("evil")).expect("outside");
    fs::write(outside.join("evil").join("SKILL.md"), "# evil\n").expect("evil file");

    // .agents links: healthy current, owned-retired (target exists), owned-broken.
    Fixture::link(&fx.agents, "orbit", &fx.skills_root.join("orbit"));
    Fixture::link(
        &fx.agents,
        "orbit-legacy",
        &fx.skills_root.join("orbit-legacy"),
    );
    Fixture::link(&fx.agents, "orbit-old", &fx.skills_root.join("orbit-old")); // broken target
    // A plain user directory in the link root must never be touched.
    fs::create_dir_all(fx.agents.join("user-skill")).expect("user dir");
    fs::write(fx.agents.join("user-skill").join("SKILL.md"), "# user\n").expect("user file");

    // .claude links: a foreign link only (current skills missing => repair).
    Fixture::link(&fx.claude, "evil", &outside.join("evil"));

    (fx, outside)
}

#[test]
fn plan_finds_retired_dirs_and_stale_links_and_retains_conflicts() {
    let (fx, _outside) = mixed_fixture();
    let collector = fx.collector();

    let report = execute_gc(&collector, fx.request(false)).expect("plan");
    assert_eq!(report.mode, GcMode::Plan);
    assert_eq!(report.outcome, GcOutcome::Clean);

    let eligible = item_ids(&report, GcItemStatus::Eligible);
    let expected: BTreeSet<String> = [
        "dir:orbit-legacy".to_string(),
        format!("link:{}", fx.agents.join("orbit-legacy").display()),
        format!("link:{}", fx.agents.join("orbit-old").display()),
    ]
    .into_iter()
    .collect();
    assert_eq!(eligible, expected, "unexpected removal candidates");

    // Plan is a no-op: nothing removed.
    assert!(fx.skills_root.join("orbit-legacy").exists());
    assert!(fx.agents.join("orbit-legacy").symlink_metadata().is_ok());
    assert_eq!(report.targets[0].counts.reclaimed, 0);

    let codes = skip_codes(&report);
    assert_eq!(
        codes.get("dir:orbit-mod").map(String::as_str),
        Some("modified_generated")
    );
    assert_eq!(
        codes.get("dir:my-custom").map(String::as_str),
        Some("user_content")
    );
    assert_eq!(
        codes
            .get(&format!("link:{}", fx.agents.join("user-skill").display()))
            .map(String::as_str),
        Some("user_content")
    );
    assert_eq!(
        codes
            .get(&format!("link:{}", fx.claude.join("evil").display()))
            .map(String::as_str),
        Some("foreign_link")
    );
}

#[test]
fn current_skill_link_repair_is_reported_separately_from_retirement() {
    let (fx, _outside) = mixed_fixture();
    let collector = fx.collector();
    let report = execute_gc(&collector, fx.request(false)).expect("plan");

    let codes = skip_codes(&report);
    // orbit-mod has no link in .agents; orbit and orbit-mod have none in .claude.
    for missing in [
        fx.agents.join("orbit-mod"),
        fx.claude.join("orbit"),
        fx.claude.join("orbit-mod"),
    ] {
        assert_eq!(
            codes
                .get(&format!("link:{}", missing.display()))
                .map(String::as_str),
            Some("link_repair"),
            "expected link_repair for {}",
            missing.display()
        );
    }
    // Repairs are skips, never removal candidates.
    let eligible = item_ids(&report, GcItemStatus::Eligible);
    assert!(eligible.iter().all(|id| !id.contains("link_repair")));
}

#[test]
fn broken_owned_link_for_current_skill_is_repair_not_removal() {
    let fx = Fixture::new();
    reconcile_managed_skills(&fx.skills_root, &[generated("orbit")]).expect("seed");
    write_generated(&fx.skills_root, "orbit");
    // The link is Orbit-owned but its target was removed. Because orbit is still
    // a managed skill, this is a repair, not a retirement removal.
    fs::remove_dir_all(fx.skills_root.join("orbit")).expect("remove target");
    Fixture::link(&fx.agents, "orbit", &fx.skills_root.join("orbit"));

    let report = execute_gc(&fx.collector(), fx.request(false)).expect("plan");
    assert!(item_ids(&report, GcItemStatus::Eligible).is_empty());
    let codes = skip_codes(&report);
    assert_eq!(
        codes
            .get(&format!("link:{}", fx.agents.join("orbit").display()))
            .map(String::as_str),
        Some("link_repair")
    );
}

#[test]
fn apply_removes_only_proven_entries_and_preserves_the_rest() {
    let (fx, outside) = mixed_fixture();
    let collector = fx.collector();

    let plan = execute_gc(&collector, fx.request(false)).expect("plan");
    let plan_eligible = item_ids(&plan, GcItemStatus::Eligible);

    let apply = execute_gc(&collector, fx.request(true)).expect("apply");
    assert_eq!(apply.mode, GcMode::Apply);
    assert_eq!(apply.outcome, GcOutcome::Clean);

    // Dry-run/apply parity: exactly the frozen candidates were reclaimed.
    let reclaimed = item_ids(&apply, GcItemStatus::Reclaimed);
    assert_eq!(reclaimed, plan_eligible);
    assert_eq!(apply.targets[0].counts.reclaimed, 3);

    // Removed: the retired directory and the two stale links.
    assert!(!fx.skills_root.join("orbit-legacy").exists());
    assert!(fx.agents.join("orbit-legacy").symlink_metadata().is_err());
    assert!(fx.agents.join("orbit-old").symlink_metadata().is_err());

    // Preserved: healthy link, current/modified/user dirs, user link dir.
    assert!(fx.agents.join("orbit").symlink_metadata().is_ok());
    assert!(fx.skills_root.join("orbit").join("SKILL.md").exists());
    assert!(fx.skills_root.join("orbit-mod").join("SKILL.md").exists());
    assert!(fx.skills_root.join("my-custom").join("SKILL.md").exists());
    assert!(fx.agents.join("user-skill").join("SKILL.md").exists());

    // The foreign link is retained and its target is never traversed/removed.
    assert!(fx.claude.join("evil").symlink_metadata().is_ok());
    assert!(outside.join("evil").join("SKILL.md").exists());
}

#[test]
fn second_apply_is_idempotent() {
    let (fx, _outside) = mixed_fixture();
    let collector = fx.collector();

    let first = execute_gc(&collector, fx.request(true)).expect("first apply");
    assert_eq!(first.targets[0].counts.reclaimed, 3);

    let second = execute_gc(&collector, fx.request(true)).expect("second apply");
    assert_eq!(second.targets[0].counts.eligible, 0);
    assert_eq!(second.targets[0].counts.reclaimed, 0);
    assert_eq!(second.outcome, GcOutcome::Clean);
}

#[test]
fn absent_managed_root_reports_repair_for_every_current_skill() {
    let fx = Fixture::new();
    reconcile_managed_skills(
        &fx.skills_root,
        &[generated("orbit"), generated("orbit-mod")],
    )
    .expect("seed");
    write_generated(&fx.skills_root, "orbit");
    write_generated(&fx.skills_root, "orbit-mod");
    // Neither managed link root exists on disk. A fully-absent root must not read
    // as healthy: it is an empty present set, so every current managed skill is a
    // missing link there and reported as link_repair.
    assert!(!fx.agents.exists());
    assert!(!fx.claude.exists());

    let report = execute_gc(&fx.collector(), fx.request(false)).expect("plan");
    assert!(
        item_ids(&report, GcItemStatus::Eligible).is_empty(),
        "an absent root has nothing owned to reclaim"
    );

    let codes = skip_codes(&report);
    for root in [&fx.agents, &fx.claude] {
        for skill in ["orbit", "orbit-mod"] {
            assert_eq!(
                codes
                    .get(&format!("link:{}", root.join(skill).display()))
                    .map(String::as_str),
                Some("link_repair"),
                "expected link_repair for `{skill}` under absent root {}",
                root.display()
            );
        }
    }

    // An absent root drives no mutation on apply and stays absent.
    let apply = execute_gc(&fx.collector(), fx.request(true)).expect("apply");
    assert_eq!(apply.targets[0].counts.reclaimed, 0);
    assert!(!fx.agents.exists());
    assert!(!fx.claude.exists());
}

#[test]
fn non_directory_managed_root_is_retained_fail_closed() {
    let fx = Fixture::new();
    reconcile_managed_skills(&fx.skills_root, &[generated("orbit")]).expect("seed");
    write_generated(&fx.skills_root, "orbit");
    // `.agents/skills` exists but is a plain file, not a directory. Fail closed:
    // report the root, never traverse it, and never repair-report through it.
    fs::create_dir_all(fx.agents.parent().expect("parent")).expect("mkdir parent");
    fs::write(&fx.agents, "not a directory\n").expect("write file root");

    let report = execute_gc(&fx.collector(), fx.request(true)).expect("apply");
    assert!(item_ids(&report, GcItemStatus::Eligible).is_empty());

    let codes = skip_codes(&report);
    assert_eq!(
        codes
            .get(&format!("root:{}", fx.agents.display()))
            .map(String::as_str),
        Some("foreign_root"),
        "a non-directory managed root is reported fail-closed"
    );
    assert!(
        !codes.contains_key(&format!("link:{}", fx.agents.join("orbit").display())),
        "must not repair-report through a non-directory root"
    );
    // The file root is left intact and never removed.
    assert!(fx.agents.is_file());
}

#[test]
fn apply_prunes_emptied_owned_link_dirs() {
    let fx = Fixture::new();
    reconcile_managed_skills(&fx.skills_root, &[generated("orbit")]).expect("seed");
    // Retire orbit so its owned link is stale (its generated dir is already gone).
    reconcile_managed_skills(&fx.skills_root, &[]).expect("retire orbit");
    // The link root holds only the one stale, now-broken owned link.
    Fixture::link(&fx.agents, "orbit", &fx.skills_root.join("orbit"));

    let report = execute_gc(&fx.collector(), fx.request(true)).expect("apply");
    assert_eq!(report.targets[0].counts.reclaimed, 1);
    // The now-empty `.agents/skills` and its `.agents` parent are pruned.
    assert!(!fx.agents.exists());
    assert!(!fx.agents.parent().expect("parent").exists());
}
