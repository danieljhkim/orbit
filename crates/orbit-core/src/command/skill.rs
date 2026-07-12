use std::path::Path;

use std::collections::HashSet;

use orbit_common::types::OrbitError;

use orbit_common::utility::fs::write_text_with_parent;

use crate::OrbitRuntime;
use crate::skill_catalog::{LoadedSkill, SkillCatalogDoctorStatus};

const DEFAULT_SKILL_FILES: [(&str, &str); 5] = [
    ("orbit", include_str!("../../assets/skills/orbit/SKILL.md")),
    (
        "orbit-task",
        include_str!("../../assets/skills/orbit-task/SKILL.md"),
    ),
    (
        "orbit-workflow",
        include_str!("../../assets/skills/orbit-workflow/SKILL.md"),
    ),
    (
        "orbit-search",
        include_str!("../../assets/skills/orbit-search/SKILL.md"),
    ),
    (
        "orbit-knowledge",
        include_str!("../../assets/skills/orbit-knowledge/SKILL.md"),
    ),
];

const DEFAULT_SKILL_RESOURCE_FILES: [(&str, &str, &str); 4] = [
    (
        "orbit-workflow",
        "references/debug-job-failure.md",
        include_str!("../../assets/skills/orbit-workflow/references/debug-job-failure.md"),
    ),
    (
        "orbit-workflow",
        "references/common_failures.md",
        include_str!("../../assets/skills/orbit-workflow/references/common_failures.md"),
    ),
    (
        "orbit-workflow",
        "references/operational-logs.md",
        include_str!("../../assets/skills/orbit-workflow/references/operational-logs.md"),
    ),
    (
        "orbit",
        "references/guide.md",
        include_str!("../../assets/skills/orbit/references/guide.md"),
    ),
];

/// Skills intentionally NOT shipped in `plugin/skills/` because they depend on
/// CLI-only surfaces the Claude Code plugin does not expose. The CLI still
/// seeds them; the plugin omits the symlink. Update this list when adding a
/// skill that should be CLI-only — the `plugin_skill_symlinks_resolve_to_assets`
/// test reads it.
#[cfg(test)]
const PLUGIN_EXCLUDED_SKILLS: &[&str] = &[
    // No `orbit run` surface in the plugin (see README "Plugin vs. CLI"), so
    // there are no jobs/routines/sweeps to run or debug.
    "orbit-workflow",
];
use crate::paths::ORBIT_ROOT_TOKEN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillDoctorStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct SkillDoctorResult {
    pub skill_name: String,
    pub status: SkillDoctorStatus,
    pub message: String,
}

pub(crate) fn default_skill_ids() -> [&'static str; 5] {
    DEFAULT_SKILL_FILES.map(|(id, _)| id)
}

pub(crate) fn seed_default_skills(
    skills_root: &Path,
    orbit_root: &Path,
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let mut count = 0usize;
    for (id, content) in DEFAULT_SKILL_FILES {
        let path = skills_root.join(id).join("SKILL.md");
        if !overwrite && path.exists() {
            continue;
        }
        let rendered = inject_skill_template_tokens(content, orbit_root);
        write_text_with_parent(&path, &rendered)?;
        seed_default_skill_resources(&skills_root.join(id), id, orbit_root, overwrite)?;
        count += 1;
    }
    Ok(count)
}

fn seed_default_skill_resources(
    skill_dir: &Path,
    skill_id: &str,
    orbit_root: &Path,
    overwrite: bool,
) -> Result<(), OrbitError> {
    for (resource_skill_id, relative_path, content) in DEFAULT_SKILL_RESOURCE_FILES {
        if resource_skill_id != skill_id {
            continue;
        }
        let path = skill_dir.join(relative_path);
        if !overwrite && path.exists() {
            continue;
        }
        let rendered = inject_skill_template_tokens(content, orbit_root);
        write_text_with_parent(&path, &rendered)?;
    }
    Ok(())
}

pub(crate) fn is_default_skill_file_for_root(
    skill_id: &str,
    path: &Path,
    orbit_root: &Path,
) -> Result<bool, OrbitError> {
    let Some((_, content)) = DEFAULT_SKILL_FILES
        .iter()
        .find(|(default_id, _)| *default_id == skill_id)
    else {
        return Ok(false);
    };
    if !path.exists() {
        return Ok(false);
    }
    let existing = std::fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    Ok(existing == inject_skill_template_tokens(content, orbit_root))
}

