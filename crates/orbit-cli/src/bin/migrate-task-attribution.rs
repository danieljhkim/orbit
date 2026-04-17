use std::path::PathBuf;

use orbit_core::{OrbitError, OrbitRuntime};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), OrbitError> {
    let mut args = std::env::args().skip(1);
    let mut root_override: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                let Some(value) = args.next() else {
                    return Err(OrbitError::InvalidInput(
                        "expected a path after --root".to_string(),
                    ));
                };
                root_override = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => {
                return Err(OrbitError::InvalidInput(format!(
                    "unknown argument: {other}"
                )));
            }
        }
    }

    let runtime = OrbitRuntime::initialize_with_root_override(root_override.as_deref())?;
    let tasks = runtime.list_tasks()?;
    let mut migrated = 0usize;
    for task in tasks {
        runtime.migrate_task_attribution_fields(&task.id)?;
        migrated += 1;
    }

    println!("migrated {migrated} tasks");
    Ok(())
}

fn print_help() {
    println!("Usage: migrate-task-attribution [--root <path>]");
    println!("Rewrites Orbit task artifacts to the current attribution schema.");
}
