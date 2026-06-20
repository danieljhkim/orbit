use orbit_common::types::OrbitError;

/// Registry operations use the shared Orbit error surface.
pub type RegistryError = OrbitError;

/// Result alias for registry APIs.
pub type RegistryResult<T> = std::result::Result<T, RegistryError>;
