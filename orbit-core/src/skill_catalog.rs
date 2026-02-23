use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use orbit_types::OrbitError;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::json_schema::validate_schema_document;

const REQUIRED_SECTIONS: [&str; 3] = ["Purpose", "Behavioral Constraints", "Output Requirements"];
const OPTIONAL_SECTIONS: [&str; 3] = ["Evaluation Focus", "Prohibitions", "Examples"];
const META_NAME: &str = "name";
const META_SUMMARY: &str = "summary";
const META_TAGS: &str = "tags";
const META_VERSION: &str = "version";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillSections {
    pub purpose: String,
    pub behavioral_constraints: String,
    pub output_requirements: String,
    pub evaluation_focus: Option<String>,
    pub prohibitions: Option<String>,
    pub examples: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillMeta {
    pub name: Option<String>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LoadedSkill {
    pub id: String,
    pub path: PathBuf,
    pub content_hash: String,
    pub content: String,
    pub sections: SkillSections,
    pub meta: Option<SkillMeta>,
    pub meta_raw: Option<Value>,
    pub output_schema: Option<Value>,
}

#[derive(Debug)]
struct ParsedMetaJson {
    meta: Option<SkillMeta>,
    meta_raw: Option<Value>,
    output_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum SkillCatalogDoctorStatus {
    Ok,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillCatalogDoctorRow {
    pub skill_id: String,
    pub status: SkillCatalogDoctorStatus,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct SkillCatalog {
    root: PathBuf,
}

impl SkillCatalog {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure_layout(&self) -> Result<(), OrbitError> {
        fs::create_dir_all(&self.root).map_err(|e| OrbitError::Io(e.to_string()))
    }

    pub fn list(&self) -> Result<Vec<LoadedSkill>, OrbitError> {
        let mut ids = self.list_candidate_ids()?;
        ids.sort();
        let mut skills = Vec::new();
        for id in ids {
            if let Ok(skill) = self.load(&id) {
                skills.push(skill);
            }
        }
        Ok(skills)
    }

    pub fn doctor(&self) -> Result<Vec<SkillCatalogDoctorRow>, OrbitError> {
        let mut ids = self.list_candidate_ids()?;
        ids.sort();
        let mut rows = Vec::new();
        for id in ids {
            match self.load(&id) {
                Ok(_) => rows.push(SkillCatalogDoctorRow {
                    skill_id: id,
                    status: SkillCatalogDoctorStatus::Ok,
                    message: String::new(),
                }),
                Err(err) => rows.push(SkillCatalogDoctorRow {
                    skill_id: id,
                    status: SkillCatalogDoctorStatus::Error,
                    message: err.to_string(),
                }),
            }
        }
        Ok(rows)
    }

    pub fn load(&self, skill_id: &str) -> Result<LoadedSkill, OrbitError> {
        if skill_id.trim().is_empty() {
            return Err(OrbitError::SkillValidation(
                "skill id must not be empty".to_string(),
            ));
        }
        let dir = self.root.join(skill_id);
        if !dir.exists() {
            return Err(OrbitError::SkillNotFound(skill_id.to_string()));
        }
        if !dir.is_dir() {
            return Err(OrbitError::SkillValidation(format!(
                "skill path is not a directory: {}",
                dir.display()
            )));
        }

        let skill_md_path = dir.join("SKILL.md");
        if !skill_md_path.exists() {
            return Err(OrbitError::SkillValidation(format!(
                "missing SKILL.md for skill '{}'",
                skill_id
            )));
        }
        let content =
            fs::read_to_string(&skill_md_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        validate_skill_content_safety(&content)?;
        let (declared_id, sections) = parse_skill_markdown(&content)?;
        if declared_id != skill_id {
            return Err(OrbitError::SkillValidation(format!(
                "skill heading '{}' must match directory '{}'",
                declared_id, skill_id
            )));
        }

        let content_hash = sha256_hex(content.as_bytes());
        let meta_path = dir.join("meta.json");
        let parsed_meta = if meta_path.exists() {
            parse_meta_json(&meta_path)?
        } else {
            ParsedMetaJson {
                meta: None,
                meta_raw: None,
                output_schema: None,
            }
        };

        Ok(LoadedSkill {
            id: skill_id.to_string(),
            path: dir,
            content_hash,
            content,
            sections,
            meta: parsed_meta.meta,
            meta_raw: parsed_meta.meta_raw,
            output_schema: parsed_meta.output_schema,
        })
    }

    fn list_candidate_ids(&self) -> Result<Vec<String>, OrbitError> {
        self.ensure_layout()?;
        let entries = fs::read_dir(&self.root).map_err(|e| OrbitError::Io(e.to_string()))?;
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            if path.join("SKILL.md").exists() {
                ids.push(name.to_string());
            }
        }
        Ok(ids)
    }
}

fn validate_skill_content_safety(content: &str) -> Result<(), OrbitError> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            return Err(OrbitError::SkillValidation(
                "code blocks are not allowed in SKILL.md".to_string(),
            ));
        }
        if trimmed.starts_with("$ ") {
            return Err(OrbitError::SkillValidation(
                "shell commands are not allowed in SKILL.md".to_string(),
            ));
        }
    }
    Ok(())
}

fn parse_skill_markdown(raw: &str) -> Result<(String, SkillSections), OrbitError> {
    let mut lines = raw.lines();
    let mut heading: Option<String> = None;

    for line in lines.by_ref() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let value = rest.trim().to_string();
            if value.is_empty() {
                return Err(OrbitError::SkillValidation(
                    "skill heading must not be empty".to_string(),
                ));
            }
            heading = Some(value);
            break;
        }
        return Err(OrbitError::SkillValidation(
            "first non-empty line must be '# <skill-id>'".to_string(),
        ));
    }

    let heading = heading.ok_or_else(|| {
        OrbitError::SkillValidation("SKILL.md must contain '# <skill-id>' heading".to_string())
    })?;

    let mut allowed = BTreeSet::new();
    for required in REQUIRED_SECTIONS {
        let _ = allowed.insert(required);
    }
    for optional in OPTIONAL_SECTIONS {
        let _ = allowed.insert(optional);
    }

    let mut current_section: Option<String> = None;
    let mut section_map: BTreeMap<String, String> = BTreeMap::new();

    for line in lines {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.trim().strip_prefix("## ") {
            let section_name = rest.trim().to_string();
            if !allowed.contains(section_name.as_str()) {
                return Err(OrbitError::SkillValidation(format!(
                    "unknown section header '{}'",
                    section_name
                )));
            }
            if section_map.contains_key(&section_name) {
                return Err(OrbitError::SkillValidation(format!(
                    "duplicate section header '{}'",
                    section_name
                )));
            }
            let _ = section_map.insert(section_name.clone(), String::new());
            current_section = Some(section_name);
            continue;
        }

        let Some(section_name) = current_section.clone() else {
            if trimmed.trim().is_empty() {
                continue;
            }
            return Err(OrbitError::SkillValidation(
                "content outside named sections is not allowed".to_string(),
            ));
        };

        let entry = section_map
            .get_mut(&section_name)
            .expect("section must exist before content append");
        entry.push_str(trimmed);
        entry.push('\n');
    }

    for required in REQUIRED_SECTIONS {
        let Some(value) = section_map.get(required) else {
            return Err(OrbitError::SkillValidation(format!(
                "missing required section '{}'",
                required
            )));
        };
        if value.trim().is_empty() {
            return Err(OrbitError::SkillValidation(format!(
                "section '{}' must not be empty",
                required
            )));
        }
    }

    Ok((
        heading,
        SkillSections {
            purpose: section_map
                .get("Purpose")
                .map(|v| v.trim().to_string())
                .unwrap_or_default(),
            behavioral_constraints: section_map
                .get("Behavioral Constraints")
                .map(|v| v.trim().to_string())
                .unwrap_or_default(),
            output_requirements: section_map
                .get("Output Requirements")
                .map(|v| v.trim().to_string())
                .unwrap_or_default(),
            evaluation_focus: section_map
                .get("Evaluation Focus")
                .map(|v| v.trim().to_string()),
            prohibitions: section_map
                .get("Prohibitions")
                .map(|v| v.trim().to_string()),
            examples: section_map.get("Examples").map(|v| v.trim().to_string()),
        },
    ))
}

