use clap::Args;
use orbit_core::command::init::InitOptions;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct InitCommand {
    /// Reset ~/.orbit to defaults before initialization
    #[arg(long)]
    pub force: bool,
}

impl Execute for InitCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.init_workspace_with_options(InitOptions { force: self.force })?;
        println!(
            "identities: root={}, created={}; skills: root={}, created={}, symlink_created={}; config: path={}, created={}; default_work_created={}",
            result.identity_root,
            result.created_identity_files,
            result.skills_root,
            result.created_skill_files,
            result.created_skills_symlink,
            result.config_path,
            result.created_config,
            result.created_default_work
        );
        Ok(())
    }
}