fn inject_skill_template_tokens(raw: &str, orbit_root: &Path) -> String {
    let orbit_root_value = orbit_root.to_string_lossy();
    raw.replace(ORBIT_ROOT_TOKEN, orbit_root_value.as_ref())
}

impl OrbitRuntime {
    pub fn list_file_skills(&self) -> Result<Vec<LoadedSkill>, OrbitError> {
        self.skill_catalog().list()
    }

    pub fn show_file_skill(&self, name: &str) -> Result<LoadedSkill, OrbitError> {
        self.skill_catalog().load(name)
    }

    pub fn doctor_file_skills(&self) -> Result<Vec<SkillDoctorResult>, OrbitError> {
        let rows = self.skill_catalog().doctor()?;
        Ok(rows
            .into_iter()
            .map(|row| SkillDoctorResult {
                skill_name: row.skill_id,
                status: match row.status {
                    SkillCatalogDoctorStatus::Ok => SkillDoctorStatus::Ok,
                    SkillCatalogDoctorStatus::Error => SkillDoctorStatus::Error,
                },
                message: row.message,
            })
            .collect())
    }

    pub(crate) fn resolve_activity_skill_refs(
        &self,
        refs: &[String],
    ) -> Result<Vec<LoadedSkill>, OrbitError> {
        let mut dedup = HashSet::new();
        let mut output = Vec::new();
        for skill_id in refs {
            if !dedup.insert(skill_id.clone()) {
                continue;
            }
            output.push(self.skill_catalog().load(skill_id)?);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod drift_tests {
    //! Parity tests guarding against drift between the four skill catalogs:
    //! the on-disk assets, the seeded registry, the plugin package, and the
    //! router skill's enumeration. The next agent who adds a skill folder
    //! must update all four; these tests fail loudly if any catalog lags.

    use super::*;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    fn assets_skills_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/skills")
    }

    fn repo_root() -> PathBuf {
        // CARGO_MANIFEST_DIR points at <repo>/crates/orbit-core. Walk up two
        // levels to reach the workspace root.
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("orbit-core has a parent (crates/)")
            .parent()
            .expect("crates/ has a parent (repo root)")
            .to_path_buf()
    }

    fn collect_relative_files(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
        let mut pending = vec![root.to_path_buf()];
        let mut files = BTreeSet::new();

        while let Some(dir) = pending.pop() {
            let entries =
                std::fs::read_dir(&dir).map_err(|e| format!("read_dir({}): {e}", dir.display()))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("read_dir({}) entry: {e}", dir.display()))?;
                let path = entry.path();
                let file_type = entry
                    .file_type()
                    .map_err(|e| format!("file_type({}): {e}", path.display()))?;
                if file_type.is_dir() {
                    pending.push(path);
                    continue;
                }
                if file_type.is_file() || file_type.is_symlink() {
                    let relative = path
                        .strip_prefix(root)
                        .map_err(|e| {
                            format!("strip_prefix({}, {}): {e}", root.display(), path.display())
                        })?
                        .to_path_buf();
                    files.insert(relative);
                }
            }
        }

        Ok(files)
    }

    fn plugin_skill_matches_asset(plugin_skill: &Path, asset_skill: &Path) -> Result<(), String> {
        let expected_path = asset_skill
            .canonicalize()
            .map_err(|e| format!("canonicalize asset path failed: {e}"))?;
        let actual_path = plugin_skill
            .canonicalize()
            .map_err(|e| format!("canonicalize plugin skill failed: {e}"))?;
        if actual_path == expected_path {
            return Ok(());
        }

        let plugin_files = collect_relative_files(plugin_skill)?;
        let asset_files = collect_relative_files(asset_skill)?;
        if plugin_files != asset_files {
            let missing_from_plugin: Vec<&PathBuf> =
                asset_files.difference(&plugin_files).collect();
            let extra_in_plugin: Vec<&PathBuf> = plugin_files.difference(&asset_files).collect();
            return Err(format!(
                "materialized files differ from asset directory (missing from plugin: {missing_from_plugin:?}; extra in plugin: {extra_in_plugin:?})"
            ));
        }

        for relative in asset_files {
            let plugin_bytes = std::fs::read(plugin_skill.join(&relative))
                .map_err(|e| format!("read plugin file {}: {e}", relative.display()))?;
            let asset_bytes = std::fs::read(asset_skill.join(&relative))
                .map_err(|e| format!("read asset file {}: {e}", relative.display()))?;
            if plugin_bytes != asset_bytes {
                return Err(format!(
                    "materialized file {} differs from asset source",
                    relative.display()
                ));
            }
        }

        Ok(())
    }

