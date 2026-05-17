use crate::types::OrbitError;

pub fn is_valid_friction_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 12
        && bytes[0] == b'F'
        && bytes[5] == b'-'
        && bytes[8] == b'-'
        && bytes[1..5].iter().all(u8::is_ascii_digit)
        && bytes[6..8].iter().all(u8::is_ascii_digit)
        && bytes[9..12].iter().all(u8::is_ascii_digit)
}

pub fn validate_friction_id(id: &str) -> Result<(), OrbitError> {
    if is_valid_friction_id(id) {
        Ok(())
    } else {
        Err(OrbitError::InvalidInput(format!(
            "friction id must match FYYYY-MM-NNN, got '{id}'"
        )))
    }
}

pub fn is_valid_learning_id(id: &str) -> bool {
    let Some(suffix) = id.strip_prefix('L') else {
        return false;
    };
    let Some((date, number)) = suffix.split_once('-') else {
        return false;
    };
    date.len() == 8
        && !number.is_empty()
        && date.as_bytes().iter().all(u8::is_ascii_digit)
        && number.as_bytes().iter().all(u8::is_ascii_digit)
}

pub fn is_valid_adr_id(id: &str) -> bool {
    let Some(suffix) = id.strip_prefix("ADR-") else {
        return false;
    };
    suffix.len() >= 4 && suffix.as_bytes().iter().all(u8::is_ascii_digit)
}
