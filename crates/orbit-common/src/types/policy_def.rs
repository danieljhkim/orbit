use std::collections::HashMap;
use std::path::{Component, Path};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::types::{OrbitError, validate_resource_name};

pub const DEFAULT_POLICY_NAME: &str = "default";
pub const UNRESTRICTED_FS_PROFILE: &str = "unrestricted";
const NO_MATCHING_RULE: &str = "<no matching rule>";
const EMPTY_RULESET: &str = "[]";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "denyRead", default, skip_serializing_if = "Vec::is_empty")]
    pub deny_read: Vec<String>,
    #[serde(rename = "denyModify", default, skip_serializing_if = "Vec::is_empty")]
    pub deny_modify: Vec<String>,
    #[serde(
        rename = "fsProfiles",
        default,
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub fs_profiles: HashMap<String, FsProfile>,
    #[serde(default = "chrono::Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "chrono::Utc::now")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct FsProfile {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub modify: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFsProfile {
    pub name: String,
    pub read: Vec<String>,
    pub modify: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FsOperation {
    Read,
    Modify,
}

impl FsOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Modify => "modify",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCheckResult {
    pub allowed: bool,
    pub matched_rule: String,
}

impl PolicyDef {
    pub fn validate(&self) -> Result<(), OrbitError> {
        validate_resource_name(&self.name)?;

        let deny_read = normalize_rule_set(&self.deny_read, "spec.denyRead")?;
        let deny_modify = normalize_rule_set(&self.deny_modify, "spec.denyModify")?;

        for (profile_name, profile) in &self.fs_profiles {
            if profile_name.trim().is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "policy `{}` has an empty fsProfile name",
                    self.name
                )));
            }

            let read = normalize_rule_set(
                &profile.read,
                &format!("spec.fsProfiles.{profile_name}.read"),
            )?;
            let modify = normalize_rule_set(
                &profile.modify,
                &format!("spec.fsProfiles.{profile_name}.modify"),
            )?;

            for rule in positive_rules(&read) {
                reject_explicit_global_deny(
                    &self.name,
                    profile_name,
                    "read",
                    rule,
                    &deny_read,
                    "denyRead",
                )?;
            }

            for rule in positive_rules(&modify) {
                reject_explicit_global_deny(
                    &self.name,
                    profile_name,
                    "modify",
                    rule,
                    &deny_modify,
                    "denyModify",
                )?;
                reject_explicit_global_deny(
                    &self.name,
                    profile_name,
                    "modify",
                    rule,
                    &deny_read,
                    "denyRead",
                )?;

                if !positive_rules(&read).any(|read_rule| rule_covers_path_rule(read_rule, rule)) {
                    return Err(OrbitError::InvalidInput(format!(
                        "policy `{}` fsProfile `{}` has modify rule `{}` that is not covered by any read rule",
                        self.name, profile_name, rule
                    )));
                }
            }
        }

        Ok(())
    }

    pub fn merged(global: &Self, workspace: &Self) -> Result<Self, OrbitError> {
        let mut fs_profiles = global.fs_profiles.clone();
        for (name, profile) in &workspace.fs_profiles {
            fs_profiles.insert(name.clone(), profile.clone());
        }

        let mut deny_read = global.deny_read.clone();
        extend_unique(&mut deny_read, &workspace.deny_read);

        let mut deny_modify = global.deny_modify.clone();
        extend_unique(&mut deny_modify, &workspace.deny_modify);

        let merged = Self {
            name: workspace.name.clone(),
            description: workspace
                .description
                .clone()
                .or_else(|| global.description.clone()),
            deny_read,
            deny_modify,
            fs_profiles,
            created_at: global.created_at,
            updated_at: if workspace.updated_at > global.updated_at {
                workspace.updated_at
            } else {
                global.updated_at
            },
        };
        merged.validate()?;
        Ok(merged)
    }

    pub fn effective_profile(&self, profile_name: &str) -> Result<ResolvedFsProfile, OrbitError> {
        let base = match self.fs_profiles.get(profile_name) {
            Some(profile) => profile.clone(),
            None if profile_name == UNRESTRICTED_FS_PROFILE => FsProfile {
                read: vec!["./**".to_string()],
                modify: vec!["./**".to_string()],
            },
            None => {
                return Err(OrbitError::InvalidInput(format!(
                    "policy `{}` does not define fsProfile `{profile_name}`",
                    self.name
                )));
            }
        };

        let mut read = normalize_rule_set(&base.read, &format!("fsProfile `{profile_name}` read"))?;
        let mut modify =
            normalize_rule_set(&base.modify, &format!("fsProfile `{profile_name}` modify"))?;
        let deny_read = normalize_rule_set(&self.deny_read, "spec.denyRead")?;
        let deny_modify = normalize_rule_set(&self.deny_modify, "spec.denyModify")?;

        read.extend(deny_read.into_iter().map(negate_rule));
        modify.extend(deny_modify.into_iter().map(negate_rule));

        Ok(ResolvedFsProfile {
            name: profile_name.to_string(),
            read,
            modify,
        })
    }

    pub fn check_path(
        &self,
        profile_name: &str,
        operation: FsOperation,
        path: &str,
    ) -> Result<FsCheckResult, OrbitError> {
        let profile = self.effective_profile(profile_name)?;
        let normalized_path = normalize_path(path)?;
        let rules = match operation {
            FsOperation::Read => &profile.read,
            FsOperation::Modify => &profile.modify,
        };

        if rules.is_empty() {
            return Ok(FsCheckResult {
                allowed: false,
                matched_rule: EMPTY_RULESET.to_string(),
            });
        }

        let mut saw_positive_rule = false;
        let mut decision = None;
        for rule in rules {
            let (negated, pattern) = split_rule(rule);
            if !negated {
                saw_positive_rule = true;
            }
            if rule_matches_path(pattern, &normalized_path)? {
                decision = Some(FsCheckResult {
                    allowed: !negated,
                    matched_rule: if negated {
                        pattern.to_string()
                    } else {
                        rule.clone()
                    },
                });
            }
        }

        Ok(decision.unwrap_or_else(|| FsCheckResult {
            allowed: false,
            matched_rule: if saw_positive_rule {
                NO_MATCHING_RULE.to_string()
            } else {
                EMPTY_RULESET.to_string()
            },
        }))
    }
}