    #[test]
    fn asset_dirs_match_default_skill_ids() {
        let dir = assets_skills_dir();
        let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read_dir({}): {e}", dir.display()))
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let file_type = entry.file_type().ok()?;
                if !file_type.is_dir() {
                    return None;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('_') {
                    return None;
                }
                Some(name)
            })
            .collect();
        let registered: BTreeSet<String> = default_skill_ids()
            .iter()
            .map(|id| (*id).to_string())
            .collect();

        let missing_from_registry: Vec<&String> = on_disk.difference(&registered).collect();
        let missing_from_disk: Vec<&String> = registered.difference(&on_disk).collect();

        assert!(
            missing_from_registry.is_empty() && missing_from_disk.is_empty(),
            "skill catalogs disagree:\n  in assets/skills/ but NOT in default_skill_ids(): {missing_from_registry:?}\n  in default_skill_ids() but NOT in assets/skills/: {missing_from_disk:?}\nfix by editing crates/orbit-core/src/command/skill.rs::DEFAULT_SKILL_FILES or moving the asset directory under assets/skills/_archive/.",
        );
    }

    #[test]
    fn plugin_skill_symlinks_resolve_to_assets() {
        let repo = repo_root();
        let plugin_skills = repo.join("plugin/skills");
        let assets = repo.join("crates/orbit-core/assets/skills");
        let codex_manifest_path = repo.join("plugin/.codex-plugin/plugin.json");
        let marketplace_path = repo.join(".agents/plugins/marketplace.json");
        let excluded: BTreeSet<&str> = PLUGIN_EXCLUDED_SKILLS.iter().copied().collect();

        let mut failures: Vec<String> = Vec::new();

        // Forward: every non-excluded default skill must have a package entry
        // that either resolves to the asset directory or is a byte-for-byte
        // materialized copy. Codex plugin installs copy the plugin package
        // without following directory symlinks, so materialized files are valid.
        let expected_ids: BTreeSet<&str> = default_skill_ids()
            .iter()
            .copied()
            .filter(|id| !excluded.contains(id))
            .collect();
        for id in &expected_ids {
            let link = plugin_skills.join(id);
            if !link.exists() {
                failures.push(format!(
                    "  {id}: plugin/skills/{id} does not exist (create a symlink to or materialized copy of crates/orbit-core/assets/skills/{id})"
                ));
                continue;
            }
            if let Err(e) = plugin_skill_matches_asset(&link, &assets.join(id)) {
                failures.push(format!(
                    "  {id}: plugin/skills/{id} does not match asset: {e}"
                ));
            }
        }

        // L-0020: retired skills can leave stale package entries behind, so
        // keep this reverse check strict about orphans. Reverse: no orphan
        // entries in plugin/skills/ (catches stale entries for retired skills
        // and accidental inclusion of an excluded skill).
        let on_disk: BTreeSet<String> = std::fs::read_dir(&plugin_skills)
            .unwrap_or_else(|e| panic!("read_dir({}): {e}", plugin_skills.display()))
            .filter_map(|entry| {
                entry
                    .ok()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
            })
            .collect();
        for name in &on_disk {
            if excluded.contains(name.as_str()) {
                failures.push(format!(
                    "  {name}: plugin/skills/{name} exists but is in PLUGIN_EXCLUDED_SKILLS — either remove the symlink or remove the exclusion"
                ));
                continue;
            }
            if !expected_ids.contains(name.as_str()) {
                failures.push(format!(
                    "  {name}: plugin/skills/{name} has no matching entry in default_skill_ids() — remove the orphan symlink or register the skill"
                ));
            }
        }

        // The Codex plugin package intentionally reuses the same shared skill
        // directory as the Claude plugin. Keep that manifest pointed at
        // plugin/skills/ and make sure its MCP config is Codex-specific rather
        // than a copy of the Claude config that depends on CLAUDE_PROJECT_DIR.
        let codex_manifest: serde_json::Value = match std::fs::read_to_string(&codex_manifest_path)
        {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(value) => value,
                Err(e) => {
                    failures.push(format!(
                        "  {}: invalid JSON: {e}",
                        codex_manifest_path.display()
                    ));
                    serde_json::Value::Null
                }
            },
            Err(e) => {
                failures.push(format!(
                    "  {}: failed to read Codex plugin manifest: {e}",
                    codex_manifest_path.display()
                ));
                serde_json::Value::Null
            }
        };
        if !codex_manifest.is_null() {
            if codex_manifest
                .get("skills")
                .and_then(|value| value.as_str())
                != Some("./skills/")
            {
                failures.push(
                    "  plugin/.codex-plugin/plugin.json: `skills` must point at ./skills/ so Codex and Claude share the canonical skill package"
                        .to_string(),
                );
            }
            if codex_manifest.get("hooks").is_some() {
                failures.push(
                    "  plugin/.codex-plugin/plugin.json: must not declare Claude-only hooks"
                        .to_string(),
                );
            }
            let mcp_servers = codex_manifest.get("mcpServers");
            if !matches!(mcp_servers, Some(serde_json::Value::Object(map)) if !map.is_empty()) {
                failures.push(
                    "  plugin/.codex-plugin/plugin.json: `mcpServers` must be a non-empty object"
                        .to_string(),
                );
            }
            if mcp_servers
                .map(|value| value.to_string().contains("CLAUDE_PROJECT_DIR"))
                .unwrap_or(false)
            {
                failures.push(
                    "  plugin/.codex-plugin/plugin.json: Codex MCP config must not reference CLAUDE_PROJECT_DIR"
                        .to_string(),
                );
            }
        }

        let marketplace: serde_json::Value = match std::fs::read_to_string(&marketplace_path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(value) => value,
                Err(e) => {
                    failures.push(format!(
                        "  {}: invalid JSON: {e}",
                        marketplace_path.display()
                    ));
                    serde_json::Value::Null
                }
            },
            Err(e) => {
                failures.push(format!(
                    "  {}: failed to read Codex marketplace manifest: {e}",
                    marketplace_path.display()
                ));
                serde_json::Value::Null
            }
        };
        if let Some(plugins) = marketplace
            .get("plugins")
            .and_then(|value| value.as_array())
        {
            let source_path = plugins
                .iter()
                .find(|entry| entry.get("name").and_then(|value| value.as_str()) == Some("orbit"))
                .and_then(|entry| entry.get("source"))
                .and_then(|source| source.get("path"))
                .and_then(|value| value.as_str());
            if source_path != Some("./plugin") {
                failures.push(
                    "  .agents/plugins/marketplace.json: orbit entry must point at ./plugin"
                        .to_string(),
                );
            }
        } else if !marketplace.is_null() {
            failures
                .push("  .agents/plugins/marketplace.json: `plugins` must be an array".to_string());
        }

        assert!(
            failures.is_empty(),
            "plugin/skills/ package parity failed for {} skill(s):\n{}",
            failures.len(),
            failures.join("\n"),
        );
    }

    #[test]
    fn router_skill_enumerates_all_defaults() {
        let router_path = assets_skills_dir().join("orbit/SKILL.md");
        let contents = std::fs::read_to_string(&router_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", router_path.display()));

        let mut missing: Vec<&str> = Vec::new();
        for id in default_skill_ids() {
            if id == "orbit" {
                // The router skill itself is not enumerated within itself.
                continue;
            }
            let needle = format!("`{id}`");
            if !contents.contains(&needle) {
                missing.push(id);
            }
        }
        assert!(
            missing.is_empty(),
            "router skill at {} does not name these default skills as inline-code identifiers (expected occurrences of `<id>`): {missing:?}\nfix by adding a bullet to the ## Skill Selection block.",
            router_path.display(),
        );
    }
}
