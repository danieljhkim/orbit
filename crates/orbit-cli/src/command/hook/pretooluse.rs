use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::hook::render::HookOutputFormat;
use crate::command::{CommandOut, CommandOutput, Execute};

#[derive(Args)]
pub struct PretooluseArgs {
    /// Render output in the hook format expected by this agent.
    #[arg(long, value_enum, default_value_t = HookOutputFormat::Claude)]
    pub format: HookOutputFormat,
}

impl Execute for PretooluseArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        if let Some(output) = orbit_cmd::learning_hook::run_pretooluse(runtime, self.format.into())
        {
            println!("{output}");
        }
        Ok(CommandOutput::Silent)
    }
}
