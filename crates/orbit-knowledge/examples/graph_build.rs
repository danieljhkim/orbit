use std::path::PathBuf;

use orbit_knowledge::graph_bench::{
    BenchScenario, GraphBenchOptions, run_benchmark_with_child_process, run_single_scenario,
    write_child_metrics,
};

struct Args {
    workspace: PathBuf,
    knowledge_dir: Option<PathBuf>,
    scoreboard_path: Option<PathBuf>,
    child_scenario: Option<BenchScenario>,
    child_output: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let mut options = GraphBenchOptions::from_workspace(args.workspace);
    if let Some(knowledge_dir) = args.knowledge_dir {
        options.knowledge_dir = knowledge_dir;
    }
    if let Some(scoreboard_path) = args.scoreboard_path {
        options.scoreboard_path = scoreboard_path;
    }

    if let Some(scenario) = args.child_scenario {
        let output_path = args
            .child_output
            .ok_or("--child-output is required with --child-scenario")?;
        let metrics = run_single_scenario(&options, scenario)?;
        write_child_metrics(&output_path, &metrics)?;
        return Ok(());
    }

    let current_exe = std::env::current_exe()?;
    let outcome = run_benchmark_with_child_process(&options, &current_exe)?;
    println!("{}", outcome.summary);
    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut workspace = std::env::current_dir()?;
    let mut knowledge_dir = None;
    let mut scoreboard_path = None;
    let mut child_scenario = None;
    let mut child_output = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => workspace = next_path(&mut args, "--workspace")?,
            "--knowledge-dir" => knowledge_dir = Some(next_path(&mut args, "--knowledge-dir")?),
            "--scoreboard" => scoreboard_path = Some(next_path(&mut args, "--scoreboard")?),
            "--child-scenario" => {
                let value = next_value(&mut args, "--child-scenario")?;
                child_scenario = Some(
                    BenchScenario::parse(&value)
                        .ok_or("expected cold_build or warm_incremental_noop")?,
                );
            }
            "--child-output" => child_output = Some(next_path(&mut args, "--child-output")?),
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }

    Ok(Args {
        workspace,
        knowledge_dir,
        scoreboard_path,
        child_scenario,
        child_output,
    })
}

fn next_path(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(PathBuf::from(next_value(args, flag)?))
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value").into())
}

fn print_help() {
    println!("Usage: graph_build [--workspace PATH] [--knowledge-dir PATH] [--scoreboard PATH]");
    println!();
    println!(
        "Runs cold_build and warm_incremental_noop graph build scenarios through pipeline::run_build."
    );
}