fn extend_unique(target: &mut Vec<String>, extra: &[String]) {
    for value in extra {
        if !target.iter().any(|existing| existing == value) {
            target.push(value.clone());
        }
    }
}

fn reject_explicit_global_deny(
    policy_name: &str,
    profile_name: &str,
    section: &str,
    rule: &str,
    deny_rules: &[String],
    deny_label: &str,
) -> Result<(), OrbitError> {
    if deny_rules.iter().any(|deny_rule| deny_rule == rule) {
        return Err(OrbitError::InvalidInput(format!(
            "policy `{}` fsProfile `{}` {} rule `{}` duplicates global {} entry",
            policy_name, profile_name, section, rule, deny_label
        )));
    }
    Ok(())
}

fn rule_covers_path_rule(read_rule: &str, path_rule: &str) -> bool {
    if read_rule == path_rule || read_rule == "**" {
        return true;
    }

    if let Some(prefix) = read_rule.strip_suffix("/**") {
        if prefix.is_empty() {
            return true;
        }

        if path_rule == prefix || path_rule.starts_with(&format!("{prefix}/")) {
            return true;
        }

        if let Some(path_prefix) = path_rule.strip_suffix("/**") {
            return path_prefix == prefix || path_prefix.starts_with(&format!("{prefix}/"));
        }
    }

    false
}

fn positive_rules(rules: &[String]) -> impl Iterator<Item = &str> {
    rules
        .iter()
        .map(String::as_str)
        .filter(|rule| !rule.starts_with('!'))
}

fn normalize_rule_set(rules: &[String], label: &str) -> Result<Vec<String>, OrbitError> {
    rules
        .iter()
        .map(|rule| normalize_rule(rule, label))
        .collect()
}

