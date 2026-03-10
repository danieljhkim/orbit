use std::path::Path;

use std::collections::HashSet;

use orbit_types::OrbitError;

use crate::OrbitRuntime;
use crate::fs_utils::write_text_with_parent;
use crate::skill_catalog::{LoadedSkill, SkillCatalogDoctorStatus};

const DEFAULT_SKILL_FILES: [(&str, &str); 9] = [
    (
        "orbit-create-task",
        include_str!("../../assets/skills/orbit-create-task/SKILL.md"),
    ),
    (
        "orbit-approve-task",
        include_str!("../../assets/skills/orbit-approve-task/SKILL.md"),
    ),
    (
        "orbit-assess-codebase",
        include_str!("../../assets/skills/orbit-assess-codebase/SKILL.md"),
    ),
    (
        "orbit-execute-change-request",
        include_str!("../../assets/skills/orbit-execute-change-request/SKILL.md"),
    ),
    (
        "orbit-maintain-system",
        include_str!("../../assets/skills/orbit-maintain-system/SKILL.md"),
    ),
    (
        "orbit-operations-management",
        include_str!("../../assets/skills/orbit-operations-management/SKILL.md"),
    ),
    (
        "orbit-manage-tasks",
        include_str!("../../assets/skills/orbit-manage-tasks/SKILL.md"),
    ),
    (
        "orbit-skills",
        include_str!("../../assets/skills/orbit-skills/SKILL.md"),
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
) -> Result<usize, OrbitError> {
    let mut created = 0usize;
    for (id, content) in DEFAULT_SKILL_FILES {
        let path = skills_root.join(id).join("SKILL.md");
        if path.exists() {
            continue;
        }
        let rendered = inject_skill_template_tokens(content, orbit_root);
        write_text_with_parent(&path, &rendered)?;
        created += 1;
    }
    Ok(created)
}

fn inject_skill_template_tokens(raw: &str, orbit_root: &Path) -> String {
    let orbit_root_value = orbit_root.to_string_lossy();
    raw.replace(ORBIT_ROOT_TOKEN, orbit_root_value.as_ref())
}

impl OrbitRuntime {
    pub fn list_file_skills(&self) -> Result<Vec<LoadedSkill>, OrbitError> {
        self.context.skill_catalog.list()
    }

    pub fn show_file_skill(&self, name: &str) -> Result<LoadedSkill, OrbitError> {
        self.context.skill_catalog.load(name)
    }

    pub fn doctor_file_skills(&self) -> Result<Vec<SkillDoctorResult>, OrbitError> {
        let rows = self.context.skill_catalog.doctor()?;
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
            output.push(self.context.skill_catalog.load(skill_id)?);
        }
        Ok(output)
    }
}
