use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Show or update Orbit configuration")]
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
    /// Display current configuration values
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
        let global_root = runtime.global_root();
        let shared_root = runtime.shared_root();
        let local_root = runtime.local_root();
        let config_path = runtime.config_path();
        let (inherit, pass) = runtime.execution_env_config();
        let (codex_sandbox, codex_approval_policy) = runtime.codex_execution_config();
        let persistence = runtime.persistence_config_json();
        let task_approval_required_for_agent = runtime.task_approval_required_for_agent();
        let task_delegate_approval = runtime.task_delegate_approval();
        if self.json {
            crate::output::json::print_pretty(&json!({
                "global_root": global_root.to_string_lossy(),
                "shared_root": shared_root.to_string_lossy(),
                "local_root": local_root.to_string_lossy(),
                "workspace_root": shared_root.to_string_lossy(),
                "root": shared_root.to_string_lossy(),
                "selected_root": shared_root.to_string_lossy(),
                "path": config_path.to_string_lossy(),
                "config_path": config_path.to_string_lossy(),
                "exists": config_path.exists(),
                "execution": {
                    "env": {
                        "inherit": inherit,
                        "pass": pass,
                    },
                    "codex": {
                        "sandbox": codex_sandbox,
                        "approval_policy": codex_approval_policy,
                    }
                },
                "task": {
                    "approval": {
                        "required_for_agent": task_approval_required_for_agent,
                        "delegate_approval": task_delegate_approval
                    }
                },
                "persistence": persistence,
            }))
        } else {
            println!("Global root:         {}", global_root.to_string_lossy());
            println!("Shared root:         {}", shared_root.to_string_lossy());
            println!("Local root:          {}", local_root.to_string_lossy());
            println!("Config path:         {}", config_path.to_string_lossy());
            println!("Exists:              {}", config_path.exists());
            println!("Execution env inherit: {}", inherit);
            println!("Execution env pass:  {}", pass.join(","));
            println!("Codex sandbox:       {}", codex_sandbox);
            println!(
                "Codex approval policy: {}",
                codex_approval_policy.unwrap_or_else(|| "provider-default".to_string())
            );
            println!(
                "Task approval required for agent: {}",
                task_approval_required_for_agent
            );
            println!("Task delegate approval: {}", task_delegate_approval);
            println!("Persistence:         {}", persistence);
            Ok(())
        }
    }
}
