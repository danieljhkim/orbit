use std::collections::HashSet;

use orbit_types::{OrbitError, Role, Skill};

use crate::OrbitRuntime;
use crate::skill_catalog::{LoadedSkill, SkillCatalogDoctorStatus};

pub struct SkillAddParams {
    pub name: String,
    pub description: Option<String>,
    pub instructions: String,
    pub context_files: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub role: Role,
}

#[derive(Default)]
pub struct SkillUpdateParams {
    pub description: Option<Option<String>>,
    pub instructions: Option<String>,
    pub context_files: Option<Vec<String>>,
    pub allowed_tools: Option<Vec<String>>,
    pub role: Option<Role>,
}

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

    pub(crate) fn resolve_work_skill_refs(
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

    pub fn add_skill(&self, _params: SkillAddParams) -> Result<Skill, OrbitError> {
        Err(OrbitError::InvalidInput(
            "legacy skill mutation is disabled; manage skills via .orbit/skills".to_string(),
        ))
    }

    pub fn list_skills(&self) -> Result<Vec<Skill>, OrbitError> {
        Err(OrbitError::InvalidInput(
            "legacy skill sqlite view is disabled; use `orbit skill list`".to_string(),
        ))
    }

    pub fn show_skill(&self, _name: &str) -> Result<Skill, OrbitError> {
        Err(OrbitError::InvalidInput(
            "legacy skill sqlite view is disabled; use `orbit skill show <id>`".to_string(),
        ))
    }

    pub fn update_skill(
        &self,
        _name: &str,
        _params: SkillUpdateParams,
    ) -> Result<Skill, OrbitError> {
        Err(OrbitError::InvalidInput(
            "legacy skill mutation is disabled; manage skills via .orbit/skills".to_string(),
        ))
    }

    pub fn delete_skill(&self, _name: &str) -> Result<(), OrbitError> {
        Err(OrbitError::InvalidInput(
            "legacy skill mutation is disabled; manage skills via .orbit/skills".to_string(),
        ))
    }

    pub fn attach_skill_to_task(
        &self,
        _task_id: &str,
        _skill_name: &str,
    ) -> Result<(), OrbitError> {
        Err(OrbitError::InvalidInput(
            "task-attached skill runtime is disabled; use work.skill_refs".to_string(),
        ))
    }

    pub fn detach_skill_from_task(
        &self,
        _task_id: &str,
        _skill_name: &str,
    ) -> Result<(), OrbitError> {
        Err(OrbitError::InvalidInput(
            "task-attached skill runtime is disabled; use work.skill_refs".to_string(),
        ))
    }

    pub fn list_task_skills(&self, _task_id: &str) -> Result<Vec<Skill>, OrbitError> {
        Err(OrbitError::InvalidInput(
            "task-attached skill runtime is disabled; use work.skill_refs".to_string(),
        ))
    }

    pub fn doctor_skills(&self) -> Result<Vec<SkillDoctorResult>, OrbitError> {
        Err(OrbitError::InvalidInput(
            "legacy skill sqlite doctor is disabled; use `orbit skill doctor`".to_string(),
        ))
    }
}
