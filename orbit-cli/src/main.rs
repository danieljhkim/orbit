use clap::{Args, Parser, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Map, Value};

trait Execute {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError>;
}

#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "Orbit v2.1 CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Tool(ToolCommand),
    Task(TaskCommand),
    Audit(AuditCommand),
    Job(JobCommand),
    Watch(WatchCommand),
}

impl Execute for Commands {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::Watch(cmd) => cmd.execute(runtime),
        }
    }
}

#[derive(Args)]
struct ToolCommand {
    #[command(subcommand)]
    command: ToolSubcommand,
}

impl Execute for ToolCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
enum ToolSubcommand {
    Run(ToolRunArgs),
}

impl Execute for ToolSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ToolSubcommand::Run(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
struct ToolRunArgs {
    name: String,
    #[arg(long)]
    path: Option<String>,
    #[arg(long)]
    content: Option<String>,
    #[arg(long)]
    program: Option<String>,
    #[arg(long = "arg")]
    args: Vec<String>,
    #[arg(long)]
    timeout_ms: Option<u64>,
}

impl Execute for ToolRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let mut input = Map::new();
        if let Some(path) = self.path {
            input.insert("path".to_string(), Value::String(path));
        }
        if let Some(content) = self.content {
            input.insert("content".to_string(), Value::String(content));
        }
        if let Some(program) = self.program {
            input.insert("program".to_string(), Value::String(program));
        }
        if !self.args.is_empty() {
            input.insert(
                "args".to_string(),
                Value::Array(self.args.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(timeout_ms) = self.timeout_ms {
            input.insert("timeout_ms".to_string(), Value::Number(timeout_ms.into()));
        }

        let output = runtime.run_tool(&self.name, Value::Object(input))?;
        println!(
            "{}",
            serde_json::to_string_pretty(&output)
                .map_err(|e| OrbitError::Execution(e.to_string()))?
        );
        Ok(())
    }
}

#[derive(Args)]
struct TaskCommand {
    #[command(subcommand)]
    command: TaskSubcommand,
}

impl Execute for TaskCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
enum TaskSubcommand {
    Add(TaskAddArgs),
    List,
}

impl Execute for TaskSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            TaskSubcommand::Add(args) => args.execute(runtime),
            TaskSubcommand::List => {
                for task in runtime.list_tasks()? {
                    println!("{task}");
                }
                Ok(())
            }
        }
    }
}

#[derive(Args)]
struct TaskAddArgs {
    title: String,
}

impl Execute for TaskAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.add_task(&self.title)?;
        println!("{task}");
        Ok(())
    }
}

#[derive(Args)]
struct AuditCommand {
    #[command(subcommand)]
    command: AuditSubcommand,
}

impl Execute for AuditCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
enum AuditSubcommand {
    List(AuditListArgs),
}

impl Execute for AuditSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            AuditSubcommand::List(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
struct AuditListArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

impl Execute for AuditListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        for audit in runtime.list_audits(self.limit)? {
            println!("{audit}");
        }
        Ok(())
    }
}

#[derive(Args)]
struct JobCommand {
    #[command(subcommand)]
    command: JobSubcommand,
}

impl Execute for JobCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
enum JobSubcommand {
    Run,
}

impl Execute for JobSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            JobSubcommand::Run => {
                let count = runtime.run_jobs()?;
                println!("ran_jobs={count}");
                Ok(())
            }
        }
    }
}

#[derive(Args)]
struct WatchCommand {
    #[command(subcommand)]
    command: WatchSubcommand,
}

impl Execute for WatchCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
enum WatchSubcommand {
    Run(WatchRunArgs),
}

impl Execute for WatchSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            WatchSubcommand::Run(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
struct WatchRunArgs {
    #[arg(long, default_value = ".")]
    path: String,
}

impl Execute for WatchRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.trigger_watch_once(&self.path)?;
        println!("watch trigger recorded for {}", self.path);
        Ok(())
    }
}

fn main() {
    let cli = Cli::parse();
    let runtime = match OrbitRuntime::initialize() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("failed to initialize runtime: {err}");
            std::process::exit(1);
        }
    };

    if let Err(err) = cli.command.execute(&runtime) {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
