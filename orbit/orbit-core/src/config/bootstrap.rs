use std::path::Path;

use orbit_types::OrbitError;

use crate::fs_utils::write_text_with_parent;

const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../../assets/config/default-config.toml");

pub(crate) fn seed_default_config(config_path: &Path) -> Result<bool, OrbitError> {
    if config_path.exists() {
        return Ok(false);
    }
    write_text_with_parent(config_path, DEFAULT_CONFIG_TEMPLATE)?;
    Ok(true)
}
