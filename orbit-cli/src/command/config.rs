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
        let config_path = OrbitRuntime::default_data_root().join("config.toml");
        let (inherit, pass) = runtime.execution_env_config();
        if self.json {
            crate::output::json::print_pretty(&json!({
                "path": config_path.to_string_lossy(),
                "exists": config_path.exists(),
                "execution": {
                    "env": {
                        "inherit": inherit,
                        "pass": pass,
                    }
                }
            }))
        } else {
            println!("Path:                {}", config_path.to_string_lossy());
            println!("Exists:              {}", config_path.exists());
            println!("Execution env inherit: {}", inherit);
            println!("Execution env pass:  {}", pass.join(","));
            Ok(())
        }
    }
}
