use serde::Deserialize;

use super::types::ArtifactRef;

pub(super) fn parse_artifact_ref(raw: &str) -> Result<ArtifactRef, String> {
    let trimmed = raw.trim();
    if is_task_ref(trimmed) {
        return Ok(ArtifactRef::Task(trimmed.to_string()));
    }
    if is_learning_ref(trimmed) {
        return Ok(ArtifactRef::Learning(trimmed.to_string()));
    }
    if is_friction_ref(trimmed) {
        return Ok(ArtifactRef::Friction(trimmed.to_string()));
    }
    if is_adr_ref(trimmed) {
        return Ok(ArtifactRef::Adr(trimmed.to_string()));
    }
    Err(format!(
        "unknown related_artifacts reference `{trimmed}`; expected ORB-NNNNN, L-NNNN, FYYYY-MM-NNN, or ADR-NNNN"
    ))
}

pub(super) fn is_task_ref(value: &str) -> bool {
    value.len() == 9
        && value.starts_with("ORB-")
        && value[4..].chars().all(|ch| ch.is_ascii_digit())
}

pub(super) fn is_learning_ref(value: &str) -> bool {
    let Some(ordinal) = value.strip_prefix("L-") else {
        return false;
    };
    ordinal.len() >= 4 && ordinal.chars().all(|ch| ch.is_ascii_digit())
}

pub(super) fn is_friction_ref(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('F') else {
        return false;
    };
    let parts = rest.split('-').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 3
        && parts
            .iter()
            .all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
}

pub(super) fn is_adr_ref(value: &str) -> bool {
    value.len() == 8
        && value.starts_with("ADR-")
        && value[4..].chars().all(|ch| ch.is_ascii_digit())
}

// Provide Deserialize for ArtifactRef (type lives in types.rs to group with Doc* types)
impl<'de> Deserialize<'de> for ArtifactRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_artifact_ref(&raw).map_err(serde::de::Error::custom)
    }
}
