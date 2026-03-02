use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

impl Execute for ConfigCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ConfigSubcommand {
    Show(ConfigShowArgs),
}

impl Execute for ConfigSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ConfigSubcommand::Show(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct ConfigShowArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for ConfigShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let orbit_root = runtime.data_root();
        let orbit_home = runtime.orbit_home();
        let config_path = runtime.config_path();
        let (inherit, pass) = runtime.execution_env_config();
        let persistence = runtime.persistence_config_json();
        let task_approval_required_for_agent = runtime.task_approval_required_for_agent();
        let task_delegate_approval = runtime.task_delegate_approval();
        let identity_root = runtime.identity_root();
        let identity_role_overrides = runtime.identity_role_overrides();
        if self.json {
            crate::output::json::print_pretty(&json!({
                "root": orbit_root.to_string_lossy(),
                "home": orbit_home.to_string_lossy(),
                "path": config_path.to_string_lossy(),
                "config_path": config_path.to_string_lossy(),
                "exists": config_path.exists(),
                "execution": {
                    "env": {
                        "inherit": inherit,
                        "pass": pass,
                    }
                },
                "task": {
                    "approval": {
                        "required_for_agent": task_approval_required_for_agent,
                        "delegate_approval": task_delegate_approval
                    }
                },
                "identity": {
                    "root": identity_root.to_string_lossy(),
                    "roles": identity_role_overrides,
                },
                "persistence": persistence,
            }))
        } else {
            println!("ORBIT_ROOT (Root):   {}", orbit_root.to_string_lossy());
            println!("ORBIT_HOME (Home):   {}", orbit_home.to_string_lossy());
            println!("Config path:         {}", config_path.to_string_lossy());
            println!("Exists:              {}", config_path.exists());
            println!("Execution env inherit: {}", inherit);
            println!("Execution env pass:  {}", pass.join(","));
            println!(
                "Task approval required for agent: {}",
                task_approval_required_for_agent
            );
            println!("Task delegate approval: {}", task_delegate_approval);
            println!("Identity root:         {}", identity_root.to_string_lossy());
            println!("Persistence:         {}", persistence);
            Ok(())
        }
    }
}