fn parse_meta_json(path: &Path) -> Result<ParsedMetaJson, OrbitError> {
    let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    let value: Value = serde_json::from_str(&raw).map_err(|e| {
        OrbitError::SkillValidation(format!("invalid meta.json at '{}': {e}", path.display()))
    })?;
    let obj = value.as_object().ok_or_else(|| {
        OrbitError::SkillValidation(format!(
            "meta.json at '{}' must be a JSON object",
            path.display()
        ))
    })?;

    let mut schema_obj = obj.clone();
    let name = parse_optional_string(&mut schema_obj, META_NAME)?;
    let summary = parse_optional_string(&mut schema_obj, META_SUMMARY)?;
    let tags = parse_optional_tags(&mut schema_obj)?;
    let version = parse_optional_semver(&mut schema_obj, META_VERSION)?;
    let meta = if name.is_some() || summary.is_some() || !tags.is_empty() || version.is_some() {
        Some(SkillMeta {
            name,
            summary,
            tags,
            version,
        })
    } else {
        None
    };

    let output_schema = Value::Object(schema_obj);
    let schema_context = format!("meta.json at '{}'", path.display());
    let _ = validate_schema_document(&output_schema, &schema_context)?;

    Ok(ParsedMetaJson {
        meta,
        meta_raw: Some(value),
        output_schema: Some(output_schema),
    })
}

fn parse_optional_string(
    obj: &mut Map<String, Value>,
    key: &str,
) -> Result<Option<String>, OrbitError> {
    let Some(value) = obj.remove(key) else {
        return Ok(None);
    };
    let string = value.as_str().ok_or_else(|| {
        OrbitError::SkillValidation(format!("meta.json field '{}' must be a string", key))
    })?;
    Ok(Some(string.to_string()))
}

fn parse_optional_tags(obj: &mut Map<String, Value>) -> Result<Vec<String>, OrbitError> {
    let Some(value) = obj.remove(META_TAGS) else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        OrbitError::SkillValidation("meta.json field 'tags' must be an array".to_string())
    })?;
    let mut tags = Vec::new();
    for tag in values {
        let item = tag.as_str().ok_or_else(|| {
            OrbitError::SkillValidation("meta.json field 'tags' must contain strings".to_string())
        })?;
        tags.push(item.to_string());
    }
    Ok(tags)
}

fn parse_optional_semver(
    obj: &mut Map<String, Value>,
    key: &str,
) -> Result<Option<String>, OrbitError> {
    let Some(value) = obj.remove(key) else {
        return Ok(None);
    };
    let version = value.as_str().ok_or_else(|| {
        OrbitError::SkillValidation(format!("meta.json field '{}' must be a string", key))
    })?;
    if !is_semver(version) {
        return Err(OrbitError::SkillValidation(format!(
            "meta.json field '{}' must be semantic version MAJOR.MINOR.PATCH",
            key
        )));
    }
    Ok(Some(version.to_string()))
}

fn is_semver(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}
