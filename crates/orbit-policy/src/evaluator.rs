use orbit_common::OrbitError;
use orbit_types::policy::{FsCheckResult, FsOperation, PolicyDef};

pub(crate) fn evaluate(
    def: &PolicyDef,
    profile: &str,
    operation: FsOperation,
    path: &str,
) -> Result<FsCheckResult, OrbitError> {
    def.check_path(profile, operation, path).map_err(Into::into)
}
