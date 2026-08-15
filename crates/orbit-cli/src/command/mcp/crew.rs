//! Server-side projection of the selected workspace's local crew config.

use std::path::Path;

use orbit_common::types::{
    CREW_DISCOVERY_SCHEMA_VERSION, CrewDiscoveryV1, ExecutionProfileCrewV1, OrbitError,
};

pub(super) fn crew_discovery(
    global_root: &Path,
    checkout_orbit_dir: &Path,
    workspace_id: &str,
    owner_machine_id: Option<String>,
) -> Result<CrewDiscoveryV1, OrbitError> {
    let environment = orbit_core::local_crew_environment(global_root, checkout_orbit_dir)?;
    let crews = environment
        .crews
        .values()
        .map(|crew| ExecutionProfileCrewV1::from_crew(crew, environment.resolved_backend))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CrewDiscoveryV1 {
        schema_version: CREW_DISCOVERY_SCHEMA_VERSION,
        workspace_id: workspace_id.to_string(),
        owner_machine_id,
        default_crew: environment.default_crew,
        crews,
    })
}
