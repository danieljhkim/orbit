mod audit_middleware;
mod command;
mod output;

use clap::Parser;
use orbit_core::OrbitRuntime;

use crate::command::{Commands, Execute};

fn main() {
    let cli = command::Cli::parse();
    let runtime = match OrbitRuntime::initialize() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("failed to initialize runtime: {err}");
            std::process::exit(1);
        }
    };

    let result = match cli.command {
        Commands::Audit(cmd) => cmd.execute(&runtime),
        other => {
            let meta = audit_middleware::extract_command_meta(&other);
            let mut guard = audit_middleware::AuditGuard::new(&runtime, meta);
            let result = other.execute(&runtime);
            match &result {
                Ok(()) => guard.mark_success(),
                Err(orbit_core::OrbitError::PolicyDenied(msg)) => guard.mark_denied(msg),
                Err(err) => guard.mark_failure(err),
            }
            result
        }
    };

    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
