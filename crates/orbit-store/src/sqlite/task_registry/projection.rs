use std::path::Path;

use orbit_common::types::OrbitError;
use orbit_common::utility::fs::create_dir_symlink;

use super::types::ProjectionRebuildResult;

pub(super) fn create_projection_symlink(
    target: &Path,
    link_path: &Path,
    result: &mut ProjectionRebuildResult,
) -> Result<(), OrbitError> {
    match create_dir_symlink(target, link_path) {
        Ok(()) => {
            result.projected += 1;
            Ok(())
        }
        Err(err) if is_symlink_degraded_error(&err) => {
            result.degraded_reason = Some(format!(
                "directory symlinks are unavailable for '{}': {err}",
                link_path.display()
            ));
            Ok(())
        }
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn is_symlink_degraded_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
    )
}
