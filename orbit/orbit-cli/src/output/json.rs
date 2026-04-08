use orbit_core::OrbitError;
use serde_json::Value;

pub fn print_pretty(value: &Value) -> Result<(), OrbitError> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|e| OrbitError::Execution(e.to_string()))?
    );
    Ok(())
}
