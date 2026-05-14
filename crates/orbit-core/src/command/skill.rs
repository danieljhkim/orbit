use std::path::Path;

use std::collections::HashSet;

use orbit_common::types::OrbitError;

use orbit_common::utility::fs::write_text_with_parent;

use crate::OrbitRuntime;
use crate::skill_catalog::{LoadedSkill, SkillCatalogDoctorStatus};

const DEFAULT_SKILL_FILES: [(&str, &str); 9] = [
    ("orbit", include_str!("../../assets/skills/orbit/SKILL.md")),
    (
        "orbit-adr",
        include_str!("../../assets/skills/orbit-adr/SKILL.md"),
    ),
    (
        "orbit-create-task",
        include_str!("../../assets/skills/orbit-create-task/SKILL.md"),
    ),
    (
        "orbit-debug-job-failure",
        include_str!("../../assets/skills/orbit-debug-job-failure/SKILL.md"),
    ),
    (
        "orbit-design",
        include_str!("../../assets/skills/orbit-design/SKILL.md"),
    ),
    (
        "orbit-execute-task",
        include_str!("../../assets/skills/orbit-execute-task/SKILL.md"),
    ),
    (
        "orbit-graph",
        include_str!("../../assets/skills/orbit-graph/SKILL.md"),
    ),
    (
        "orbit-learning",
        include_str!("../../assets/skills/orbit-learning/SKILL.md"),
    ),
    (
        "orbit-track-issues",
        include_str!("../../assets/skills/orbit-track-issues/SKILL.md"),
    ),
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

pub(crate) fn default_skill_ids() -> [&'static str; 9] {
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
        count += 1;
    }
    Ok(count)
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