fn normalize_rule(rule: &str, label: &str) -> Result<String, OrbitError> {
    let trimmed = rule.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "{label} contains an empty path rule"
        )));
    }

    let (negated, body) = split_rule(trimmed);
    let mut normalized = body.replace('\\', "/");
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_string();
    }
    if normalized.is_empty() {
        normalized = ".".to_string();
    }

    if normalized == "~"
        || normalized.starts_with("~/")
        || normalized.starts_with("../")
        || normalized == ".."
    {
        return Err(OrbitError::InvalidInput(format!(
            "{label} rule `{trimmed}` must stay inside the workspace root"
        )));
    }

    let path = Path::new(&normalized);
    if path.is_absolute() {
        return Err(OrbitError::InvalidInput(format!(
            "{label} rule `{trimmed}` must stay inside the workspace root"
        )));
    }

    for component in path.components() {
        match component {
            Component::CurDir | Component::Normal(_) => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(OrbitError::InvalidInput(format!(
                    "{label} rule `{trimmed}` must stay inside the workspace root"
                )));
            }
        }
    }

    compile_rule_regex(&normalized).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "{label} rule `{trimmed}` is not a valid filesystem glob: {error}"
        ))
    })?;

    Ok(if negated {
        negate_rule(normalized)
    } else {
        normalized
    })
}

fn normalize_path(path: &str) -> Result<String, OrbitError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "filesystem path must not be empty".to_string(),
        ));
    }

    let mut normalized = trimmed.replace('\\', "/");
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_string();
    }
    if normalized == "." {
        normalized.clear();
    }
    if normalized == "~"
        || normalized.starts_with("~/")
        || normalized.starts_with("../")
        || normalized == ".."
    {
        return Err(OrbitError::InvalidInput(format!(
            "filesystem path `{path}` must stay inside the workspace root"
        )));
    }

    let path_ref = Path::new(&normalized);
    if path_ref.is_absolute() {
        return Err(OrbitError::InvalidInput(format!(
            "filesystem path `{path}` must stay inside the workspace root"
        )));
    }

    for component in path_ref.components() {
        match component {
            Component::CurDir | Component::Normal(_) => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(OrbitError::InvalidInput(format!(
                    "filesystem path `{path}` must stay inside the workspace root"
                )));
            }
        }
    }

    Ok(normalized)
}

fn split_rule(rule: &str) -> (bool, &str) {
    rule.strip_prefix('!')
        .map(|rest| (true, rest))
        .unwrap_or((false, rule))
}

fn negate_rule(rule: String) -> String {
    format!("!{rule}")
}

fn rule_matches_path(rule: &str, path: &str) -> Result<bool, OrbitError> {
    let regex = compile_rule_regex(rule).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid filesystem glob `{rule}`: {error}"))
    })?;
    Ok(regex.is_match(path))
}

fn compile_rule_regex(rule: &str) -> Result<Regex, regex::Error> {
    if rule == "." {
        return Regex::new(r"^$");
    }

    if let Some(prefix) = rule.strip_suffix("/**") {
        if prefix.is_empty() {
            return Regex::new(r"^.*$");
        }
        let escaped = regex::escape(prefix);
        return Regex::new(&format!("^{escaped}(?:/.*)?$"));
    }

    let chars: Vec<char> = rule.chars().collect();
    let mut index = 0usize;
    let mut pattern = String::from("^");
    while index < chars.len() {
        if chars[index] == '*' {
            if index + 2 < chars.len() && chars[index + 1] == '*' && chars[index + 2] == '/' {
                pattern.push_str("(?:.*/)?");
                index += 3;
                continue;
            }
            if index + 1 < chars.len() && chars[index + 1] == '*' {
                pattern.push_str(".*");
                index += 2;
                continue;
            }
            pattern.push_str("[^/]*");
            index += 1;
            continue;
        }

        if chars[index] == '?' {
            pattern.push_str("[^/]");
            index += 1;
            continue;
        }

        pattern.push_str(&regex::escape(&chars[index].to_string()));
        index += 1;
    }
    pattern.push('$');
    Regex::new(&pattern)
}
