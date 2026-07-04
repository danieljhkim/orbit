use clap::Args;
use orbit_core::OrbitError;
use orbit_core::routines::resume_routine;
use orbit_core::workspace_registry;

#[derive(Args)]
pub struct RoutineResumeArgs {
    /// Routine name.
    pub name: String,
}

impl RoutineResumeArgs {
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        let global_root = workspace_registry::global_orbit_dir()?;
        if resume_routine(&global_root, &self.name)? {
            println!("resumed '{}' on this host", self.name);
        } else {
            println!("'{}' was not paused on this host", self.name);
        }
        Ok(())
    }
}
