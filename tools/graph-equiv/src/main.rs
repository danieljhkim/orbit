//! Binary scaffold for the orbit-graph equivalence harness.

mod backend;

use std::env;
use std::error::Error;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use backend::{Backend, V1Backend, V2Backend};

fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "{error}");
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("smoke") => run_smoke(SmokeOptions::parse(args)?),
        Some("--help") | Some("-h") => Ok(()),
        Some(other) => Err(input_error(format!("unknown command `{other}`"))),
    }
}

fn run_smoke(options: SmokeOptions) -> Result<(), Box<dyn Error>> {
    match options.backend.as_str() {
        "v1" => smoke_backend(
            &V1Backend::for_workspace(options.workspace, options.knowledge_dir),
            &options.query,
        ),
        "v2" => smoke_backend(&V2Backend, &options.query),
        other => Err(input_error(format!(
            "`--backend` must be `v1` or `v2`, got `{other}`"
        ))),
    }
}

fn smoke_backend(backend: &dyn Backend, query: &str) -> Result<(), Box<dyn Error>> {
    let search = backend.search(query)?;
    let Some(first_hit) = search.first() else {
        return Err(input_error(format!(
            "v1 smoke search returned no results for `{query}`"
        )));
    };
    let _show = backend.show(&first_hit.selector)?;
    let _ = backend.refs(&first_hit.selector);
    let _ = backend.callees(&first_hit.selector);
    let _ = backend.impact(&first_hit.selector, 3);
    Ok(())
}

struct SmokeOptions {
    backend: String,
    workspace: PathBuf,
    knowledge_dir: Option<PathBuf>,
    query: String,
}

impl SmokeOptions {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut backend = "v1".to_string();
        let mut workspace = env::current_dir()?;
        let mut knowledge_dir = None;
        let mut query = "GraphCommandContext".to_string();
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--backend" => backend = required_value(&mut args, "--backend")?,
                "--workspace" => {
                    workspace = PathBuf::from(required_value(&mut args, "--workspace")?)
                }
                "--knowledge-dir" => {
                    knowledge_dir =
                        Some(PathBuf::from(required_value(&mut args, "--knowledge-dir")?));
                }
                "--query" => query = required_value(&mut args, "--query")?,
                "--help" | "-h" => {}
                other => return Err(input_error(format!("unknown smoke option `{other}`"))),
            }
        }

        Ok(Self {
            backend,
            workspace,
            knowledge_dir,
            query,
        })
    }
}

fn required_value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| input_error(format!("missing value for `{flag}`")))
}

fn input_error(message: String) -> Box<dyn Error> {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into()
}
