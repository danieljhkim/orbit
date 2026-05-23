use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;
use crate::command::hook::render::HookOutputFormat;

#[derive(Args)]
pub struct PretooluseArgs {
    /// Render output in the hook format expected by this agent.
    #[arg(long, value_enum, default_value_t = HookOutputFormat::Claude)]
    pub format: HookOutputFormat,
}

impl Execute for PretooluseArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if let Some(output) =
            orbit_core::command::learning_hook::run_pretooluse(runtime, self.format.into())
        {
            println!("{output}");
        }
        Ok(())
    }
}
