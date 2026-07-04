use clap::Args;
use orbit_core::OrbitError;
use orbit_core::routines::{install_clock, resolve_host_id, write_host_id};
use orbit_core::workspace_registry;

#[derive(Args)]
#[command(
    after_help = "Writes ~/.orbit/host.toml (the identity `hosts:` pinning matches\n\
                        against) and, with --install-clock, installs the per-user OS clock\n\
                        unit that invokes `orbit sweep` every minute (launchd on macOS, a\n\
                        systemd user timer on Linux)."
)]
pub struct RoutineInitArgs {
    /// Host identity to write (defaults to the machine hostname).
    #[arg(long)]
    pub host_id: Option<String>,
    /// Also install and activate the OS clock unit driving `orbit sweep`.
    #[arg(long)]
    pub install_clock: bool,
}

impl RoutineInitArgs {
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        let global_root = workspace_registry::global_orbit_dir()?;

        let host_id = match self.host_id {
            Some(host_id) => host_id,
            None => resolve_host_id(&global_root)?,
        };
        let path = write_host_id(&global_root, &host_id)?;
        println!("host_id \"{host_id}\" written to {}", path.display());

        if !self.install_clock {
            println!("clock unit not installed (pass --install-clock to set up `orbit sweep`)");
            return Ok(());
        }

        let report = install_clock(&global_root)?;
        for file in &report.files_written {
            println!("wrote {}", file.display());
        }
        if report.activated {
            println!("clock unit active: `orbit sweep` runs every minute on this host");
        } else {
            println!("clock unit files written but not activated; run:");
            for step in &report.manual_steps {
                println!("  {step}");
            }
        }
        Ok(())
    }
}
