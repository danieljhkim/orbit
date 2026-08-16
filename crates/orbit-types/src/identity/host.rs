use super::IdentityError;

/// Maximum encoded length for a stable registry identifier.
pub const REGISTRY_IDENTIFIER_MAX_BYTES: usize = 128;
/// Namespace prefix for generated machine identifiers.
pub const MACHINE_ID_PREFIX: &str = "hm_";

/// Validate a path-free, normalized identifier stored in public registry
/// records. Transport targets and filesystem paths are never identities.
pub fn validate_registry_identifier(field: &str, value: &str) -> Result<(), IdentityError> {
    if value.is_empty() {
        return Err(IdentityError::Invalid(format!("{field} must not be empty")));
    }
    if value.trim() != value {
        return Err(IdentityError::Invalid(format!(
            "{field} must not contain leading or trailing whitespace"
        )));
    }
    if value.len() > REGISTRY_IDENTIFIER_MAX_BYTES {
        return Err(IdentityError::Invalid(format!(
            "{field} must not exceed {REGISTRY_IDENTIFIER_MAX_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) || value.contains(['/', '\\']) {
        return Err(IdentityError::Invalid(format!(
            "{field} must be a logical registry identifier, not a path"
        )));
    }
    Ok(())
}

/// Validate the stable machine key used in host and workspace role records.
pub fn validate_machine_id(machine_id: &str) -> Result<(), IdentityError> {
    validate_registry_identifier("machine_id", machine_id)?;
    let Some(suffix) = machine_id.strip_prefix(MACHINE_ID_PREFIX) else {
        return Err(IdentityError::Invalid(
            "machine_id must use the canonical 'hm_' namespace".to_string(),
        ));
    };
    if suffix.is_empty()
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(IdentityError::Invalid(
            "machine_id must contain 'hm_' followed by ASCII letters, digits, '_' or '-'"
                .to_string(),
        ));
    }
    Ok(())
}

/// Validate a human-readable name stored in local host identity and workspace records.
pub fn validate_host_id(host_id: &str) -> Result<(), IdentityError> {
    validate_registry_identifier("host_id", host_id)
}
