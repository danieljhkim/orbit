use clap::Args;
use orbit_graph::EXTRACTOR_VERSION;
use serde::Serialize;

use super::{CliError, json_value};

#[derive(Debug, Args)]
pub(crate) struct VersionCommand;

impl VersionCommand {
    pub(crate) fn run(&self) -> Result<serde_json::Value, CliError> {
        json_value(VersionOutput {
            crate_version: env!("CARGO_PKG_VERSION"),
            extractor_version: EXTRACTOR_VERSION,
        })
    }
}

#[derive(Debug, Serialize)]
struct VersionOutput {
    crate_version: &'static str,
    extractor_version: u32,
}
